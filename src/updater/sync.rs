//! Master-data synchronization from a peer node (the region's "owner").
//!
//! The owner node downloads master data from the game/CDN with its own
//! accounts (see [`super::master::MasterUpdater`]); other nodes pull the
//! result from the owner over the internal network instead. Two triggers feed
//! [`MasterSyncer::sync_once`]: the owner's webhook (`POST
//! /internal/master-updated`, immediate) and a fallback cron poll comparing
//! versions, so a missed webhook self-heals. The bundle is a plain tar of the
//! owner's master directory plus its version file (transport compression is
//! negotiated per-request via Accept-Encoding, unless `compress_bundle` is
//! off); after unpacking, the same ingest / version-merge / git-push pipeline
//! as a locally downloaded update runs, so a pulling node can still be the git
//! publisher. Unpacking leaves a file whose bytes did not change untouched, so
//! its mtime (and with it git's index and the digest caches) stays valid.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use tracing::{error, info, warn};

use super::git::GitHelper;
use super::master::{
    adopt_peer_app_identity, is_safe_path_component, persist_app_identity, persist_version_file,
    AppIdentity,
};
use super::prune::{prune_stale_master_files, PrunePolicy};
use crate::client::helper::VersionInfo;
use crate::client::SekaiClient;
use crate::config::{Config, ServerRegion};
use crate::error::AppError;

/// Name of the tar entry carrying the owner's version file. Uses characters
/// that can never collide with a master table name.
pub const BUNDLE_VERSION_ENTRY: &str = "__haruki_version__.json";

/// A bundle's embedded version (when present) and its master file names.
type UnpackedBundle = (Option<VersionInfo>, HashSet<String>);

/// Decide whether the remote (owner) master state warrants a pull: the owner
/// is authoritative, so any dataVersion difference counts, as does a newer
/// Nuverse cdnVersion.
fn need_sync(remote: &VersionInfo, local: &VersionInfo) -> bool {
    (!remote.data_version.trim().is_empty() && remote.data_version != local.data_version)
        || remote.cdn_version > local.cdn_version
}

pub struct MasterSyncer {
    region: ServerRegion,
    master_dir: String,
    version_path: String,
    source_url: String,
    source_token: String,
    /// Let the owner compress the bundle (see `MasterSyncConfig`).
    compress_bundle: bool,
    /// The region's `pin_app_version_patch_to_zero`, applied to the owner's
    /// app identity when it is adopted.
    pin_app_version_patch_to_zero: bool,
    http: reqwest::Client,
    master_db: Option<sea_orm::DatabaseConnection>,
    /// Files ingested at once when `master_db` is set.
    ingest_concurrency: usize,
    git_helper: Option<GitHelper>,
    /// The local client, when this node also serves the region itself; its
    /// in-memory version (and, on an app-identity change, its login headers)
    /// is updated after a pull.
    client: Option<Arc<SekaiClient>>,
    /// Serializes version-file writes with any local updaters for the region.
    version_lock: Arc<tokio::sync::Mutex<()>>,
    /// Collapses concurrent triggers (webhook + poll) into one running sync.
    sync_lock: tokio::sync::Mutex<()>,
    /// Set when the last DB ingest failed so the next trigger retries even
    /// with an unchanged version (same contract as MasterUpdater).
    ingest_failed: AtomicBool,
    /// Outcome of the last git push attempt, so an operator-facing endpoint can
    /// report a mirror that stopped moving. A failed push is otherwise invisible:
    /// publishing continues and only a log line records it.
    git_state: parking_lot::Mutex<Option<GitPushState>>,
    /// Stale-file pruning after a complete bundle unpack (the bundle is the
    /// owner's whole master directory, so it is the authoritative file set).
    prune: PrunePolicy,
}

/// What happened the last time this region's mirror was pushed. `reason` is a
/// closed set rather than the git error text: this is reported by `/health`,
/// and git's stderr carries remote URLs and local paths. The full error is in
/// the log line emitted next to it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitPushState {
    pub ok: bool,
    pub at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<GitPushFailure>,
}

/// Why a push did not land, in terms an operator can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GitPushFailure {
    /// The mirror carries commits this clone does not have; reconcile by hand.
    RemoteDiverged,
    /// Transport, credentials or the git invocation itself failed.
    PushFailed,
}

impl GitPushFailure {
    fn of(error: &AppError) -> Self {
        match error {
            AppError::UpstreamData(message) if message.contains("diverged") => Self::RemoteDiverged,
            _ => Self::PushFailed,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::RemoteDiverged => "remote_diverged",
            Self::PushFailed => "push_failed",
        }
    }
}

impl MasterSyncer {
    /// Pull-and-apply one sync cycle. Returns Ok(true) when new master data
    /// was applied, Ok(false) when already up to date or a sync is running.
    pub async fn sync_once(&self) -> Result<bool, AppError> {
        let _guard = match self.sync_lock.try_lock() {
            Ok(g) => g,
            Err(_) => {
                info!(
                    "{} Master sync already in progress, skipping",
                    self.region.as_str().to_uppercase()
                );
                return Ok(false);
            }
        };
        let remote = self.fetch_remote_version().await?;
        let local = self.load_local_version().await;
        let need = need_sync(&remote, &local);
        let retry_ingest = !need && self.ingest_failed.load(Ordering::Relaxed);
        if !need && !retry_ingest {
            self.follow_owner_app_identity(&remote).await?;
            let (changed, _) = self.ensure_compact_exports(None).await?;
            // The version is saved before pushing. After a crash the commit may
            // still be local, even though the in-memory failure state was lost.
            // Reconcile once after every start, then retry failures on each tick.
            let pending = self.git_push_pending();
            if pending {
                info!(
                    "{} Reconciling git mirror at the current version...",
                    self.region.as_str().to_uppercase()
                );
            }
            if changed || pending {
                self.git_push(&local.data_version).await;
            }
            return Ok(changed);
        }
        if retry_ingest {
            warn!(
                "{} Previous sync ingest failed; re-pulling current version...",
                self.region.as_str().to_uppercase()
            );
        } else {
            info!(
                "{} Master sync: remote dataVersion {} (local {}), pulling bundle...",
                self.region.as_str().to_uppercase(),
                remote.data_version,
                if local.data_version.is_empty() {
                    "<none>"
                } else {
                    &local.data_version
                }
            );
        }

        // The bundle embeds the owner's version file as snapshotted at tar
        // time; prefer it over the separately fetched version so the recorded
        // version always matches the files actually written.
        let (bundle_version, mut produced) = self.pull_and_unpack().await?;
        let version = bundle_version.unwrap_or(remote);
        let (_, exports) = self.ensure_compact_exports(Some(produced.clone())).await?;
        produced.extend(exports);
        self.prune_stale_files(produced).await?;

        self.ingest().await;

        let (merged, app_changed) = {
            let _vguard = self.version_lock.lock().await;
            // The owner's appVersion/appHash (what its accounts logged in
            // with) replace this node's before the merge, which otherwise lets
            // on-disk values win; forward only, like every peer adoption.
            let app_changed = self.adopt_owner_app_identity(&version).await?;
            let merged = persist_version_file(
                self.region,
                self.pin_app_version_patch_to_zero,
                &self.version_path,
                &version,
            )
            .await?;
            (merged, app_changed)
        };
        if let Some(ref client) = self.client {
            if app_changed {
                client.apply_version(merged.clone());
            } else {
                client.version_helper.update(merged.clone());
            }
        }
        self.git_push(&merged.data_version).await;
        info!(
            "{} Master sync complete (dataVersion {})",
            self.region.as_str().to_uppercase(),
            merged.data_version
        );
        Ok(true)
    }

