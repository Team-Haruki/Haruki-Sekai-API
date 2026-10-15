//! Registry service: ties the per-region syncers (pull from owner, ingest,
//! git push) to the on-disk state (manifest, history, app identity) and the
//! subscriber fan-out.

use std::collections::HashMap;
use std::sync::Arc;

use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};
use tracing::{error, info, warn};

use super::blobs::{BlobStore, FsBlobStore, StoreMode, StoreStats};
use super::metas::MusicMetasManager;
use super::state::RegistryState;
use crate::api::internal::{build_master_manifest, MasterManifest};
use crate::client::helper::AppInfo;
use crate::config::{Config, ServerRegion};
use crate::error::AppError;
use crate::updater::sync::MasterSyncer;

/// The subscriber notice for a publish. `server` and `dataVersion` are what
/// every receiver has always read; `contentHash`, `gitCommit` and the file
/// diff against the previous `current` (`changedFiles`: new or modified,
/// `removedFiles`) are additions older receivers ignore. The notice is a
/// trigger only: a receiver must re-read `current` rather than trust it.
pub fn publish_notice(
    manifest: &MasterManifest,
    previous: Option<&MasterManifest>,
) -> serde_json::Value {
    let before: HashMap<&str, &str> = previous
        .map(|p| {
            p.files
                .iter()
                .map(|f| (f.name.as_str(), f.sha256.as_str()))
                .collect()
        })
        .unwrap_or_default();
    let changed: Vec<&str> = manifest
        .files
        .iter()
        .filter(|f| before.get(f.name.as_str()) != Some(&f.sha256.as_str()))
        .map(|f| f.name.as_str())
        .collect();
    let now: std::collections::HashSet<&str> =
        manifest.files.iter().map(|f| f.name.as_str()).collect();
    let mut removed: Vec<&str> = before
        .keys()
        .copied()
        .filter(|n| !now.contains(n))
        .collect();
    removed.sort_unstable();
    serde_json::json!({
        "server": manifest.server,
        "dataVersion": manifest.data_version,
        "contentHash": super::state::content_hash(manifest),
        "gitCommit": manifest.git_commit,
        "changedFiles": changed,
        "removedFiles": removed,
    })
}

/// Outcome of pushing an app identity to one account node.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AppIdentityPush {
    pub url: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub struct Registry {
    pub config: Arc<Config>,
    pub state: RegistryState,
    /// Where master file content is served from (`registry.blob_store`).
    pub blobs: Arc<dyn BlobStore>,
    pub syncers: HashMap<ServerRegion, Arc<MasterSyncer>>,
    /// The music_metas feed, when `registry.music_metas.enabled`.
    pub metas: Option<MusicMetasManager>,
    http: reqwest::Client,
    /// One publish at a time per region so a webhook and a poll cannot
    /// interleave their manifest/history writes.
    publish_locks: HashMap<ServerRegion, tokio::sync::Mutex<()>>,
    /// Serializes app-identity writes and their delivery per region so two
    /// concurrent PUTs cannot persist one identity and deliver the other.
    app_locks: HashMap<ServerRegion, tokio::sync::Mutex<()>>,
    /// The synced appVersion an older override was last reported against,
    /// so the warning is logged once per owner version, not on every poll.
    stale_override_warned: parking_lot::Mutex<HashMap<ServerRegion, String>>,
}

impl Registry {
    /// A registry on file-backed state under `registry.state_dir`.
    pub fn new(config: Arc<Config>, syncers: HashMap<ServerRegion, Arc<MasterSyncer>>) -> Self {
        let state = RegistryState::new(&config.registry.state_dir);
        Self::with_state(config, syncers, state)
    }

