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
    /// that cannot be filled from anywhere is rejected.
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
            app_version: pick(
                &requested.app_version,
                "appVersion",
                current.as_ref().map(|c| &c.app_version),
            )?,
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
        let pushed = self.push_app_identity(region, &effective).await;
        Ok((effective, pushed))
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
        if let Some(info) = self.state.app_identity(region).await? {
            return Ok(info);
        }
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
            app_version: version.app_version,
            app_hash: version.app_hash,
        })
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
            let manifest = tokio::task::spawn_blocking(move || {
                build_master_manifest(region, &dir, &version_path)
            })
            .await
            .map_err(|e| AppError::Internal(format!("manifest task: {e}")))??;
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
    /// new master data was applied.
    pub async fn refresh(&self, region: ServerRegion) -> Result<bool, AppError> {
        let Some(syncer) = self.syncers.get(&region) else {
            return Err(AppError::NotFound(format!(
                "no master sync configured for region {}",
                region.as_str()
            )));
        };
        let applied = syncer.sync_once().await?;
        if applied {
            self.publish(region).await?;
        }
        Ok(applied)
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