    async fn ensure_compact_exports(
        &self,
        sources: Option<HashSet<String>>,
    ) -> Result<(bool, HashSet<String>), AppError> {
        if self.region != ServerRegion::Jp {
            return Ok((false, HashSet::new()));
        }
        let master_dir = PathBuf::from(&self.master_dir);
        tokio::task::spawn_blocking(move || {
            super::compact::ensure_exports(&master_dir, sources.as_ref())
        })
        .await
        .map_err(|e| AppError::Internal(format!("compact master export task: {e}")))?
    }

    /// Adopt the owner's app identity from `owner` (its version file or the
    /// bundle's copy) under [`adopt_peer_app_identity`]'s forward-only rule.
    /// Callers hold the version lock. A missing local version file is left
    /// to the version merge, which takes the owner's identity as is.
    async fn adopt_owner_app_identity(&self, owner: &VersionInfo) -> Result<bool, AppError> {
        let owner_app = AppIdentity {
            app_version: owner.app_version.clone(),
            app_hash: owner.app_hash.clone(),
        };
        if !owner_app.is_known()
            || !tokio::fs::try_exists(&self.version_path)
                .await
                .unwrap_or(false)
        {
            return Ok(false);
        }
        adopt_peer_app_identity(
            self.region,
            self.pin_app_version_patch_to_zero,
            &self.version_path,
            &owner_app,
            "owner",
        )
        .await
    }

    /// Write an authoritative app identity (the registry's own `PUT /v1/app`)
    /// into this node's version file unconditionally, a downgrade included,
    /// so the forward-only adoption cannot keep a corrected version out of it.
    /// Returns whether the file changed; a missing file is left alone.
    pub async fn store_app_identity(&self, app: &AppIdentity) -> Result<bool, AppError> {
        let changed = {
            let _vguard = self.version_lock.lock().await;
            if !tokio::fs::try_exists(&self.version_path)
                .await
                .unwrap_or(false)
            {
                return Ok(false);
            }
            persist_app_identity(self.pin_app_version_patch_to_zero, &self.version_path, app)
                .await?
        };
        if changed {
            if let Some(ref client) = self.client {
                client.refresh_version().await?;
            }
        }
        Ok(changed)
    }

    /// Follow an app-identity change on the owner while its master data is
    /// unchanged: an app-identity push lands on the owner only, and waiting
    /// for the next master update would leave this node (and a registry's
    /// `current`) on the old appVersion.
    async fn follow_owner_app_identity(&self, owner: &VersionInfo) -> Result<(), AppError> {
        let changed = {
            let _vguard = self.version_lock.lock().await;
            self.adopt_owner_app_identity(owner).await?
        };
        if changed {
            if let Some(ref client) = self.client {
                client.refresh_version().await?;
            }
        }
        Ok(())
    }

