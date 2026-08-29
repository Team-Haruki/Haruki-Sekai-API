//! Master-data synchronization from a peer node (the region's "owner").
//!
//! The owner node downloads master data from the game/CDN with its own
//! accounts (see [`super::master::MasterUpdater`]); other nodes pull the
//! result from the owner over the internal network instead. Two triggers feed
//! [`MasterSyncer::sync_once`]: the owner's webhook (`POST
//! /internal/master-updated`, immediate) and a fallback cron poll comparing
//! versions, so a missed webhook self-heals. The bundle is a plain tar of the
//! owner's master directory plus its version file (transport compression is
//! negotiated per-request via Accept-Encoding); after unpacking, the same
//! ingest / version-merge / git-push pipeline as a locally downloaded update
//! runs, so a pulling node can still be the git publisher.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use tracing::{error, info, warn};

use super::git::GitHelper;
use super::master::{is_safe_path_component, persist_version_file};
use crate::client::helper::{VersionHelper, VersionInfo};
use crate::config::{Config, ServerRegion};
use crate::error::AppError;

/// Name of the tar entry carrying the owner's version file. Uses characters
/// that can never collide with a master table name.
pub const BUNDLE_VERSION_ENTRY: &str = "__haruki_version__.json";

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
    http: reqwest::Client,
    master_db: Option<sea_orm::DatabaseConnection>,
    git_helper: Option<GitHelper>,
    /// The local client's in-memory version state, when this node also serves
    /// the region itself; updated after a pull so request headers stay current.
    version_helper: Option<Arc<VersionHelper>>,
    /// Serializes version-file writes with any local updaters for the region.
    version_lock: Arc<tokio::sync::Mutex<()>>,
    /// Collapses concurrent triggers (webhook + poll) into one running sync.
    sync_lock: tokio::sync::Mutex<()>,
    /// Set when the last DB ingest failed so the next trigger retries even
    /// with an unchanged version (same contract as MasterUpdater).
    ingest_failed: AtomicBool,
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
            return Ok(false);
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
        let bundle_version = self.pull_and_unpack().await?;
        let version = bundle_version.unwrap_or(remote);

        self.ingest().await;

        let merged = {
            let _vguard = self.version_lock.lock().await;
            persist_version_file(self.region, &self.version_path, &version).await?
        };
        if let Some(ref helper) = self.version_helper {
            helper.update(merged.clone());
        }
        self.git_push(&merged.data_version).await;
        info!(
            "{} Master sync complete (dataVersion {})",
            self.region.as_str().to_uppercase(),
            merged.data_version
        );
        Ok(true)
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
    /// updater's per-file atomicity). Returns the bundle's embedded version.
    async fn pull_and_unpack(&self) -> Result<Option<VersionInfo>, AppError> {
        let url = format!(
            "{}/internal/master/{}/bundle",
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
            let mut file = tokio::fs::File::create(&tmp_tar).await?;
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
            Ok(Ok(true)) => info!("{} Git pushed synced changes successfully", region_upper),
            Ok(Ok(false)) => {}
            Ok(Err(e)) => error!("{} Git push after sync failed: {}", region_upper, e),
            Err(e) => error!("{} Git push task after sync failed: {}", region_upper, e),
        }
    }
}

/// Unpack a master bundle tar: every entry must be a bare `<name>.json`
/// filename (no directories, no dot segments); anything else is skipped with a
/// warning. Files are written via temp+rename so concurrent readers never see
/// partial content. Returns the embedded version entry, when present.
fn unpack_master_tar(
    tar_path: &Path,
    master_dir: &Path,
    region_upper: &str,
) -> Result<Option<VersionInfo>, AppError> {
    use std::io::Read;

    let file = std::fs::File::open(tar_path)?;
    let mut archive = tar::Archive::new(std::io::BufReader::new(file));
    let mut version: Option<VersionInfo> = None;
    let mut written = 0usize;
    let mut skipped = 0usize;
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
        let tmp = master_dir.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, &contents)?;
        std::fs::rename(&tmp, &target)?;
        written += 1;
    }
    info!(
        "{} Unpacked {} master files from bundle ({} skipped)",
        region_upper, written, skipped
    );
    if written == 0 {
        return Err(AppError::UpstreamData(
            "bundle contained no master files".to_string(),
        ));
    }
    Ok(version)
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
                http: http.clone(),
                master_db: master_db.clone(),
                git_helper: git_helper.clone(),
                version_helper: clients.get(region).map(|c| c.version_helper.clone()),
                version_lock,
                sync_lock: tokio::sync::Mutex::new(()),
                ingest_failed: AtomicBool::new(false),
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
                header.set_mode(0o644);
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

        let version = unpack_master_tar(&tar_path, &out, "TEST").unwrap();
        assert_eq!(version.unwrap().data_version, "9.9.9.9");
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
        assert!(!syncer.sync_once().await.unwrap());

        let guard = syncer.sync_lock.lock().await;
        assert!(!syncer.sync_once().await.unwrap());
        drop(guard);
        syncer.ingest().await;
        syncer.git_push("2.0.0.1").await;
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
            http: reqwest::Client::new(),
            master_db: None,
            git_helper: None,
            version_helper: None,
            version_lock: Arc::new(tokio::sync::Mutex::new(())),
            sync_lock: tokio::sync::Mutex::new(()),
            ingest_failed: AtomicBool::new(false),
        };
        assert_eq!(syncer.load_local_version().await.data_version, "");
        assert!(syncer.fetch_remote_version().await.is_err());
        assert!(syncer.pull_and_unpack().await.is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