    /// A registry on the given state (file or database backed).
    pub fn with_state(
        config: Arc<Config>,
        syncers: HashMap<ServerRegion, Arc<MasterSyncer>>,
        state: RegistryState,
    ) -> Self {
        let metas = if config.registry.music_metas.enabled {
            match MusicMetasManager::new(&config, state.clone()) {
                Ok(m) => Some(m),
                Err(e) => {
                    error!("music_metas feed disabled: {}", e);
                    None
                }
            }
        } else {
            None
        };
        let publish_locks = config
            .servers
            .keys()
            .map(|region| (*region, tokio::sync::Mutex::new(())))
            .collect();
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();
        Self {
            config,
            state,
            blobs: Arc::new(FsBlobStore),
            syncers,
            metas,
            http,
            publish_locks,
            app_locks: [
                ServerRegion::Jp,
                ServerRegion::En,
                ServerRegion::Tw,
                ServerRegion::Kr,
                ServerRegion::Cn,
            ]
            .into_iter()
            .map(|region| (region, tokio::sync::Mutex::new(())))
            .collect(),
            stale_override_warned: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    /// Serve master file content from `blobs` instead of the master
    /// directories (see [`super::blobs::open_blob_store`]).
    pub fn with_blob_store(mut self, blobs: Arc<dyn BlobStore>) -> Self {
        self.blobs = blobs;
        self
    }

    /// Store a (possibly partial) app-identity override and deliver the
    /// resulting complete identity to every account node, serialized per
    /// region. Omitted fields are filled from the current effective identity
    /// (previous override, else the owner's synced version file); a field
    /// that cannot be filled from anywhere is rejected. appVersion goes
    /// through the region's `pin_app_version_patch_to_zero`, so the stored
    /// override is what the account nodes store.
    pub async fn set_app_identity(
        &self,
        region: ServerRegion,
        requested: &AppInfo,
    ) -> Result<(AppInfo, Vec<AppIdentityPush>), AppError> {
        let _guard = match self.app_locks.get(&region) {
            Some(lock) => lock.lock().await,
            None => return Err(AppError::InvalidServerRegion(region.as_str().to_string())),
        };
        let current = self.app_identity(region).await.ok();
        let pick =
            |given: &str, field: &str, fallback: Option<&String>| -> Result<String, AppError> {
                if !given.trim().is_empty() {
                    return Ok(given.trim().to_string());
                }
                fallback
                    .filter(|v| !v.trim().is_empty())
                    .cloned()
                    .ok_or_else(|| {
                        AppError::ParseError(format!(
                            "{field} is required: no current value to keep for region {}",
                            region.as_str()
                        ))
                    })
            };
        let effective = AppInfo {
            app_version: self.effective_app_version(
                region,
                &pick(
                    &requested.app_version,
                    "appVersion",
                    current.as_ref().map(|c| &c.app_version),
                )?,
            ),
            app_hash: pick(
                &requested.app_hash,
                "appHash",
                current.as_ref().map(|c| &c.app_hash),
            )?,
        };
        self.state.set_app_identity(region, &effective).await?;
        info!(
            "{} App identity override set: appVersion={} appHash={}",
            region.as_str().to_uppercase(),
            effective.app_version,
            effective.app_hash.chars().take(16).collect::<String>()
        );
        // The override is authoritative here too: the registry's own version
        // file (and with it the git mirror) takes it as is, a downgrade
        // included, which the forward-only owner adoption would not.
        if let Some(syncer) = self.syncers.get(&region) {
            let app = crate::updater::master::AppIdentity {
                app_version: effective.app_version.clone(),
                app_hash: effective.app_hash.clone(),
            };
            if let Err(e) = syncer.store_app_identity(&app).await {
                warn!(
                    "{} App identity override not written to the version file: {}",
                    region.as_str().to_uppercase(),
                    e
                );
            }
        }
        let pushed = self.push_app_identity(region, &effective).await;
        self.republish_app_identity_logged(region).await;
        Ok((effective, pushed))
    }

    /// Remove the region's app-identity override; `current` and `GET /v1/app`
    /// fall back to the synced version file. Returns whether one was stored.
    pub async fn clear_app_identity(&self, region: ServerRegion) -> Result<bool, AppError> {
        let removed = {
            let _guard = match self.app_locks.get(&region) {
                Some(lock) => lock.lock().await,
                None => return Err(AppError::InvalidServerRegion(region.as_str().to_string())),
            };
            self.state.clear_app_identity(region).await?
        };
        if removed {
            self.republish_app_identity_logged(region).await;
        }
        Ok(removed)
    }

    /// Re-record `current` with the region's effective app identity when it
    /// changed (an override set or cleared, the synced version file following
    /// an app-identity push on the owner), without waiting for master data to
    /// change: consumers poll `current` for the appVersion. The file set,
    /// versions and `gitCommit` stay as published. Returns whether `current`
    /// changed; subscribers are notified as for any publish.
    pub async fn republish_app_identity(&self, region: ServerRegion) -> Result<bool, AppError> {
        let (manifest, previous) = {
            let _guard = match self.publish_locks.get(&region) {
                Some(lock) => lock.lock().await,
                None => return Err(AppError::InvalidServerRegion(region.as_str().to_string())),
            };
            let Some(previous) = self.state.current(region).await? else {
                return Ok(false);
            };
            let synced = self.synced_app_identity(region).await.ok();
            let Some(app) = self.effective_identity(region, synced).await? else {
                return Ok(false);
            };
            if previous.app_version == app.app_version && previous.app_hash == app.app_hash {
                return Ok(false);
            }
            let mut manifest = previous.clone();
            manifest.app_version = app.app_version;
            manifest.app_hash = app.app_hash;
            manifest.generated_at =
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            if !self.state.publish(region, &manifest).await? {
                return Ok(false);
            }
            (manifest, previous)
        };
        info!(
            "{} Published app identity: appVersion {} -> {} (dataVersion {})",
            region.as_str().to_uppercase(),
            previous.app_version,
            manifest.app_version,
            manifest.data_version
        );
        self.notify_subscribers(region, &publish_notice(&manifest, Some(&previous)))
            .await;
        Ok(true)
    }

    async fn republish_app_identity_logged(&self, region: ServerRegion) {
        if let Err(e) = self.republish_app_identity(region).await {
            warn!(
                "{} Republishing current with the app identity failed (next poll retries): {}",
                region.as_str().to_uppercase(),
                e
            );
        }
    }

    /// `app_version` through the region's `pin_app_version_patch_to_zero`.
    fn effective_app_version(&self, region: ServerRegion, app_version: &str) -> String {
        crate::client::helper::effective_app_version(
            crate::config::pins_app_version_patch_to_zero(&self.config.servers, region),
            app_version,
        )
    }

    /// Regions this registry manages: those with a master directory.
    pub fn regions(&self) -> Vec<ServerRegion> {
        let mut regions: Vec<_> = self
            .config
            .servers
            .iter()
            .filter(|(_, server)| !server.master_dir.is_empty())
            .map(|(region, _)| *region)
            .collect();
        regions.sort();
        regions
    }

    /// `(master_dir, version_path)` for a managed region.
    pub fn region_paths(&self, region: ServerRegion) -> Result<(String, String), AppError> {
        self.config
            .servers
            .get(&region)
            .filter(|server| !server.master_dir.is_empty())
            .map(|server| (server.master_dir.clone(), server.version_path.clone()))
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "region {} has no master directory configured",
                    region.as_str()
                ))
            })
    }

    /// Effective app identity for a region: the operator override when one
    /// is stored, otherwise what the region's version file (synced from the
    /// owner, i.e. the identity the owner logged in with) says.
    pub async fn app_identity(&self, region: ServerRegion) -> Result<AppInfo, AppError> {
        // Both sources go through the region's pin: an override stored
        // before it applied here, or a version file written with another
        // setting, may carry the raw version; report what the nodes store.
        let synced = self.synced_app_identity(region).await;
        match self
            .effective_identity(region, synced.as_ref().ok().cloned())
            .await?
        {
            Some(info) => Ok(info),
            None => synced,
        }
    }

    /// The override when one is stored, else `synced` (the version file's
    /// identity, `None` when there is none).
    async fn effective_identity(
        &self,
        region: ServerRegion,
        synced: Option<AppInfo>,
    ) -> Result<Option<AppInfo>, AppError> {
        if let Some(mut info) = self.state.app_identity(region).await? {
            info.app_version = self.effective_app_version(region, &info.app_version);
            if let Some(synced) = &synced {
                self.warn_if_override_is_older(region, &info, synced);
            }
            return Ok(Some(info));
        }
        Ok(synced)
    }

    /// The app identity in the region's version file (synced from the owner).
    async fn synced_app_identity(&self, region: ServerRegion) -> Result<AppInfo, AppError> {
        let (_, version_path) = self.region_paths(region)?;
        if version_path.is_empty() {
            return Err(AppError::NotFound(format!(
                "region {} has no version file configured",
                region.as_str()
            )));
        }
        let data = tokio::fs::read(&version_path).await?;
        let version: crate::client::helper::VersionInfo = sonic_rs::from_slice(&data)
            .map_err(|e| AppError::ParseError(format!("version file: {e}")))?;
        Ok(AppInfo {
            app_version: self.effective_app_version(region, &version.app_version),
            app_hash: version.app_hash,
        })
    }

    /// An override keeps winning after the owner has moved past it (a later
    /// app-identity push): say so, once per synced version, so it is cleared.
    fn warn_if_override_is_older(&self, region: ServerRegion, stored: &AppInfo, synced: &AppInfo) {
        let older = crate::client::helper::cmp_version(&stored.app_version, &synced.app_version)
            .map(|o| o == std::cmp::Ordering::Less)
            .unwrap_or(false);
        let mut warned = self.stale_override_warned.lock();
        if !older {
            warned.remove(&region);
            return;
        }
        if warned.get(&region) == Some(&synced.app_version) {
            return;
        }
        warned.insert(region, synced.app_version.clone());
        warn!(
            "{} App identity override {} is older than the owner's {}; it still wins \
(DELETE /v1/app/{} to follow the owner)",
            region.as_str().to_uppercase(),
            stored.app_version,
            synced.app_version,
            region.as_str()
        );
    }

    /// Build the region's manifest from disk, record it, and notify
    /// subscribers when it changed. Returns the manifest and whether it was
    /// a new publish.
    pub async fn publish(&self, region: ServerRegion) -> Result<(MasterManifest, bool), AppError> {
        let (manifest, changed, previous) = {
            // The lock covers manifest build and state write only; the
            // subscriber fan-out (up to one timeout per unreachable peer)
            // runs after it is released so a concurrent refresh is not held up.
            let _guard = match self.publish_locks.get(&region) {
                Some(lock) => lock.lock().await,
                None => return Err(AppError::InvalidServerRegion(region.as_str().to_string())),
            };
            let (master_dir, version_path) = self.region_paths(region)?;
            let dir = master_dir.clone();
            let mut manifest = tokio::task::spawn_blocking(move || {
                build_master_manifest(region, &dir, &version_path)
            })
            .await
            .map_err(|e| AppError::Internal(format!("manifest task: {e}")))??;
            // `current` carries the effective identity, as `GET /v1/app`.
            let synced = AppInfo {
                app_version: self.effective_app_version(region, &manifest.app_version),
                app_hash: std::mem::take(&mut manifest.app_hash),
            };
            if let Some(app) = self.effective_identity(region, Some(synced)).await? {
                manifest.app_version = app.app_version;
                manifest.app_hash = app.app_hash;
            }
            // Content first: the manifest is committed only once every file
            // it lists can be served from the blob store.
            let stats = self
                .blobs
                .store_manifest(
                    std::path::Path::new(&master_dir),
                    &manifest,
                    StoreMode::Strict,
                )
                .await?;
            if stats.stored > 0 {
                info!(
                    "{} Stored {} new blob(s): {} -> {} bytes",
                    region.as_str().to_uppercase(),
                    stats.stored,
                    stats.bytes_in,
                    stats.bytes_stored
                );
            }
            let previous = self.state.current(region).await.ok().flatten();
            let changed = self.state.publish(region, &manifest).await?;
            (manifest, changed, previous)
        };
        if changed {
            info!(
                "{} Published master dataVersion {} ({} files)",
                region.as_str().to_uppercase(),
                manifest.data_version,
                manifest.files.len()
            );
            self.notify_subscribers(region, &publish_notice(&manifest, previous.as_ref()))
                .await;
            self.collect_garbage().await;
        }
        Ok((manifest, changed))
    }

    /// One bounded blob garbage collection pass (pg store; failures only
    /// logged, the next publish retries).
    pub async fn collect_garbage(&self) {
        if let Err(e) = self.blobs.collect_garbage(&self.state).await {
            warn!("Registry blob GC failed: {}", e);
        }
    }

    /// Store the blobs of every region's current manifest and retained
    /// snapshots that the blob store is missing (switching an existing
    /// deployment to `blob_store: pg`). Files come from the master directory
    /// when they still match, else from the manifest's git commit; anything
    /// found nowhere is skipped with a warning. Idempotent and a no-op for
    /// the fs store; one file in memory at a time.
    pub async fn import_blobs(&self) -> StoreStats {
        let mut total = StoreStats::default();
        if self.blobs.kind() == crate::config::BlobStoreKind::Fs {
            return total;
        }
        let started = std::time::Instant::now();
        for region in self.regions() {
            let Ok((master_dir, _)) = self.region_paths(region) else {
                continue;
            };
            let dir = std::path::Path::new(&master_dir);
            let mut manifests = Vec::new();
            match self.state.current(region).await {
                Ok(Some(current)) => manifests.push(current),
                Ok(None) => {}
                Err(e) => warn!(
                    "{} Blob import: unreadable current manifest: {}",
                    region.as_str().to_uppercase(),
                    e
                ),
            }
            match self.state.snapshots(region).await {
                Ok(snapshots) => manifests.extend(snapshots),
                Err(e) => warn!(
                    "{} Blob import: unreadable snapshots: {}",
                    region.as_str().to_uppercase(),
                    e
                ),
            }
            let mut seen = std::collections::HashSet::new();
            for manifest in manifests {
                if !seen.insert(super::state::content_hash(&manifest)) {
                    continue;
                }
                // Hold the publish lock so an import never races a publish
                // of the same region over the same files.
                let _guard = match self.publish_locks.get(&region) {
                    Some(lock) => lock.lock().await,
                    None => continue,
                };
                match self
                    .blobs
                    .store_manifest(dir, &manifest, StoreMode::Lenient)
                    .await
                {
                    Ok(stats) => total += stats,
                    Err(e) => warn!(
                        "{} Blob import of {} failed: {}",
                        region.as_str().to_uppercase(),
                        super::state::content_hash(&manifest),
                        e
                    ),
                }
            }
        }
        if total.stored > 0 || total.unavailable > 0 {
            info!(
                "Blob import done in {:.1}s: {} stored ({} -> {} bytes), {} present, {} unavailable",
                started.elapsed().as_secs_f64(),
                total.stored,
                total.bytes_in,
                total.bytes_stored,
                total.reused,
                total.unavailable
            );
        }
        self.collect_garbage().await;
        total
    }

    /// Pull the region from its owner if newer, then publish. Returns whether
    /// `current` changed (new master data, or only the app identity).
    pub async fn refresh(&self, region: ServerRegion) -> Result<bool, AppError> {
        let Some(syncer) = self.syncers.get(&region) else {
            return Err(AppError::NotFound(format!(
                "no master sync configured for region {}",
                region.as_str()
            )));
        };
        if syncer.sync_once().await? {
            self.publish(region).await?;
            return Ok(true);
        }
        // No new master data, but the app identity may have moved (the
        // syncer followed the owner's, or an override changed).
        self.republish_app_identity(region).await
    }

    /// Startup pass: publish every managed region whose directory already
    /// holds master files but has no recorded manifest yet, so `current` is
    /// available before the first sync.
    pub async fn publish_missing(&self) {
        for region in self.regions() {
            match self.state.current(region).await {
                Ok(Some(_)) => continue,
                Ok(None) => {}
                Err(e) => {
                    warn!(
                        "{} Unreadable registry state, republishing: {}",
                        region.as_str().to_uppercase(),
                        e
                    );
                }
            }
            match self.publish(region).await {
                Ok(_) => {}
                Err(AppError::IoError(e)) => info!(
                    "{} Not published at startup (no master data yet): {}",
                    region.as_str().to_uppercase(),
                    e
                ),
                Err(e) => error!(
                    "{} Startup publish failed: {}",
                    region.as_str().to_uppercase(),
                    e
                ),
            }
        }
    }

    /// Push an app identity to every configured account node
    /// (`POST /internal/app-identity`), returning one outcome per node.
    pub async fn push_app_identity(
        &self,
        region: ServerRegion,
        info: &AppInfo,
    ) -> Vec<AppIdentityPush> {
        let payload = serde_json::json!({
            "server": region.as_str(),
            "appVersion": info.app_version,
            "appHash": info.app_hash,
        });
        // Bounded fan-out; `buffered` keeps configuration order in the result.
        const PUSH_CONCURRENCY: usize = 4;
        use futures::StreamExt;
        let nodes: Vec<crate::config::MasterSyncPeer> = self.config.registry.account_nodes.clone();
        futures::stream::iter(nodes)
            .map(|node| {
                let payload = payload.clone();
                async move { self.push_app_identity_to(region, &node, payload).await }
            })
            .buffered(PUSH_CONCURRENCY)
            .collect()
            .await
    }

    async fn push_app_identity_to(
        &self,
        region: ServerRegion,
        node: &crate::config::MasterSyncPeer,
        payload: serde_json::Value,
    ) -> AppIdentityPush {
        {
            let endpoint = format!("{}/internal/app-identity", node.url.trim_end_matches('/'));
            let mut req = self.http.post(&endpoint).json(&payload);
            if !node.token.is_empty() {
                req = req.bearer_auth(&node.token);
            }
            let outcome = match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    let body: serde_json::Value = resp.json().await.unwrap_or_default();
                    if body.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                        info!(
                            "{} App identity pushed to {}",
                            region.as_str().to_uppercase(),
                            node.url
                        );
                        AppIdentityPush {
                            url: node.url.clone(),
                            ok: true,
                            message: None,
                        }
                    } else {
                        let message = body
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("rejected")
                            .to_string();
                        warn!(
                            "{} Account node {} rejected app identity: {}",
                            region.as_str().to_uppercase(),
                            node.url,
                            message
                        );
                        AppIdentityPush {
                            url: node.url.clone(),
                            ok: false,
                            message: Some(message),
                        }
                    }
                }
                Ok(resp) => {
                    warn!(
                        "{} Account node {} returned {}",
                        region.as_str().to_uppercase(),
                        node.url,
                        resp.status()
                    );
                    AppIdentityPush {
                        url: node.url.clone(),
                        ok: false,
                        message: Some(format!("HTTP {}", resp.status())),
                    }
                }
                Err(e) => {
                    warn!(
                        "{} Failed to push app identity to {}: {}",
                        region.as_str().to_uppercase(),
                        node.url,
                        e
                    );
                    AppIdentityPush {
                        url: node.url.clone(),
                        ok: false,
                        message: Some(e.to_string()),
                    }
                }
            };
            outcome
        }
    }

    async fn notify_subscribers(&self, region: ServerRegion, payload: &serde_json::Value) {
        for peer in &self.config.registry.subscribers {
            let endpoint = format!("{}/internal/master-updated", peer.url.trim_end_matches('/'));
            let mut req = self.http.post(&endpoint).json(payload);
            if !peer.token.is_empty() {
                req = req.bearer_auth(&peer.token);
            }
            match req.send().await {
                Ok(resp) if resp.status().is_success() => info!(
                    "{} Notified subscriber {}",
                    region.as_str().to_uppercase(),
                    peer.url
                ),
                Ok(resp) => warn!(
                    "{} Subscriber {} returned {}",
                    region.as_str().to_uppercase(),
                    peer.url,
                    resp.status()
                ),
                Err(e) => warn!(
                    "{} Failed to notify subscriber {}: {}",
                    region.as_str().to_uppercase(),
                    peer.url,
                    e
                ),
            }
        }
    }

    /// Schedule the per-region fallback polls (`master_sync.poll_cron`), each
    /// running `refresh`, plus the music_metas tick. Owner webhooks remain the
    /// primary trigger for master data.
    pub async fn start_polls(self: &Arc<Self>) -> Result<JobScheduler, JobSchedulerError> {
        let scheduler = JobScheduler::new().await?;
        if self.metas.is_some() {
            let cron = self.config.registry.music_metas.cron.clone();
            let registry = self.clone();
            // First pull right away so `current` exists before the first tick.
            tokio::spawn({
                let registry = registry.clone();
                async move {
                    if let Some(metas) = &registry.metas {
                        metas.refresh_all().await;
                    }
                }
            });
            match Job::new_async(cron.as_str(), move |_uuid, _lock| {
                let registry = registry.clone();
                Box::pin(async move {
                    if let Some(metas) = &registry.metas {
                        metas.refresh_all().await;
                    }
                })
            }) {
                Ok(job) => {
                    scheduler.add(job).await?;
                    info!("music_metas tick scheduled: {}", cron);
                }
                Err(e) => error!("Invalid music_metas cron '{}': {}", cron, e),
            }
        }
        for (region, syncer) in &self.syncers {
            let _ = syncer;
            let cron = self
                .config
                .servers
                .get(region)
                .map(|s| s.master_sync.poll_cron.clone())
                .unwrap_or_default();
            if cron.is_empty() {
                continue;
            }
            let registry = self.clone();
            let region = *region;
            let job = Job::new_async(cron.as_str(), move |_uuid, _lock| {
                let registry = registry.clone();
                Box::pin(async move {
                    if let Err(e) = registry.refresh(region).await {
                        error!(
                            "{} Registry poll failed: {}",
                            region.as_str().to_uppercase(),
                            e
                        );
                    }
                })
            });
            match job {
                Ok(job) => {
                    scheduler.add(job).await?;
                    info!(
                        "{} Registry poll scheduled: {}",
                        region.as_str().to_uppercase(),
                        cron
                    );
                }
                Err(e) => error!(
                    "{} Invalid poll cron '{}': {}",
                    region.as_str().to_uppercase(),
                    cron,
                    e
                ),
            }
        }
        scheduler.start().await?;
        Ok(scheduler)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::internal::MasterManifestFile;

    fn manifest(files: &[(&str, &str)]) -> MasterManifest {
        MasterManifest {
            server: "jp".into(),
            app_version: String::new(),
            app_hash: String::new(),
            data_version: "1.0.0".into(),
            asset_version: String::new(),
            asset_hash: String::new(),
            cdn_version: 0,
            generated_at: String::new(),
            content_hash: String::new(),
            git_commit: Some("abc".into()),
            files: files
                .iter()
                .map(|(name, sha)| MasterManifestFile {
                    name: name.to_string(),
                    size: 1,
                    sha256: sha.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn publish_notice_lists_the_file_diff() {
        let old = manifest(&[("a.json", "1"), ("b.json", "2"), ("gone.json", "3")]);
        let new = manifest(&[("a.json", "1"), ("b.json", "9"), ("new.json", "4")]);
        let notice = publish_notice(&new, Some(&old));
        assert_eq!(notice["server"], "jp");
        assert_eq!(notice["dataVersion"], "1.0.0");
        assert_eq!(notice["gitCommit"], "abc");
        assert_eq!(
            notice["contentHash"],
            super::super::state::content_hash(&new)
        );
        assert_eq!(
            notice["changedFiles"],
            serde_json::json!(["b.json", "new.json"])
        );
        assert_eq!(notice["removedFiles"], serde_json::json!(["gone.json"]));
        let first = publish_notice(&new, None);
        assert_eq!(first["changedFiles"].as_array().unwrap().len(), 3);
        assert_eq!(first["removedFiles"], serde_json::json!([]));
    }
}