    async fn fetch_remote_version(&self) -> Result<VersionInfo, AppError> {
        let url = format!(
            "{}/internal/master/{}/version",
            self.source_url.trim_end_matches('/'),
            self.region.as_str()
        );
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.source_token)
            .send()
            .await
            .map_err(|e| AppError::NetworkError(format!("sync source: {}", e)))?;
        if !resp.status().is_success() {
            return Err(AppError::NetworkError(format!(
                "sync source version endpoint returned {}",
                resp.status()
            )));
        }
        let info: VersionInfo = resp
            .json()
            .await
            .map_err(|e| AppError::ParseError(format!("sync source version: {}", e)))?;
        Ok(info)
    }

    async fn load_local_version(&self) -> VersionInfo {
        match tokio::fs::read(&self.version_path).await {
            Ok(data) => sonic_rs::from_slice(&data).unwrap_or_default(),
            Err(_) => VersionInfo::default(),
        }
    }

    /// Download the owner's bundle to a temp file, then unpack it into
    /// `master_dir` (each file written via temp+rename, matching the local
    /// updater's per-file atomicity). Returns the bundle's embedded version and
    /// the names of the master files it carried.
    async fn pull_and_unpack(&self) -> Result<UnpackedBundle, AppError> {
        let url = format!(
            "{}/internal/master/{}/bundle",
            self.source_url.trim_end_matches('/'),
            self.region.as_str()
        );
        let mut request = self.http.get(&url).bearer_auth(&self.source_token);
        if !self.compress_bundle {
            // An explicit `identity` keeps reqwest from advertising gzip/br/zstd,
            // so the owner's CompressionLayer sends the tar as it is.
            request = request.header(reqwest::header::ACCEPT_ENCODING, "identity");
        }
        let resp = request
            .send()
            .await
            .map_err(|e| AppError::NetworkError(format!("sync source: {}", e)))?;
        if !resp.status().is_success() {
            return Err(AppError::NetworkError(format!(
                "sync source bundle endpoint returned {}",
                resp.status()
            )));
        }

        let tmp_tar = std::env::temp_dir().join(format!(
            "haruki-master-pull-{}-{}.tar",
            self.region.as_str(),
            uuid::Uuid::new_v4()
        ));
        let result = async {
            let mut file = tokio::io::BufWriter::with_capacity(
                crate::utils::FILE_STREAM_CHUNK,
                tokio::fs::File::create(&tmp_tar).await?,
            );
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| AppError::NetworkError(e.to_string()))?;
                file.write_all(&chunk).await?;
            }
            file.flush().await?;
            drop(file);

            tokio::fs::create_dir_all(&self.master_dir).await?;
            let tar_path = tmp_tar.clone();
            let master_dir = PathBuf::from(&self.master_dir);
            let region_upper = self.region.as_str().to_uppercase();
            tokio::task::spawn_blocking(move || {
                unpack_master_tar(&tar_path, &master_dir, &region_upper)
            })
            .await
            .map_err(|e| AppError::Internal(format!("unpack task: {}", e)))?
        }
        .await;
        let _ = tokio::fs::remove_file(&tmp_tar).await;
        result
    }

    /// Delete master files the bundle just unpacked did not carry. Runs only
    /// after the whole bundle unpacked without error.
    async fn prune_stale_files(&self, produced: HashSet<String>) -> Result<(), AppError> {
        let policy = self.prune.clone();
        let master_dir = PathBuf::from(&self.master_dir);
        let version_path = self.version_path.clone();
        let region_upper = self.region.as_str().to_uppercase();
        tokio::task::spawn_blocking(move || {
            prune_stale_master_files(
                &master_dir,
                &version_path,
                &produced,
                &policy,
                &region_upper,
            )
        })
        .await
        .map_err(|e| AppError::Internal(format!("prune task: {}", e)))??;
        Ok(())
    }

    /// Best-effort DB ingestion, same contract as the local updater: failures
    /// are loud but do not fail the sync (files on disk are already valid),
    /// and `ingest_failed` makes the next trigger retry.
    async fn ingest(&self) {
        let Some(db) = self.master_db.clone() else {
            return;
        };
        let region_upper = self.region.as_str().to_uppercase();
        info!(
            "{} Starting database ingestion for synced master data...",
            region_upper
        );
        let ok = match crate::ingest_engine::IngestionEngine::new(db).await {
            Ok(engine) => match engine
                .with_concurrency(self.ingest_concurrency)
                .ingest_master_data(&self.master_dir, self.region.as_str())
                .await
            {
                Ok(()) => {
                    info!("{} Synced master data ingested into database", region_upper);
                    true
                }
                Err(e) => {
                    error!(
                        "{} Synced master data DB ingestion failed (files saved; will retry on \
next trigger): {e:#}",
                        region_upper
                    );
                    false
                }
            },
            Err(e) => {
                error!(
                    "{} Failed to initialize ingestion engine for sync (will retry on next \
trigger): {e:#}",
                    region_upper
                );
                false
            }
        };
        self.ingest_failed.store(!ok, Ordering::Relaxed);
    }

    /// The last git push outcome for this region, `None` before the first
    /// attempt or when this node does not push at all.
    pub fn git_state(&self) -> Option<GitPushState> {
        self.git_state.lock().clone()
    }

    /// An enabled mirror needs reconciliation until its first successful check
    /// in this process, including when a previous process died during a push.
    pub fn git_push_pending(&self) -> bool {
        self.git_helper.is_some() && self.git_state.lock().as_ref().is_none_or(|state| !state.ok)
    }

    fn record_git_state(&self, ok: bool, reason: Option<GitPushFailure>) {
        *self.git_state.lock() = Some(GitPushState {
            ok,
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            reason,
        });
    }

    async fn git_push(&self, data_version: &str) {
        let Some(ref git_helper) = self.git_helper else {
            return;
        };
        let git_helper = git_helper.clone();
        let master_dir = self.master_dir.clone();
        let data_version = data_version.to_string();
        let region_upper = self.region.as_str().to_uppercase();
        let push = tokio::task::spawn_blocking(move || {
            git_helper.push_changes(&master_dir, &data_version)
        })
        .await;
        match push {
            Ok(Ok(true)) => {
                info!("{} Git pushed synced changes successfully", region_upper);
                self.record_git_state(true, None);
            }
            Ok(Ok(false)) => self.record_git_state(true, None),
            Ok(Err(e)) => {
                error!("{} Git push after sync failed: {}", region_upper, e);
                self.record_git_state(false, Some(GitPushFailure::of(&e)));
            }
            Err(e) => {
                error!("{} Git push task after sync failed: {}", region_upper, e);
                self.record_git_state(false, Some(GitPushFailure::PushFailed));
            }
        }
    }
}

/// Unpack a master bundle tar: every entry must be a bare `<name>.json`
/// filename (no directories, no dot segments); anything else is skipped with a
/// warning. Files are written via temp+rename so concurrent readers never see
/// partial content. Returns the embedded version entry, when present, and the
/// names of the master files written.
fn unpack_master_tar(
    tar_path: &Path,
    master_dir: &Path,
    region_upper: &str,
) -> Result<UnpackedBundle, AppError> {
    use std::io::Read;

    let file = std::fs::File::open(tar_path)?;
    let mut archive = tar::Archive::new(std::io::BufReader::new(file));
    let mut version: Option<VersionInfo> = None;
    let mut written = 0usize;
    let mut unchanged = 0usize;
    let mut skipped = 0usize;
    let mut produced = HashSet::new();
    for entry in archive.entries().map_err(AppError::from)? {
        let mut entry = entry.map_err(AppError::from)?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let name = match entry.path() {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if !is_safe_path_component(&name) || !name.ends_with(".json") {
            warn!("{} Skipping unsafe bundle entry {:?}", region_upper, name);
            skipped += 1;
            continue;
        }
        let mut contents = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut contents).map_err(AppError::from)?;
        if name == BUNDLE_VERSION_ENTRY {
            match sonic_rs::from_slice::<VersionInfo>(&contents) {
                Ok(v) => version = Some(v),
                Err(e) => warn!("{} Invalid bundle version entry: {}", region_upper, e),
            }
            continue;
        }
        let target = master_dir.join(&name);
        if same_file_contents(&target, &contents) {
            // Rewriting identical bytes would only bump the mtime, which makes
            // git and the digest caches re-hash the file.
            unchanged += 1;
        } else {
            let tmp = master_dir.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
            std::fs::write(&tmp, &contents)?;
            std::fs::rename(&tmp, &target)?;
            written += 1;
        }
        // Unchanged files are part of the bundle too: prune keys on this set.
        produced.insert(name);
    }
    info!(
        "{} Unpacked {} master files from bundle ({} written, {} unchanged, {} skipped)",
        region_upper,
        produced.len(),
        written,
        unchanged,
        skipped
    );
    if produced.is_empty() {
        return Err(AppError::UpstreamData(
            "bundle contained no master files".to_string(),
        ));
    }
    Ok((version, produced))
}

/// Whether `path` is a regular file holding exactly `expected`. The size is
/// checked first; the bytes are compared in chunks so a large table is never
/// read into a second full buffer. Any error counts as "different".
fn same_file_contents(path: &Path, expected: &[u8]) -> bool {
    use std::io::Read;

    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() == expected.len() as u64 => {}
        _ => return false,
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = vec![0u8; 64 * 1024];
    let mut offset = 0usize;
    loop {
        match file.read(&mut buf) {
            Ok(0) => return offset == expected.len(),
            Ok(n) => {
                let end = offset + n;
                if end > expected.len() || buf[..n] != expected[offset..end] {
                    return false;
                }
                offset = end;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
}

/// Build the per-region syncers for every region that configures a
/// `master_sync.source_url`. `version_locks` must be the same instances handed
/// to the scheduler so version-file writers stay serialized.
pub fn build_syncers(
    config: &Config,
    clients: &HashMap<ServerRegion, Arc<crate::client::SekaiClient>>,
    master_db: Option<sea_orm::DatabaseConnection>,
    version_locks: &HashMap<ServerRegion, Arc<tokio::sync::Mutex<()>>>,
) -> HashMap<ServerRegion, Arc<MasterSyncer>> {
    let proxy = if config.proxy.is_empty() {
        None
    } else {
        Some(config.proxy.clone())
    };
    let git_helper = if config.git.enabled {
        Some(GitHelper::new(&config.git, proxy))
    } else {
        None
    };
    // Dedicated client: no total timeout (bundles can be large over WAN), but
    // connect/read timeouts so a dead peer still fails fast.
    let http = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .read_timeout(Duration::from_secs(60))
        .tcp_keepalive(Duration::from_secs(60))
        .build()
        .unwrap_or_default();

    let mut syncers = HashMap::new();
    for (region, server_config) in &config.servers {
        let sync = &server_config.master_sync;
        if sync.source_url.is_empty() {
            continue;
        }
        if server_config.master_dir.is_empty() || server_config.version_path.is_empty() {
            warn!(
                "{} master_sync.source_url set but master_dir/version_path missing; sync disabled",
                region.as_str().to_uppercase()
            );
            continue;
        }
        let version_lock = version_locks
            .get(region)
            .cloned()
            .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(())));
        syncers.insert(
            *region,
            Arc::new(MasterSyncer {
                region: *region,
                master_dir: server_config.master_dir.clone(),
                version_path: server_config.version_path.clone(),
                source_url: sync.source_url.clone(),
                source_token: sync.source_token.clone(),
                compress_bundle: sync.compress_bundle,
                pin_app_version_patch_to_zero: server_config.pin_app_version_patch_to_zero,
                http: http.clone(),
                master_db: master_db.clone(),
                ingest_concurrency: config.master_database.ingest_concurrency,
                git_helper: git_helper.clone(),
                client: clients.get(region).cloned(),
                version_lock,
                sync_lock: tokio::sync::Mutex::new(()),
                ingest_failed: AtomicBool::new(false),
                git_state: parking_lot::Mutex::new(None),
                prune: PrunePolicy::from_config(server_config),
            }),
        );
    }
    syncers
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::extract::State;
    use axum::http::{Response, Uri};
    use axum::Router;

    use super::*;

    #[derive(Clone)]
    struct SyncReply {
        version: Vec<u8>,
        bundle: Vec<u8>,
    }

    async fn sync_handler(State(reply): State<SyncReply>, uri: Uri) -> Response<Body> {
        let (content_type, body) = if uri.path().ends_with("/version") {
            ("application/json", reply.version)
        } else {
            ("application/x-tar", reply.bundle)
        };
        Response::builder()
            .header("content-type", content_type)
            .body(Body::from(body))
            .unwrap()
    }

    async fn spawn_sync_source(reply: SyncReply) -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().fallback(sync_handler).with_state(reply);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{address}"), task)
    }

    fn bundle_bytes(root: &Path, version: &VersionInfo) -> Vec<u8> {
        let source = root.join("bundle-source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("cards.json"), "[{\"id\":1}]").unwrap();
        std::fs::write(
            source.join(BUNDLE_VERSION_ENTRY),
            sonic_rs::to_string(version).unwrap(),
        )
        .unwrap();
        let tar_path = root.join("bundle.tar");
        let file = std::fs::File::create(&tar_path).unwrap();
        let mut builder = tar::Builder::new(file);
        builder
            .append_path_with_name(source.join("cards.json"), "cards.json")
            .unwrap();
        builder
            .append_path_with_name(source.join(BUNDLE_VERSION_ENTRY), BUNDLE_VERSION_ENTRY)
            .unwrap();
        builder.finish().unwrap();
        std::fs::read(tar_path).unwrap()
    }

    fn version(data: &str, cdn: i32) -> VersionInfo {
        VersionInfo {
            data_version: data.to_string(),
            cdn_version: cdn,
            ..Default::default()
        }
    }

    #[test]
    fn need_sync_on_data_version_difference_or_newer_cdn() {
        assert!(need_sync(&version("2.0.0.1", 0), &version("1.0.0.1", 0)));
        // Owner is authoritative: any difference counts, not just "newer".
        assert!(need_sync(&version("1.0.0.1", 0), &version("2.0.0.1", 0)));
        assert!(need_sync(&version("1.0.0.1", 5), &version("1.0.0.1", 4)));
        assert!(!need_sync(&version("1.0.0.1", 4), &version("1.0.0.1", 4)));
        // Empty remote dataVersion alone must not trigger a pull.
        assert!(!need_sync(&version("", 0), &version("1.0.0.1", 0)));
    }

    #[test]
    fn unpack_rejects_unsafe_entries_and_reads_version() {
        let dir = std::env::temp_dir().join(format!("haruki_sync_test_{}", uuid::Uuid::new_v4()));
        let src = dir.join("src");
        let out = dir.join("out");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&out).unwrap();

        let tar_path = dir.join("bundle.tar");
        {
            let file = std::fs::File::create(&tar_path).unwrap();
            let mut b = tar::Builder::new(file);
            let add = |b: &mut tar::Builder<std::fs::File>, name: &str, data: &[u8]| {
                let mut header = tar::Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(0o600);
                header.set_cksum();
                b.append_data(&mut header, name, data).unwrap();
            };
            add(&mut b, "events.json", br#"[{"id":1}]"#);
            add(&mut b, "cards.json", br#"[]"#);
            // The tar crate itself refuses ".." components at build time, so a
            // nested path stands in for "any entry that is not a bare filename".
            add(&mut b, "nested/escape.json", b"{}");
            add(&mut b, "not_json.txt", b"x");
            add(
                &mut b,
                BUNDLE_VERSION_ENTRY,
                br#"{"appVersion":"1.0.0","appHash":"h","dataVersion":"9.9.9.9","assetVersion":"1","assetHash":"","cdnVersion":7}"#,
            );
            b.finish().unwrap();
        }

        let (version, produced) = unpack_master_tar(&tar_path, &out, "TEST").unwrap();
        assert_eq!(version.unwrap().data_version, "9.9.9.9");
        let mut produced: Vec<_> = produced.into_iter().collect();
        produced.sort();
        assert_eq!(produced, ["cards.json", "events.json"]);
        assert!(out.join("events.json").exists());
        assert!(out.join("cards.json").exists());
        assert!(!out.join("escape.json").exists());
        assert!(!out.join("nested").exists());
        assert!(!out.join("not_json.txt").exists());
        // No stray temp files left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&out)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unpack_fails_on_bundle_with_no_master_files() {
        let dir = std::env::temp_dir().join(format!("haruki_sync_empty_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let tar_path = dir.join("empty.tar");
        {
            let file = std::fs::File::create(&tar_path).unwrap();
            let mut b = tar::Builder::new(file);
            b.finish().unwrap();
        }
        let err = unpack_master_tar(&tar_path, &dir, "TEST").unwrap_err();
        assert!(matches!(err, AppError::UpstreamData(_)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unpack_keeps_unchanged_files_and_rewrites_changed_ones() {
        use std::os::unix::fs::MetadataExt;

        let dir = std::env::temp_dir().join(format!("haruki_sync_keep_{}", uuid::Uuid::new_v4()));
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let seed = |name: &str, data: &[u8]| {
            let path = out.join(name);
            std::fs::write(&path, data).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(old)
                .unwrap();
        };
        let stat = |name: &str| {
            let meta = std::fs::metadata(out.join(name)).unwrap();
            (meta.modified().unwrap(), meta.ino())
        };
        seed("same.json", b"[1,2,3]");
        seed("resized.json", b"[1]");
        seed("edited.json", b"[1,2,3]");
        seed("target.json", b"[7]");
        std::os::unix::fs::symlink(out.join("target.json"), out.join("linked.json")).unwrap();
        let same_before = stat("same.json");

        let tar_path = dir.join("bundle.tar");
        let entries: [(&str, &[u8]); 5] = [
            ("same.json", b"[1,2,3]"),
            ("resized.json", b"[1,2]"),
            // Same size, different bytes.
            ("edited.json", b"[4,5,6]"),
            ("fresh.json", b"[]"),
            // Same bytes as the symlink's target: replaced by a regular file.
            ("linked.json", b"[7]"),
        ];
        std::fs::write(&tar_path, tar_of(&entries)).unwrap();
        let (_, produced) = unpack_master_tar(&tar_path, &out, "TEST").unwrap();
        let mut produced: Vec<_> = produced.into_iter().collect();
        produced.sort();
        assert_eq!(
            produced,
            [
                "edited.json",
                "fresh.json",
                "linked.json",
                "resized.json",
                "same.json"
            ]
        );
        assert_eq!(
            stat("same.json"),
            same_before,
            "unchanged file was rewritten"
        );
        for (name, data) in entries {
            assert_eq!(std::fs::read(out.join(name)).unwrap(), data, "{name}");
        }
        for name in ["resized.json", "edited.json"] {
            assert_ne!(stat(name).0, old, "{name} kept its old mtime");
        }
        assert!(!std::fs::symlink_metadata(out.join("linked.json"))
            .unwrap()
            .file_type()
            .is_symlink());

        // Unpacking the same bundle again writes nothing, and is not an error.
        let names: Vec<&str> = entries.iter().map(|(n, _)| *n).collect();
        let before: Vec<_> = names.iter().map(|n| stat(n)).collect();
        let (_, produced) = unpack_master_tar(&tar_path, &out, "TEST").unwrap();
        assert_eq!(produced.len(), entries.len());
        let after: Vec<_> = names.iter().map(|n| stat(n)).collect();
        assert_eq!(before, after);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn bundle_compression_follows_the_config() {
        use tower_http::compression::CompressionLayer;

        let root = std::env::temp_dir().join(format!("haruki_sync_enc_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let seen = Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
        let table = "[".to_string() + &"{\"id\":1},".repeat(500) + "{}]";
        let bundle = tar_of(&[("cards.json", table.as_bytes())]);
        let app = Router::new()
            .fallback({
                let seen = seen.clone();
                move |headers: axum::http::HeaderMap| {
                    let seen = seen.clone();
                    let bundle = bundle.clone();
                    async move {
                        seen.lock().push(
                            headers
                                .get("accept-encoding")
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("")
                                .to_string(),
                        );
                        Response::builder()
                            .header("content-type", "application/x-tar")
                            .body(Body::from(bundle))
                            .unwrap()
                    }
                }
            })
            // What the owner's router does with the bundle response.
            .layer(CompressionLayer::new());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        for compress in [true, false] {
            let mut config: Config = serde_yaml::from_str("backend: {}").unwrap();
            let mut server_config: crate::config::ServerConfig =
                serde_yaml::from_str("{}").unwrap();
            let master = root.join(format!("master-{compress}"));
            server_config.master_dir = master.to_string_lossy().into_owned();
            server_config.version_path = root.join("version.json").to_string_lossy().into_owned();
            server_config.master_sync.source_url = url.clone();
            server_config.master_sync.compress_bundle = compress;
            config.servers.insert(ServerRegion::Kr, server_config);
            let syncers = build_syncers(&config, &HashMap::new(), None, &HashMap::new());
            let (_, produced) = syncers[&ServerRegion::Kr].pull_and_unpack().await.unwrap();
            assert_eq!(produced.len(), 1);
            assert_eq!(
                std::fs::read_to_string(master.join("cards.json")).unwrap(),
                table
            );
            let accept = seen.lock().last().cloned().unwrap();
            if compress {
                assert!(accept.contains("zstd"), "{accept}");
            } else {
                assert_eq!(accept, "identity");
            }
        }
        // Absent from the YAML, the option keeps compression on.
        let parsed: crate::config::ServerConfig =
            serde_yaml::from_str("master_sync: {source_url: x}").unwrap();
        assert!(parsed.master_sync.compress_bundle);
        assert!(crate::config::MasterSyncConfig::default().compress_bundle);
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn sync_once_downloads_bundle_persists_version_and_skips_when_current() {
        let root = std::env::temp_dir().join(format!("haruki_sync_e2e_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let remote = VersionInfo {
            app_version: "6.0.1".to_string(),
            app_hash: "hash".to_string(),
            data_version: "2.0.0.1".to_string(),
            asset_version: "2.0.0.1".to_string(),
            asset_hash: "asset".to_string(),
            cdn_version: 2,
        };
        let (url, server) = spawn_sync_source(SyncReply {
            version: sonic_rs::to_string(&remote).unwrap().into_bytes(),
            bundle: bundle_bytes(&root, &remote),
        })
        .await;

        let mut config: Config = serde_yaml::from_str("backend: {}").unwrap();
        let mut server_config: crate::config::ServerConfig = serde_yaml::from_str("{}").unwrap();
        server_config.master_dir = root.join("master").to_string_lossy().into_owned();
        server_config.version_path = root.join("version.json").to_string_lossy().into_owned();
        // A stale local app identity must be replaced by the owner's.
        std::fs::write(
            root.join("version.json"),
            r#"{"appVersion":"0.0.1","appHash":"stale","dataVersion":"1.0.0.0"}"#,
        )
        .unwrap();
        server_config.master_sync.source_url = url;
        server_config.master_sync.source_token = "token".to_string();
        config.servers.insert(ServerRegion::Cn, server_config);
        let syncers = build_syncers(&config, &HashMap::new(), None, &HashMap::new());
        let syncer = syncers.get(&ServerRegion::Cn).unwrap();

        assert!(syncer.sync_once().await.unwrap());
        assert!(root.join("master/cards.json").exists());
        let local = syncer.load_local_version().await;
        assert_eq!(local.data_version, "2.0.0.1");
        assert_eq!(local.cdn_version, 2);
        // The owner's app version is kept as is (the patch pin is opt-in).
        assert_eq!(local.app_version, "6.0.1");
        assert_eq!(local.app_hash, "hash");
        assert!(!syncer.sync_once().await.unwrap());

        let guard = syncer.sync_lock.lock().await;
        assert!(!syncer.sync_once().await.unwrap());
        drop(guard);
        syncer.ingest().await;
        assert!(syncer.git_state().is_none(), "no push attempted yet");
        assert!(!syncer.git_push_pending());
        syncer.git_push("2.0.0.1").await;
        // Without a git helper the push is a no-op and records nothing.
        assert!(syncer.git_state().is_none());
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn restarted_syncer_recovers_committed_and_dirty_mirrors_at_current_version() {
        fn git(repo: &Path, args: &[&str]) -> String {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap()
        }

        fn syncer(config: &Config) -> Arc<MasterSyncer> {
            build_syncers(config, &HashMap::new(), None, &HashMap::new())
                .remove(&ServerRegion::Cn)
                .unwrap()
        }

        let root = std::env::temp_dir().join(format!("haruki_sync_git_{}", uuid::Uuid::new_v4()));
        let work = root.join("work");
        let remote = root.join("remote.git");
        let offline = root.join("offline.git");
        std::fs::create_dir_all(work.join("master")).unwrap();
        git(&root, &["init", "--bare", "-b", "main", "remote.git"]);
        git(&work, &["init", "-b", "main"]);
        git(
            &work,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        std::fs::write(work.join("master/cards.json"), "[]").unwrap();
        std::fs::write(
            work.join("version.json"),
            sonic_rs::to_vec(&version("1", 0)).unwrap(),
        )
        .unwrap();

        let mut config: Config = serde_yaml::from_str("backend: {}").unwrap();
        config.git.enabled = true;
        config.git.username = "test".into();
        config.git.email = "test@example.test".into();
        let helper = GitHelper::new(&config.git, None);
        helper.push_changes(work.to_str().unwrap(), "1").unwrap();
        let old_head = git(&remote, &["rev-parse", "main"]);
        let current = version("2", 0);
        let (url, source) = spawn_sync_source(SyncReply {
            version: sonic_rs::to_vec(&current).unwrap(),
            bundle: bundle_bytes(&root, &current),
        })
        .await;
        let mut settings: crate::config::ServerConfig = serde_yaml::from_str("{}").unwrap();
        settings.master_dir = work.join("master").to_string_lossy().into_owned();
        settings.version_path = work.join("version.json").to_string_lossy().into_owned();
        settings.master_sync.source_url = url;
        config.servers.insert(ServerRegion::Cn, settings);

        // The bundle and version are saved, and a commit is created, but its
        // push fails. The next process has only the repository to recover from.
        std::fs::rename(&remote, &offline).unwrap();
        let first = syncer(&config);
        assert!(first.sync_once().await.unwrap());
        assert!(first.git_push_pending());
        assert!(!first.git_state().unwrap().ok);
        assert_eq!(first.load_local_version().await.data_version, "2");
        let pending_head = git(&work, &["rev-parse", "HEAD"]);
        assert_ne!(pending_head, old_head);
        assert!(git(&work, &["status", "--porcelain"]).is_empty());
        drop(first);
        source.abort();

        // A same-version retry must not request a bundle: this source would
        // return an invalid archive if the restart tried to download it again.
        let (url, source) = spawn_sync_source(SyncReply {
            version: sonic_rs::to_vec(&current).unwrap(),
            bundle: Vec::new(),
        })
        .await;
        config
            .servers
            .get_mut(&ServerRegion::Cn)
            .unwrap()
            .master_sync
            .source_url = url;
        let restarted = syncer(&config);
        assert!(restarted.git_state().is_none());
        assert!(restarted.git_push_pending());
        assert!(!restarted.sync_once().await.unwrap());
        assert!(!restarted.git_state().unwrap().ok, "offline is not healthy");
        std::fs::rename(&offline, &remote).unwrap();
        assert!(!restarted.sync_once().await.unwrap());
        assert!(restarted.git_state().unwrap().ok);
        assert!(!restarted.git_push_pending());
        assert_eq!(git(&remote, &["rev-parse", "main"]), pending_head);
        assert_eq!(git(&work, &["rev-parse", "HEAD"]), pending_head);

        // Already delivered mirrors are checked once after restart without
        // creating another commit or doing network git on every later tick.
        let clean = syncer(&config);
        assert!(!clean.sync_once().await.unwrap());
        assert!(!clean.git_push_pending());
        assert_eq!(git(&work, &["rev-parse", "HEAD"]), pending_head);
        std::fs::rename(&remote, &offline).unwrap();
        assert!(!clean.sync_once().await.unwrap());
        assert!(clean.git_state().unwrap().ok);
        std::fs::rename(&offline, &remote).unwrap();

        // A crash before the commit is recoverable too: the version may already
        // be current while the updated master files are still uncommitted.
        std::fs::write(work.join("master/cards.json"), "[{\"id\":2}]").unwrap();
        let dirty = syncer(&config);
        assert!(!dirty.sync_once().await.unwrap());
        assert!(!dirty.git_push_pending());
        assert_ne!(git(&work, &["rev-parse", "HEAD"]), pending_head);
        assert_eq!(
            git(&remote, &["show", "main:master/cards.json"]),
            "[{\"id\":2}]"
        );
        assert!(git(&work, &["status", "--porcelain"]).is_empty());
        source.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn jp_exports_survive_sync_and_backfill_without_a_version_change() {
        let root =
            std::env::temp_dir().join(format!("haruki_compact_sync_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let remote = version("2.0.0.1", 0);
        let (url, server) = spawn_sync_source(SyncReply {
            version: sonic_rs::to_vec(&remote).unwrap(),
            bundle: bundle_bytes(&root, &remote),
        })
        .await;
        let mut config: Config = serde_yaml::from_str("backend: {}").unwrap();
        let mut settings: crate::config::ServerConfig = serde_yaml::from_str("{}").unwrap();
        settings.master_dir = root.join("master").to_string_lossy().into_owned();
        settings.version_path = root.join("version.json").to_string_lossy().into_owned();
        settings.master_sync.source_url = url;
        config.servers.insert(ServerRegion::Jp, settings);
        let syncers = build_syncers(&config, &HashMap::new(), None, &HashMap::new());
        let syncer = &syncers[&ServerRegion::Jp];
        assert!(syncer.sync_once().await.unwrap());
        let compact = root.join("master/compactCards.json");
        assert!(compact.exists());
        assert!(!syncer.sync_once().await.unwrap());
        std::fs::remove_file(&compact).unwrap();
        assert!(
            syncer.sync_once().await.unwrap(),
            "backfill triggers publication"
        );
        assert!(compact.exists());
        assert!(!syncer.sync_once().await.unwrap());
        // Re-pull the same source files: pruning must retain derived exports.
        std::fs::remove_file(root.join("version.json")).unwrap();
        assert!(syncer.sync_once().await.unwrap());
        assert!(compact.exists());
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn syncer_follows_the_owner_app_identity_forward_only() {
        let root = std::env::temp_dir().join(format!("haruki_sync_app_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("version.json"),
            r#"{"appVersion":"6.5.0","appHash":"local","dataVersion":"1.0.0.0"}"#,
        )
        .unwrap();
        let owner = |app_version: &str, app_hash: &str| VersionInfo {
            app_version: app_version.to_string(),
            app_hash: app_hash.to_string(),
            data_version: "2.0.0.1".to_string(),
            asset_version: "2.0.0.1".to_string(),
            asset_hash: String::new(),
            cdn_version: 2,
        };
        let syncer_for = |url: String| {
            let mut config: Config = serde_yaml::from_str("backend: {}").unwrap();
            let mut server_config: crate::config::ServerConfig =
                serde_yaml::from_str("{}").unwrap();
            server_config.master_dir = root.join("master").to_string_lossy().into_owned();
            server_config.version_path = root.join("version.json").to_string_lossy().into_owned();
            server_config.master_sync.source_url = url;
            config.servers.insert(ServerRegion::Tw, server_config);
            build_syncers(&config, &HashMap::new(), None, &HashMap::new())
                .remove(&ServerRegion::Tw)
                .unwrap()
        };
        let serve = |version: VersionInfo| {
            let bundle = bundle_bytes(&root, &version);
            spawn_sync_source(SyncReply {
                version: sonic_rs::to_string(&version).unwrap().into_bytes(),
                bundle,
            })
        };

        // A bundle from an owner that has not received the push yet brings
        // new master data but keeps this node's newer app identity.
        let (url, server) = serve(owner("6.4.0", "stale")).await;
        let syncer = syncer_for(url);
        assert!(syncer.sync_once().await.unwrap());
        let local = syncer.load_local_version().await;
        assert_eq!(local.data_version, "2.0.0.1");
        assert_eq!(
            (local.app_version.as_str(), local.app_hash.as_str()),
            ("6.5.0", "local")
        );
        server.abort();

        // The owner moves forward without new master data: followed at once,
        // no bundle pulled.
        let (url, server) = serve(owner("6.6.0", "pushed")).await;
        let syncer = syncer_for(url);
        assert!(!syncer.sync_once().await.unwrap());
        let local = syncer.load_local_version().await;
        assert_eq!(
            (local.app_version.as_str(), local.app_hash.as_str()),
            ("6.6.0", "pushed")
        );
        assert_eq!(local.data_version, "2.0.0.1");
        server.abort();

        // An authoritative store (the registry's own override) may go back.
        let app = AppIdentity {
            app_version: "6.5.5".to_string(),
            app_hash: "fixed".to_string(),
        };
        assert!(syncer.store_app_identity(&app).await.unwrap());
        let local = syncer.load_local_version().await;
        assert_eq!(
            (local.app_version.as_str(), local.app_hash.as_str()),
            ("6.5.5", "fixed")
        );
        // Storing the same identity again changes nothing.
        assert!(!syncer.store_app_identity(&app).await.unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn tar_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            builder.append_data(&mut header, name, *data).unwrap();
        }
        builder.into_inner().unwrap()
    }

    #[tokio::test]
    async fn sync_prunes_files_missing_from_a_complete_bundle_only() {
        let root = std::env::temp_dir().join(format!("haruki_sync_prune_{}", uuid::Uuid::new_v4()));
        let master = root.join("master");
        std::fs::create_dir_all(&master).unwrap();
        std::fs::write(master.join("dropped.json"), "[]").unwrap();
        // Protected: kept even though the owner's bundle lacks it.
        std::fs::write(master.join("events.json"), "[]").unwrap();
        let v2 = sonic_rs::to_string(&version("2.0.0.1", 0)).unwrap();
        let bundle_version = v2.as_bytes();
        let tables: Vec<(String, &[u8])> = "abcdefgh"
            .chars()
            .map(|c| (format!("{c}.json"), b"[]" as &[u8]))
            .collect();
        let tables: Vec<(&str, &[u8])> = tables.iter().map(|(n, d)| (n.as_str(), *d)).collect();
        let mut full_entries = tables.clone();
        full_entries.push((BUNDLE_VERSION_ENTRY, bundle_version));
        let full = tar_of(&full_entries);
        let (url, server) = spawn_sync_source(SyncReply {
            version: bundle_version.to_vec(),
            bundle: full,
        })
        .await;
        let mut config: Config = serde_yaml::from_str("backend: {}").unwrap();
        let mut server_config: crate::config::ServerConfig = serde_yaml::from_str("{}").unwrap();
        server_config.master_dir = master.to_string_lossy().into_owned();
        server_config.version_path = root.join("version.json").to_string_lossy().into_owned();
        server_config.master_sync.source_url = url;
        config
            .servers
            .insert(ServerRegion::Jp, server_config.clone());
        let syncers = build_syncers(&config, &HashMap::new(), None, &HashMap::new());
        assert!(syncers[&ServerRegion::Jp].sync_once().await.unwrap());
        assert!(!master.join("dropped.json").exists());
        assert!(master.join("events.json").exists());
        assert!(master.join("d.json").exists());
        server.abort();

        // A bundle that fails to unpack leaves every file in place.
        std::fs::write(master.join("dropped.json"), "[]").unwrap();
        let (url, server) = spawn_sync_source(SyncReply {
            version: sonic_rs::to_string(&version("3.0.0.1", 0))
                .unwrap()
                .into_bytes(),
            bundle: b"definitely not a tar archive".to_vec(),
        })
        .await;
        server_config.master_sync.source_url = url;
        config
            .servers
            .insert(ServerRegion::Jp, server_config.clone());
        let syncers = build_syncers(&config, &HashMap::new(), None, &HashMap::new());
        assert!(syncers[&ServerRegion::Jp].sync_once().await.is_err());
        assert!(master.join("dropped.json").exists());
        server.abort();

        // The cap applies to the syncer too: over it, nothing is deleted.
        let (url, server) = spawn_sync_source(SyncReply {
            version: sonic_rs::to_string(&version("4.0.0.1", 0))
                .unwrap()
                .into_bytes(),
            bundle: tar_of(&tables),
        })
        .await;
        server_config.master_sync.source_url = url;
        server_config.prune_max_files = 0;
        config
            .servers
            .insert(ServerRegion::Jp, server_config.clone());
        let syncers = build_syncers(&config, &HashMap::new(), None, &HashMap::new());
        assert!(syncers[&ServerRegion::Jp].sync_once().await.unwrap());
        assert!(master.join("dropped.json").exists());
        server.abort();

        // A bundle whose files are all unchanged still prunes what it lacks,
        // and leaves the unchanged files (and their mtimes) alone.
        let kept = std::fs::metadata(master.join("a.json"))
            .unwrap()
            .modified()
            .unwrap();
        let (url, server) = spawn_sync_source(SyncReply {
            version: sonic_rs::to_string(&version("4.5.0.1", 0))
                .unwrap()
                .into_bytes(),
            bundle: tar_of(&tables),
        })
        .await;
        server_config.master_sync.source_url = url;
        server_config.prune_max_files = 10;
        config
            .servers
            .insert(ServerRegion::Jp, server_config.clone());
        let syncers = build_syncers(&config, &HashMap::new(), None, &HashMap::new());
        assert!(syncers[&ServerRegion::Jp].sync_once().await.unwrap());
        assert!(!master.join("dropped.json").exists());
        assert!(master.join("events.json").exists());
        assert_eq!(
            std::fs::metadata(master.join("a.json"))
                .unwrap()
                .modified()
                .unwrap(),
            kept
        );
        server.abort();

        // `prune_stale: false` keeps files a complete bundle no longer carries.
        std::fs::write(master.join("dropped.json"), "[]").unwrap();
        let (url, server) = spawn_sync_source(SyncReply {
            version: sonic_rs::to_string(&version("5.0.0.1", 0))
                .unwrap()
                .into_bytes(),
            bundle: tar_of(&tables),
        })
        .await;
        server_config.master_sync.source_url = url;
        server_config.prune_max_files = 10;
        server_config.prune_stale = false;
        config.servers.insert(ServerRegion::Jp, server_config);
        let syncers = build_syncers(&config, &HashMap::new(), None, &HashMap::new());
        assert!(syncers[&ServerRegion::Jp].sync_once().await.unwrap());
        assert!(master.join("dropped.json").exists());
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn syncer_builder_filters_incomplete_configuration() {
        let root = std::env::temp_dir().join(format!("haruki_sync_build_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut config: Config = serde_yaml::from_str("backend: {}").unwrap();
        let empty: crate::config::ServerConfig = serde_yaml::from_str("{}").unwrap();
        config.servers.insert(ServerRegion::Jp, empty);
        let mut incomplete: crate::config::ServerConfig = serde_yaml::from_str("{}").unwrap();
        incomplete.master_sync.source_url = "http://127.0.0.1:1".to_string();
        config.servers.insert(ServerRegion::Tw, incomplete);
        assert!(build_syncers(&config, &HashMap::new(), None, &HashMap::new()).is_empty());

        let syncer = MasterSyncer {
            region: ServerRegion::Jp,
            master_dir: root.join("master").to_string_lossy().into_owned(),
            version_path: root.join("missing.json").to_string_lossy().into_owned(),
            source_url: "http://127.0.0.1:1".to_string(),
            source_token: String::new(),
            compress_bundle: true,
            pin_app_version_patch_to_zero: false,
            http: reqwest::Client::new(),
            master_db: None,
            ingest_concurrency: crate::ingest_engine::DEFAULT_INGEST_CONCURRENCY,
            git_helper: None,
            client: None,
            version_lock: Arc::new(tokio::sync::Mutex::new(())),
            sync_lock: tokio::sync::Mutex::new(()),
            ingest_failed: AtomicBool::new(false),
            git_state: parking_lot::Mutex::new(None),
            prune: PrunePolicy::default(),
        };
        assert_eq!(syncer.load_local_version().await.data_version, "");
        assert!(syncer.fetch_remote_version().await.is_err());
        assert!(syncer.pull_and_unpack().await.is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
