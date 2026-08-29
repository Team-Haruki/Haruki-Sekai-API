use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tracing::{error, info, warn};

use super::git::GitHelper;
use crate::client::helper::{compare_version, effective_app_version, VersionInfo};
use crate::client::{AccountSession, LoginResponse, SekaiClient};
use crate::config::{AssetUpdaterInfo, GitConfig, MasterRemoteSourceConfig, ServerRegion};
use crate::error::AppError;
use crate::upstream::{
    GameStreamRequest, InternalApiResponse, LoginProbeRequest, LoginProbeResponse,
};

const ASSET_UPDATER_CONFLICT_RETRY_DELAY_SECS: u64 = 60;
const ASSET_UPDATER_MAX_CONFLICT_RETRIES: u8 = 10;
const CP_MASTER_SPLIT_MAX_RETRIES: u8 = 3;
const CP_MASTER_SPLIT_RETRY_DELAY_SECS: u64 = 2;
const CP_MASTER_SPLIT_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Serialize, Deserialize)]
struct AssetUpdaterPayload {
    region: String,
    asset_version: String,
    asset_hash: String,
    dry_run: bool,
}

/// A peer node lending its game accounts to this node's master production.
/// The peer only ever executes a login (returning version metadata) or relays
/// an authenticated GET as an untouched encrypted byte stream; every
/// memory-heavy step (decrypt, decode, unpack, ingest) runs on this node.
pub struct RemoteMasterSource {
    region: ServerRegion,
    base_url: String,
    token: String,
    http: reqwest::Client,
}

impl RemoteMasterSource {
    pub fn new(
        region: ServerRegion,
        config: &MasterRemoteSourceConfig,
        http: reqwest::Client,
    ) -> Self {
        Self {
            region,
            base_url: config.url.trim_end_matches('/').to_string(),
            token: config.token.clone(),
            http,
        }
    }

    /// Login on the account node; returns the version metadata as a
    /// LoginResponse (session token intentionally absent).
    async fn probe(&self) -> Result<LoginResponse, AppError> {
        let resp = self
            .http
            .post(format!("{}/internal/login-probe", self.base_url))
            .bearer_auth(&self.token)
            .json(&LoginProbeRequest {
                server: self.region.as_str().to_string(),
            })
            .send()
            .await
            .map_err(|e| AppError::NetworkError(format!("login probe: {}", e)))?;
        if !resp.status().is_success() {
            return Err(AppError::NetworkError(format!(
                "login probe returned {}",
                resp.status()
            )));
        }
        let probe: LoginProbeResponse = resp
            .json()
            .await
            .map_err(|e| AppError::NetworkError(format!("login probe: {}", e)))?;
        if !probe.ok {
            return Err(AppError::from_kind(
                probe.kind.as_deref().unwrap_or(""),
                None,
                probe.message.unwrap_or_default(),
            ));
        }
        Ok(LoginResponse {
            session_token: String::new(),
            data_version: probe.data_version,
            asset_version: probe.asset_version,
            asset_hash: probe.asset_hash,
            suite_master_split_path: probe.suite_master_split_path,
            cdn_version: probe.cdn_version,
            user_registration: None,
        })
    }

    /// Fetch one CP master split as the raw encrypted bytes relayed by the
    /// account node. The full split buffers here (the decoding node), never on
    /// the relay.
    async fn fetch_split_bytes(&self, api_path: &str) -> Result<Vec<u8>, AppError> {
        let resp = self
            .http
            .post(format!("{}/internal/game-stream", self.base_url))
            .bearer_auth(&self.token)
            .json(&GameStreamRequest {
                server: self.region.as_str().to_string(),
                path: api_path.to_string(),
            })
            .send()
            .await
            .map_err(|e| AppError::NetworkError(format!("game stream: {}", e)))?;
        let status = resp.status();
        let is_json = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/json"));
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::NetworkError(format!("game stream: {}", e)))?;
        if !status.is_success() {
            return Err(AppError::NetworkError(format!(
                "game stream returned {}",
                status
            )));
        }
        if is_json {
            let envelope: InternalApiResponse = serde_json::from_slice(&bytes)
                .map_err(|e| AppError::NetworkError(format!("game stream envelope: {}", e)))?;
            return Err(AppError::from_kind(
                envelope.kind.as_deref().unwrap_or(""),
                envelope.status,
                envelope.message.unwrap_or_default(),
            ));
        }
        Ok(bytes.to_vec())
    }
}

pub struct MasterUpdater {
    pub region: ServerRegion,
    pub client: Arc<SekaiClient>,
    /// When set, master production borrows a peer's accounts (login probe +
    /// relayed split bytes) instead of this node's own sessions.
    remote_source: Option<RemoteMasterSource>,
    pub git_helper: Option<GitHelper>,
    pub asset_updater_servers: Vec<AssetUpdaterInfo>,
    http_client: reqwest::Client,
    update_lock: tokio::sync::Mutex<()>,
    /// Serializes version-file writes with the AppHashUpdater for the same region
    /// so their read-modify-writes do not clobber each other's fields.
    version_lock: Arc<tokio::sync::Mutex<()>>,
    db: Option<sea_orm::DatabaseConnection>,
    /// Set when the last DB ingest failed. The next cron tick retries the ingest
    /// even if the master version is unchanged, so a transient DB failure does not
    /// leave the DB out of sync until the next upstream version bump.
    ingest_failed: std::sync::atomic::AtomicBool,
}

impl MasterUpdater {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        region: ServerRegion,
        client: Arc<SekaiClient>,
        git_config: Option<&GitConfig>,
        proxy: Option<String>,
        asset_updater_servers: Vec<AssetUpdaterInfo>,
        db: Option<sea_orm::DatabaseConnection>,
        version_lock: Arc<tokio::sync::Mutex<()>>,
        remote_source: Option<RemoteMasterSource>,
    ) -> Self {
        let git_helper = git_config
            .filter(|c| c.enabled)
            .map(|c| GitHelper::new(c, proxy));

        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self {
            region,
            client,
            remote_source,
            git_helper,
            asset_updater_servers,
            http_client,
            update_lock: tokio::sync::Mutex::new(()),
            version_lock,
            db,
            ingest_failed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub async fn check_update(&self) {
        let _lock = match self.update_lock.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                info!(
                    "{} Master update check already in progress, skipping...",
                    self.region.as_str().to_uppercase()
                );
                return;
            }
        };
        info!(
            "{} Checking for master data updates...",
            self.region.as_str().to_uppercase()
        );
        let current_version = match self.client.version_helper.load().await {
            Ok(v) => v,
            Err(e) => {
                error!(
                    "{} Failed to load version file: {}",
                    self.region.as_str().to_uppercase(),
                    e
                );
                return;
            }
        };
        let Some((session, login_response)) = self.load_login_context().await else {
            return;
        };
        let (need_master_update, need_asset_update, need_version_save) =
            self.required_updates(&login_response, &current_version);
        if need_asset_update {
            self.call_all_asset_updaters(&login_response.asset_version, &login_response.asset_hash)
                .await;
        }
        let retry_ingest = !need_master_update
            && self
                .ingest_failed
                .load(std::sync::atomic::Ordering::Relaxed);
        if retry_ingest {
            warn!(
                "{} Previous DB ingest failed; re-running master update for the current version...",
                self.region.as_str().to_uppercase()
            );
        }
        if need_master_update || retry_ingest {
            self.log_master_update(&login_response, need_master_update);
            if let Err(e) = self
                .update_master_data(session.as_deref(), &login_response)
                .await
            {
                error!(
                    "{} Failed to update master data: {}",
                    self.region.as_str().to_uppercase(),
                    e
                );
                return;
            }
        }
        if need_version_save {
            if let Err(e) = self
                .save_publish_and_notify(current_version, &login_response, need_master_update)
                .await
            {
                error!(
                    "{} Failed to save version file: {}",
                    self.region.as_str().to_uppercase(),
                    e
                );
                return;
            }
        }
        info!(
            "{} Master data check complete",
            self.region.as_str().to_uppercase()
        );
    }

    async fn load_login_context(&self) -> Option<(Option<Arc<AccountSession>>, LoginResponse)> {
        if let Some(remote) = &self.remote_source {
            return match remote.probe().await {
                Ok(response) => Some((None, response)),
                Err(e) => {
                    error!(
                        "{} Remote login probe failed: {}",
                        self.region.as_str().to_uppercase(),
                        e
                    );
                    None
                }
            };
        }
        let session = self.client.get_session().or_else(|| {
            error!(
                "{} No session available",
                self.region.as_str().to_uppercase()
            );
            None
        })?;
        self.login_with_version_refresh(&session)
            .await
            .map(|response| (Some(session), response))
    }

    async fn login_with_version_refresh(&self, session: &AccountSession) -> Option<LoginResponse> {
        match self.client.login(session).await {
            Ok(response) => Some(response),
            Err(AppError::UpgradeRequired) => {
                warn!(
                    "{} Server upgrade required during check_update login, refreshing version...",
                    self.region.as_str().to_uppercase()
                );
                if let Err(e) = self.client.refresh_version().await {
                    error!(
                        "{} Failed to refresh version: {}",
                        self.region.as_str().to_uppercase(),
                        e
                    );
                    return None;
                }
                self.client
                    .login(session)
                    .await
                    .map_err(|e| {
                        error!(
                            "{} Failed to login after version refresh: {}",
                            self.region.as_str().to_uppercase(),
                            e
                        );
                    })
                    .ok()
            }
            Err(e) => {
                error!(
                    "{} Failed to login: {}",
                    self.region.as_str().to_uppercase(),
                    e
                );
                None
            }
        }
    }

    fn required_updates(&self, login: &LoginResponse, current: &VersionInfo) -> (bool, bool, bool) {
        if self.region.is_cp_server() {
            let (master, asset) = self.check_cp_versions(login, current);
            return (master, asset, master || asset);
        }
        if login.data_version.trim().is_empty()
            || login.asset_version.trim().is_empty()
            || login.cdn_version <= 0
        {
            warn!(
                "{} Ignoring incomplete Nuverse version metadata: dataVersion_present={}, assetVersion_present={}, cdnVersion={}",
                self.region.as_str().to_uppercase(),
                !login.data_version.trim().is_empty(),
                !login.asset_version.trim().is_empty(),
                login.cdn_version
            );
        }
        check_nuverse_versions(login, current)
    }

    fn log_master_update(&self, login: &LoginResponse, is_new_version: bool) {
        if !is_new_version {
            return;
        }
        let region = self.region.as_str().to_uppercase();
        if self.region.is_cp_server() {
            info!("{} New master data version: {}", region, login.data_version);
        } else {
            info!(
                "{} New master data version (cdnVersion: {})",
                region, login.cdn_version
            );
        }
    }

    async fn save_publish_and_notify(
        &self,
        current: VersionInfo,
        login: &LoginResponse,
        notify_peers: bool,
    ) -> Result<(), AppError> {
        let new_version = VersionInfo {
            app_version: current.app_version,
            app_hash: current.app_hash,
            data_version: login.data_version.clone(),
            asset_version: login.asset_version.clone(),
            asset_hash: login.asset_hash.clone(),
            cdn_version: login.cdn_version,
        };
        let merged = self.save_version(&new_version).await?;
        let data_version = merged.data_version.clone();
        self.client.version_helper.update(merged);
        self.push_master_changes(&data_version).await;
        if notify_peers {
            self.notify_sync_peers(&data_version).await;
        }
        Ok(())
    }

    async fn push_master_changes(&self, data_version: &str) {
        let Some(git_helper) = self.git_helper.clone() else {
            return;
        };
        let master_dir = self.client.config.master_dir.clone();
        let data_version = data_version.to_string();
        let region = self.region.as_str().to_uppercase();
        let push = tokio::task::spawn_blocking(move || {
            git_helper.push_changes(&master_dir, &data_version)
        })
        .await;
        match push {
            Ok(Ok(true)) => info!("{} Git pushed changes successfully", region),
            Ok(Ok(false)) => {}
            Ok(Err(e)) => error!("{} Git push failed: {}", region, e),
            Err(e) => error!("{} Git push task failed: {}", region, e),
        }
    }

    fn check_cp_versions(
        &self,
        login: &crate::client::LoginResponse,
        current: &VersionInfo,
    ) -> (bool, bool) {
        // A version string that fails to parse must not silently freeze updates
        // forever: log it, and treat "different and unparseable" as an update.
        let compare_or_differ = |what: &str, new: &str, cur: &str| -> bool {
            if new.trim().is_empty() {
                warn!(
                    "{} Ignoring empty {} version from login",
                    self.region.as_str().to_uppercase(),
                    what
                );
                return false;
            }
            match compare_version(new, cur) {
                Ok(newer) => newer,
                Err(e) => {
                    warn!(
                        "{} Failed to compare {} versions ({:?} vs {:?}): {}; \
treating difference as an update",
                        self.region.as_str().to_uppercase(),
                        what,
                        new,
                        cur,
                        e
                    );
                    new != cur
                }
            }
        };
        let need_master = compare_or_differ("data", &login.data_version, &current.data_version);
        let need_asset = compare_or_differ("asset", &login.asset_version, &current.asset_version);

        (need_master, need_asset)
    }

    async fn call_all_asset_updaters(&self, asset_version: &str, asset_hash: &str) {
        if self.asset_updater_servers.is_empty() {
            return;
        }
        info!(
            "{} Calling {} asset updater server(s)...",
            self.region.as_str().to_uppercase(),
            self.asset_updater_servers.len()
        );
        let payload = AssetUpdaterPayload {
            region: self.region.as_str().to_string(),
            asset_version: asset_version.to_string(),
            asset_hash: asset_hash.to_string(),
            dry_run: false,
        };
        let futures: Vec<_> = self
            .asset_updater_servers
            .iter()
            .map(|info| self.call_asset_updater(info, &payload))
            .collect();
        futures::future::join_all(futures).await;
        info!(
            "{} Asset updater calls complete",
            self.region.as_str().to_uppercase()
        );
    }

    async fn call_asset_updater(&self, info: &AssetUpdaterInfo, payload: &AssetUpdaterPayload) {
        let endpoint = &info.url;
        let mut conflict_retries = 0u8;
        loop {
            let mut req = self
                .http_client
                .post(endpoint)
                .header("Content-Type", "application/json")
                .header(
                    "User-Agent",
                    format!("Haruki-Sekai-API/{}", env!("CARGO_PKG_VERSION")),
                );
            if !info.authorization.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", info.authorization));
            }
            let result = req.json(payload).send().await;
            match result {
                Ok(resp) => {
                    if resp.status().as_u16() == 409 {
                        if conflict_retries >= ASSET_UPDATER_MAX_CONFLICT_RETRIES {
                            warn!(
                                "{} Asset updater call to {} kept returning 409; giving up after {} retries",
                                self.region.as_str().to_uppercase(),
                                endpoint,
                                ASSET_UPDATER_MAX_CONFLICT_RETRIES
                            );
                            return;
                        }
                        conflict_retries += 1;
                        warn!(
                            "{} Asset updater call to {} returned 409; retry {}/{} in {}s",
                            self.region.as_str().to_uppercase(),
                            endpoint,
                            conflict_retries,
                            ASSET_UPDATER_MAX_CONFLICT_RETRIES,
                            ASSET_UPDATER_CONFLICT_RETRY_DELAY_SECS
                        );
                        tokio::time::sleep(Duration::from_secs(
                            ASSET_UPDATER_CONFLICT_RETRY_DELAY_SECS,
                        ))
                        .await;
                        continue;
                    }
                    if !resp.status().is_success() {
                        warn!(
                            "{} Asset updater call to {} returned status {}",
                            self.region.as_str().to_uppercase(),
                            endpoint,
                            resp.status()
                        );
                    }
                    return;
                }
                Err(e) => {
                    warn!(
                        "{} Asset updater call to {} failed: {}",
                        self.region.as_str().to_uppercase(),
                        endpoint,
                        e
                    );
                    return;
                }
            }
        }
    }

    async fn update_master_data(
        &self,
        session: Option<&crate::client::AccountSession>,
        login: &crate::client::LoginResponse,
    ) -> Result<(), crate::error::AppError> {
        info!(
            "{} Downloading master data...",
            self.region.as_str().to_uppercase()
        );
        let master_dir = &self.client.config.master_dir;
        tokio::fs::create_dir_all(master_dir).await?;
        self.download_master_files(session, login, master_dir)
            .await?;
        self.ingest_master_files(master_dir).await;
        info!(
            "{} Master data updated",
            self.region.as_str().to_uppercase()
        );
        Ok(())
    }

    async fn download_master_files(
        &self,
        session: Option<&AccountSession>,
        login: &LoginResponse,
        master_dir: &str,
    ) -> Result<(), AppError> {
        if self.region.is_cp_server() {
            let paths: Vec<String> = login
                .suite_master_split_path
                .iter()
                .map(|p| {
                    if p.starts_with('/') {
                        p.clone()
                    } else {
                        format!("/{}", p)
                    }
                })
                .collect();
            for api_path in paths {
                let data = if let Some(ref remote) = self.remote_source {
                    self.download_cp_master_split_remote(remote, &api_path)
                        .await?
                } else {
                    let session = session.ok_or_else(|| {
                        AppError::Internal("no session for local master download".to_string())
                    })?;
                    self.download_cp_master_split(session, &api_path).await?
                };
                self.save_master_files(&data, master_dir).await?;
            }
        } else {
            let url = format!(
                "{}/master-data-{}.info",
                self.client.config.nuverse_master_data_url, login.cdn_version
            );
            let restored = self.download_nuverse_master(&url).await?;
            self.save_master_files(&restored, master_dir).await?;
        }
        Ok(())
    }

    async fn ingest_master_files(&self, master_dir: &str) {
        let Some(db) = self.db.clone() else {
            return;
        };
        let region = self.region.as_str().to_uppercase();
        info!(
            "{} Starting database ingestion for new master data...",
            region
        );
        let ingest_ok = match crate::ingest_engine::IngestionEngine::new(db).await {
            Ok(engine) => self.run_ingestion(&engine, master_dir, &region).await,
            Err(e) => {
                error!(
                    "{} Failed to initialize ingestion engine (skipping DB ingest; will retry on the next cron tick): {e:#}",
                    region
                );
                false
            }
        };
        self.ingest_failed
            .store(!ingest_ok, std::sync::atomic::Ordering::Relaxed);
    }

    async fn run_ingestion(
        &self,
        engine: &crate::ingest_engine::IngestionEngine,
        master_dir: &str,
        region_upper: &str,
    ) -> bool {
        let region = self.region.as_str().to_lowercase();
        match engine.ingest_master_data(master_dir, &region).await {
            Ok(()) => {
                info!(
                    "{} Master Data successfully ingested into database",
                    region_upper
                );
                true
            }
            Err(e) => {
                error!(
                    "{} Master Data DB ingestion failed (files saved; git mirror and version unaffected; will retry on the next cron tick): {e:#}",
                    region_upper
                );
                false
            }
        }
    }

    /// Download and restore the Nuverse master blob with a bounded retry, mirroring
    /// the CP split download. Checks the HTTP status before reading the body so a
    /// CDN 404/5xx surfaces as a clear error instead of an opaque decrypt failure.
    async fn download_nuverse_master(
        &self,
        url: &str,
    ) -> Result<IndexMap<String, serde_json::Value>, crate::error::AppError> {
        use crate::error::AppError;
        let region = self.region.as_str().to_uppercase();
        let http_client = &self.client.http_client;
        let mut last_err = AppError::NetworkError("Nuverse master download failed".to_string());
        for attempt in 1..=CP_MASTER_SPLIT_MAX_RETRIES {
            match http_client.get(url).send().await {
                Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                    Ok(body) => match self.client.restore_nuverse_master(&body) {
                        Ok(restored) => return Ok(restored),
                        Err(e) => last_err = e,
                    },
                    Err(e) => last_err = AppError::NetworkError(e.to_string()),
                },
                Ok(resp) => {
                    last_err = AppError::NetworkError(format!(
                        "Nuverse master download returned HTTP {} for {}",
                        resp.status(),
                        url
                    ));
                }
                Err(e) => last_err = AppError::NetworkError(e.to_string()),
            }
            if attempt < CP_MASTER_SPLIT_MAX_RETRIES {
                warn!(
                    "{} Nuverse master download attempt {}/{} failed: {}; retrying...",
                    region, attempt, CP_MASTER_SPLIT_MAX_RETRIES, last_err
                );
                tokio::time::sleep(Duration::from_secs(CP_MASTER_SPLIT_RETRY_DELAY_SECS)).await;
            }
        }
        Err(last_err)
    }

    /// Fetch and decode one CP master split via the remote account node: the
    /// relay hands back the untouched encrypted bytes and decoding happens
    /// here with this node's own cryptor (same region keys).
    async fn download_cp_master_split_remote(
        &self,
        remote: &RemoteMasterSource,
        api_path: &str,
    ) -> Result<IndexMap<String, JsonValue>, crate::error::AppError> {
        let mut last_err =
            AppError::NetworkError("remote master split download failed".to_string());
        for attempt in 1..=CP_MASTER_SPLIT_MAX_RETRIES {
            match remote.fetch_split_bytes(api_path).await {
                Ok(bytes) => match self.client.cryptor.unpack_ordered(&bytes) {
                    Ok(map) => return Ok(map),
                    Err(e) => last_err = e,
                },
                Err(e) => last_err = e,
            }
            if attempt < CP_MASTER_SPLIT_MAX_RETRIES {
                warn!(
                    "{} Remote master split {} attempt {}/{} failed: {}; retrying...",
                    self.region.as_str().to_uppercase(),
                    api_path,
                    attempt,
                    CP_MASTER_SPLIT_MAX_RETRIES,
                    last_err
                );
                tokio::time::sleep(Duration::from_secs(CP_MASTER_SPLIT_RETRY_DELAY_SECS)).await;
            }
        }
        Err(last_err)
    }

    async fn download_cp_master_split(
        &self,
        session: &crate::client::AccountSession,
        api_path: &str,
    ) -> Result<IndexMap<String, JsonValue>, crate::error::AppError> {
        for attempt in 1..=CP_MASTER_SPLIT_MAX_RETRIES {
            let resp = match self
                .client
                .get_with_timeout(
                    session,
                    api_path,
                    None,
                    Duration::from_secs(CP_MASTER_SPLIT_TIMEOUT_SECS),
                )
                .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    if matches!(e, crate::error::AppError::NetworkError(_))
                        && attempt < CP_MASTER_SPLIT_MAX_RETRIES
                    {
                        warn!(
                            "{} Failed to request master split {} (attempt {}/{}): {}; retrying in {}s",
                            self.region.as_str().to_uppercase(),
                            api_path,
                            attempt,
                            CP_MASTER_SPLIT_MAX_RETRIES,
                            e,
                            CP_MASTER_SPLIT_RETRY_DELAY_SECS
                        );
                        tokio::time::sleep(Duration::from_secs(CP_MASTER_SPLIT_RETRY_DELAY_SECS))
                            .await;
                        continue;
                    }
                    warn!(
                        "{} Failed to request master split {}: {}",
                        self.region.as_str().to_uppercase(),
                        api_path,
                        e
                    );
                    return Err(e);
                }
            };

            let status = resp.status();
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("")
                .to_string();
            let content_encoding = resp
                .headers()
                .get("content-encoding")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("")
                .to_string();

            match self.client.handle_response_ordered(resp).await {
                Ok((data, _status)) => return Ok(data),
                Err(e) => {
                    if matches!(e, crate::error::AppError::NetworkError(_))
                        && attempt < CP_MASTER_SPLIT_MAX_RETRIES
                    {
                        warn!(
                            "{} Failed to read master split {} (attempt {}/{}; status={}, content-type={}, content-encoding={}): {}; retrying in {}s",
                            self.region.as_str().to_uppercase(),
                            api_path,
                            attempt,
                            CP_MASTER_SPLIT_MAX_RETRIES,
                            status,
                            content_type,
                            content_encoding,
                            e,
                            CP_MASTER_SPLIT_RETRY_DELAY_SECS
                        );
                        tokio::time::sleep(Duration::from_secs(CP_MASTER_SPLIT_RETRY_DELAY_SECS))
                            .await;
                        continue;
                    }
                    warn!(
                        "{} Failed to process master split {} (status={}, content-type={}, content-encoding={}): {}",
                        self.region.as_str().to_uppercase(),
                        api_path,
                        status,
                        content_type,
                        content_encoding,
                        e
                    );
                    return Err(e);
                }
            }
        }

        Err(crate::error::AppError::NetworkError(format!(
            "Failed to download master split {} after {} retries",
            api_path, CP_MASTER_SPLIT_MAX_RETRIES
        )))
    }

    async fn save_master_files(
        &self,
        data: &IndexMap<String, JsonValue>,
        master_dir: &str,
    ) -> Result<(), crate::error::AppError> {
        let total_keys = data.len();
        let mut success_count = 0;
        let mut fail_count = 0;
        for (key, value) in data {
            if !is_safe_path_component(key) {
                warn!(
                    "{} Skipping master key {:?}: not a safe filename",
                    self.region.as_str().to_uppercase(),
                    key
                );
                fail_count += 1;
                continue;
            }
            let file_path = Path::new(master_dir).join(format!("{}.json", key));
            let json = match sonic_rs::to_string_pretty(value) {
                Ok(j) => j,
                Err(e) => {
                    warn!(
                        "{} Failed to serialize {}: {}",
                        self.region.as_str().to_uppercase(),
                        key,
                        e
                    );
                    fail_count += 1;
                    continue;
                }
            };
            match crate::client::helper::write_file_atomic(&file_path, json.as_bytes()).await {
                Ok(_) => success_count += 1,
                Err(e) => {
                    warn!(
                        "{} Failed to write {}: {}",
                        self.region.as_str().to_uppercase(),
                        key,
                        e
                    );
                    fail_count += 1;
                }
            }
        }
        info!(
            "{} Wrote {}/{} master files ({} failed)",
            self.region.as_str().to_uppercase(),
            success_count,
            total_keys,
            fail_count
        );
        if fail_count > 0 {
            // A torn write set must not be recorded as a completed update: bail so
            // the caller neither saves the version nor pushes the git mirror, and
            // the next cron tick re-downloads.
            return Err(crate::error::AppError::IoError(format!(
                "{} of {} master file writes failed",
                fail_count, total_keys
            )));
        }
        Ok(())
    }

    /// Persist the master/asset version fields, preserving whatever
    /// `appVersion`/`appHash` are currently on disk: those belong to the
    /// AppHashUpdater, and our in-memory copy may be minutes stale (snapshotted
    /// before a long download), so overwriting them here would revert a
    /// concurrent app-hash update. Returns the merged state as written.
    async fn save_version(
        &self,
        version: &VersionInfo,
    ) -> Result<VersionInfo, crate::error::AppError> {
        // Serialize with the AppHashUpdater so neither clobbers the other's fields.
        let _guard = self.version_lock.lock().await;
        persist_version_file(self.region, &self.client.config.version_path, version).await
    }

    /// Webhook peer nodes that pull this region's master data from us, so they
    /// sync immediately instead of waiting for their fallback poll. Best-effort:
    /// failures are logged and covered by the peers' polling.
    async fn notify_sync_peers(&self, data_version: &str) {
        let peers = &self.client.config.master_sync.notify;
        if peers.is_empty() {
            return;
        }
        let payload = serde_json::json!({
            "server": self.region.as_str(),
            "dataVersion": data_version,
        });
        for peer in peers {
            let endpoint = format!("{}/internal/master-updated", peer.url.trim_end_matches('/'));
            let mut req = self.http_client.post(&endpoint).json(&payload);
            if !peer.token.is_empty() {
                req = req.bearer_auth(&peer.token);
            }
            match req.send().await {
                Ok(resp) if resp.status().is_success() => info!(
                    "{} Notified sync peer {}",
                    self.region.as_str().to_uppercase(),
                    peer.url
                ),
                Ok(resp) => warn!(
                    "{} Sync peer {} returned {}",
                    self.region.as_str().to_uppercase(),
                    peer.url,
                    resp.status()
                ),
                Err(e) => warn!(
                    "{} Failed to notify sync peer {}: {}",
                    self.region.as_str().to_uppercase(),
                    peer.url,
                    e
                ),
            }
        }
    }
}

/// Merge `version` into the on-disk version file at `path` and write it
/// atomically, plus a `<dataVersion>.json` snapshot next to it. Callers must
/// hold the region's version lock. Shared by the master updater (own download)
/// and the master syncer (pulled from a peer node).
pub(crate) async fn persist_version_file(
    region: ServerRegion,
    path: &str,
    version: &VersionInfo,
) -> Result<VersionInfo, crate::error::AppError> {
    if let Some(parent) = Path::new(path).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut existing: serde_json::Map<String, serde_json::Value> = if Path::new(path).exists() {
        let data = tokio::fs::read(path).await?;
        sonic_rs::from_slice(&data).unwrap_or_default()
    } else {
        serde_json::Map::new()
    };
    let merged_version = merge_version_state(region, &mut existing, version);
    let json = sonic_rs::to_string_pretty(&existing)
        .map_err(|e| crate::error::AppError::ParseError(e.to_string()))?;
    crate::client::helper::write_file_atomic(Path::new(path), json.as_bytes()).await?;
    // The versioned snapshot filename embeds a server-supplied string; refuse
    // anything that could escape the version directory.
    if is_safe_path_component(&merged_version.data_version) {
        let dir = Path::new(path).parent().unwrap_or(Path::new("."));
        let versioned_path = dir.join(format!("{}.json", merged_version.data_version));
        crate::client::helper::write_file_atomic(&versioned_path, json.as_bytes()).await?;
    } else {
        warn!(
            "{} Skipping versioned snapshot: dataVersion {:?} is not a safe filename",
            region.as_str().to_uppercase(),
            merged_version.data_version
        );
    }
    Ok(merged_version)
}

fn check_nuverse_versions(login: &LoginResponse, current: &VersionInfo) -> (bool, bool, bool) {
    let need_cdn_update = login.cdn_version > current.cdn_version;
    let need_asset_update = need_cdn_update && !login.asset_version.trim().is_empty();
    let need_data_version_save =
        !login.data_version.trim().is_empty() && login.data_version != current.data_version;
    let need_version_save = need_cdn_update || need_data_version_save;
    (need_cdn_update, need_asset_update, need_version_save)
}

fn non_empty_or_existing(
    incoming: &str,
    existing: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> String {
    if !incoming.trim().is_empty() {
        return incoming.to_string();
    }
    existing
        .get(key)
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_default()
        .to_string()
}

fn existing_or_non_empty(
    existing: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    fallback: &str,
) -> String {
    existing
        .get(key)
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            if fallback.trim().is_empty() {
                ""
            } else {
                fallback
            }
        })
        .to_string()
}

fn merge_version_state(
    region: ServerRegion,
    existing: &mut serde_json::Map<String, serde_json::Value>,
    incoming: &VersionInfo,
) -> VersionInfo {
    // AppHashUpdater owns these two fields, so the on-disk values win unless
    // they are empty. appVersion is still normalized for Nuverse regions.
    let app_version = effective_app_version(
        region,
        &existing_or_non_empty(existing, "appVersion", &incoming.app_version),
    );
    let app_hash = existing_or_non_empty(existing, "appHash", &incoming.app_hash);
    let data_version = non_empty_or_existing(&incoming.data_version, existing, "dataVersion");
    let asset_version = non_empty_or_existing(&incoming.asset_version, existing, "assetVersion");
    // Nuverse legitimately returns an empty assetHash. CP servers do not, so an
    // empty CP value is treated as missing and preserves the previous hash.
    let asset_hash = if region.is_cp_server() {
        non_empty_or_existing(&incoming.asset_hash, existing, "assetHash")
    } else {
        incoming.asset_hash.clone()
    };
    let cdn_version = if incoming.cdn_version > 0 {
        incoming.cdn_version
    } else {
        existing
            .get("cdnVersion")
            .and_then(|value| value.as_i64())
            .and_then(|value| i32::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or_default()
    };

    existing.insert(
        "appVersion".to_string(),
        serde_json::Value::String(app_version.clone()),
    );
    existing.insert(
        "appHash".to_string(),
        serde_json::Value::String(app_hash.clone()),
    );
    existing.insert(
        "dataVersion".to_string(),
        serde_json::Value::String(data_version.clone()),
    );
    existing.insert(
        "assetVersion".to_string(),
        serde_json::Value::String(asset_version.clone()),
    );
    existing.insert(
        "assetHash".to_string(),
        serde_json::Value::String(asset_hash.clone()),
    );
    existing.insert(
        "cdnVersion".to_string(),
        serde_json::Value::Number(cdn_version.into()),
    );

    VersionInfo {
        app_version,
        app_hash,
        data_version,
        asset_version,
        asset_hash,
        cdn_version,
    }
}

/// True if `s` can be safely embedded in a filename within the intended
/// directory: non-empty, no path separators, and not a dot-relative component.
pub(crate) fn is_safe_path_component(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.contains('/')
        && !s.contains('\\')
        && !s.contains('\0')
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::extract::State;
    use axum::http::Response;
    use axum::Router;

    use super::*;

    const KEY: &str = "00112233445566778899aabbccddeeff";
    const IV: &str = "ffeeddccbbaa99887766554433221100";

    #[derive(Clone)]
    struct Reply {
        status: u16,
        content_type: &'static str,
        body: Vec<u8>,
    }

    async fn handler(State(reply): State<Reply>) -> Response<Body> {
        Response::builder()
            .status(reply.status)
            .header("content-type", reply.content_type)
            .body(Body::from(reply.body))
            .unwrap()
    }

    async fn spawn_server(reply: Reply) -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().fallback(handler).with_state(reply);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{address}"), task)
    }

    #[derive(Clone)]
    struct UpdateReply {
        probe: Vec<u8>,
        master: Vec<u8>,
    }

    async fn update_handler(
        State(reply): State<UpdateReply>,
        uri: axum::http::Uri,
    ) -> Response<Body> {
        if uri.path().ends_with("/internal/login-probe") {
            Response::builder()
                .header("content-type", "application/json")
                .body(Body::from(reply.probe))
                .unwrap()
        } else {
            Response::builder()
                .header("content-type", "application/octet-stream")
                .body(Body::from(reply.master))
                .unwrap()
        }
    }

    async fn spawn_update_server(reply: UpdateReply) -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().fallback(update_handler).with_state(reply);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{address}"), task)
    }

    fn temp_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("haruki_master_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    async fn make_updater(
        region: ServerRegion,
        api_url: &str,
        root: &std::path::Path,
        asset_updaters: Vec<AssetUpdaterInfo>,
    ) -> MasterUpdater {
        let mut config: crate::config::ServerConfig = serde_yaml::from_str("{}").unwrap();
        config.api_url = api_url.to_string();
        config.nuverse_master_data_url = api_url.to_string();
        config.aes_key_hex = KEY.to_string();
        config.aes_iv_hex = IV.to_string();
        config.master_dir = root.join("master").to_string_lossy().into_owned();
        config.account_dir = root.join("accounts").to_string_lossy().into_owned();
        config.version_path = root.join("version.json").to_string_lossy().into_owned();
        std::fs::create_dir_all(&config.account_dir).unwrap();
        std::fs::write(
            &config.version_path,
            sonic_rs::to_string(&version_info()).unwrap(),
        )
        .unwrap();
        let client = SekaiClient::new(
            region,
            config,
            None,
            None,
            SekaiClient::build_http_client(None).unwrap(),
            None,
        )
        .await
        .unwrap();
        MasterUpdater::new(
            region,
            Arc::new(client),
            None,
            None,
            asset_updaters,
            None,
            Arc::new(tokio::sync::Mutex::new(())),
            None,
        )
    }

    fn version_info() -> VersionInfo {
        VersionInfo {
            app_version: "6.0.2".to_string(),
            app_hash: "app-hash".to_string(),
            data_version: "6.0.0.48".to_string(),
            asset_version: "6.0.0.1".to_string(),
            asset_hash: "stale-asset-hash".to_string(),
            cdn_version: 159,
        }
    }

    fn login_response() -> LoginResponse {
        LoginResponse {
            session_token: String::new(),
            data_version: String::new(),
            asset_version: String::new(),
            asset_hash: String::new(),
            suite_master_split_path: Vec::new(),
            cdn_version: 0,
            user_registration: None,
        }
    }

    #[test]
    fn empty_nuverse_data_version_does_not_trigger_save() {
        assert_eq!(
            check_nuverse_versions(&login_response(), &version_info()),
            (false, false, false)
        );
    }

    #[test]
    fn nuverse_cdn_update_does_not_send_empty_asset_version() {
        let mut login = login_response();
        login.cdn_version = 160;

        assert_eq!(
            check_nuverse_versions(&login, &version_info()),
            (true, false, true)
        );
    }

    #[test]
    fn nuverse_empty_fields_preserve_versions_but_allow_empty_asset_hash() {
        let mut existing = serde_json::json!({
            "appVersion": "6.0.2",
            "appHash": "app-hash",
            "dataVersion": "6.0.0.48",
            "assetVersion": "6.0.0.1",
            "assetHash": "stale-asset-hash",
            "cdnVersion": 159
        })
        .as_object()
        .unwrap()
        .clone();
        let incoming = login_response();
        let merged = merge_version_state(
            ServerRegion::Cn,
            &mut existing,
            &VersionInfo {
                app_version: String::new(),
                app_hash: String::new(),
                data_version: incoming.data_version,
                asset_version: incoming.asset_version,
                asset_hash: incoming.asset_hash,
                cdn_version: incoming.cdn_version,
            },
        );

        assert_eq!(merged.app_version, "6.0.0");
        assert_eq!(merged.app_hash, "app-hash");
        assert_eq!(merged.data_version, "6.0.0.48");
        assert_eq!(merged.asset_version, "6.0.0.1");
        assert_eq!(merged.asset_hash, "");
        assert_eq!(merged.cdn_version, 159);
        assert_eq!(existing["assetHash"], "");
    }

    #[test]
    fn cp_empty_asset_hash_preserves_existing_value() {
        let mut existing = serde_json::json!({"assetHash": "cp-asset-hash"})
            .as_object()
            .unwrap()
            .clone();
        let mut incoming = version_info();
        incoming.asset_hash.clear();

        let merged = merge_version_state(ServerRegion::Jp, &mut existing, &incoming);

        assert_eq!(merged.asset_hash, "cp-asset-hash");
    }

    #[test]
    fn validates_safe_components_and_merge_fallbacks() {
        assert!(is_safe_path_component("master_001"));
        for invalid in ["", ".", "..", "a/b", "a\\b", "a\0b"] {
            assert!(!is_safe_path_component(invalid));
        }

        let mut existing = serde_json::Map::new();
        existing.insert("appVersion".to_string(), JsonValue::String(String::new()));
        existing.insert("cdnVersion".to_string(), JsonValue::from(-1));
        let merged = merge_version_state(ServerRegion::Jp, &mut existing, &version_info());
        assert_eq!(merged.app_version, "6.0.2");
        assert_eq!(merged.cdn_version, 159);
        assert_eq!(
            non_empty_or_existing("new", &existing, "dataVersion"),
            "new"
        );
        assert_eq!(
            existing_or_non_empty(&existing, "missing", "fallback"),
            "fallback"
        );
    }

    #[tokio::test]
    async fn persists_versions_and_master_files_atomically() {
        let root = temp_dir();
        let updater = make_updater(ServerRegion::Jp, "http://127.0.0.1:1", &root, Vec::new()).await;
        let merged = updater.save_version(&version_info()).await.unwrap();
        assert_eq!(merged.data_version, "6.0.0.48");
        assert!(root.join("6.0.0.48.json").exists());

        let master_dir = root.join("master");
        std::fs::create_dir_all(&master_dir).unwrap();
        let data = IndexMap::from([
            ("cards".to_string(), serde_json::json!([{"id": 1}])),
            ("musics".to_string(), serde_json::json!([])),
        ]);
        updater
            .save_master_files(&data, master_dir.to_str().unwrap())
            .await
            .unwrap();
        assert!(master_dir.join("cards.json").exists());
        let unsafe_data = IndexMap::from([("../escape".to_string(), serde_json::json!([]))]);
        assert!(updater
            .save_master_files(&unsafe_data, master_dir.to_str().unwrap())
            .await
            .is_err());

        let mut no_split = login_response();
        no_split.data_version = "6.0.0.49".to_string();
        updater.update_master_data(None, &no_split).await.unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn computes_required_updates_and_publishes_version() {
        let root = temp_dir();
        let updater = make_updater(ServerRegion::Jp, "http://127.0.0.1:1", &root, Vec::new()).await;
        let mut login = login_response();
        login.data_version = "6.0.0.49".to_string();
        login.asset_version = "broken".to_string();
        login.asset_hash = "new-hash".to_string();
        assert_eq!(
            updater.required_updates(&login, &version_info()),
            (true, true, true)
        );
        updater.log_master_update(&login, false);
        updater.log_master_update(&login, true);
        updater
            .save_publish_and_notify(version_info(), &login, true)
            .await
            .unwrap();
        assert_eq!(updater.client.version_helper.get().data_version, "6.0.0.49");
        updater.push_master_changes("6.0.0.49").await;
        updater.notify_sync_peers("6.0.0.49").await;
        updater.call_all_asset_updaters("asset", "hash").await;

        let nuverse = make_updater(ServerRegion::Cn, "http://127.0.0.1:1", &root, Vec::new()).await;
        assert_eq!(
            nuverse.required_updates(&login_response(), &version_info()),
            (false, false, false)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn calls_asset_updater_for_success_failure_and_network_error() {
        for status in [204, 500] {
            let (url, server) = spawn_server(Reply {
                status,
                content_type: "application/json",
                body: Vec::new(),
            })
            .await;
            let root = temp_dir();
            let updater = make_updater(
                ServerRegion::Jp,
                "http://127.0.0.1:1",
                &root,
                vec![AssetUpdaterInfo {
                    url,
                    authorization: "token".to_string(),
                }],
            )
            .await;
            updater.call_all_asset_updaters("asset", "hash").await;
            server.abort();
            std::fs::remove_dir_all(root).unwrap();
        }

        let root = temp_dir();
        let updater = make_updater(
            ServerRegion::Jp,
            "http://127.0.0.1:1",
            &root,
            vec![AssetUpdaterInfo {
                url: "http://127.0.0.1:1".to_string(),
                authorization: String::new(),
            }],
        )
        .await;
        updater.call_all_asset_updaters("asset", "hash").await;
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn remote_source_probes_and_fetches_streams() {
        let probe = LoginProbeResponse {
            ok: true,
            kind: None,
            message: None,
            data_version: "data".to_string(),
            asset_version: "asset".to_string(),
            asset_hash: "hash".to_string(),
            cdn_version: 7,
            suite_master_split_path: vec!["split".to_string()],
        };
        let (url, server) = spawn_server(Reply {
            status: 200,
            content_type: "application/json",
            body: serde_json::to_vec(&probe).unwrap(),
        })
        .await;
        let source = RemoteMasterSource::new(
            ServerRegion::Jp,
            &MasterRemoteSourceConfig {
                url,
                token: "token".to_string(),
            },
            crate::upstream::build_internal_http_client().unwrap(),
        );
        assert_eq!(source.probe().await.unwrap().data_version, "data");
        server.abort();

        let (url, server) = spawn_server(Reply {
            status: 200,
            content_type: "application/octet-stream",
            body: vec![1, 2, 3],
        })
        .await;
        let source = RemoteMasterSource::new(
            ServerRegion::Jp,
            &MasterRemoteSourceConfig {
                url,
                token: String::new(),
            },
            crate::upstream::build_internal_http_client().unwrap(),
        );
        assert_eq!(
            source.fetch_split_bytes("/split").await.unwrap(),
            vec![1, 2, 3]
        );
        server.abort();

        let envelope = InternalApiResponse {
            ok: false,
            status: Some(403),
            data: None,
            kind: Some("session_error".to_string()),
            message: Some("expired".to_string()),
        };
        let (url, server) = spawn_server(Reply {
            status: 200,
            content_type: "application/json",
            body: serde_json::to_vec(&envelope).unwrap(),
        })
        .await;
        let source = RemoteMasterSource::new(
            ServerRegion::Jp,
            &MasterRemoteSourceConfig {
                url,
                token: String::new(),
            },
            crate::upstream::build_internal_http_client().unwrap(),
        );
        assert!(matches!(
            source.fetch_split_bytes("/split").await,
            Err(AppError::SessionError)
        ));
        server.abort();
    }

    #[tokio::test]
    async fn downloads_and_restores_nuverse_master() {
        let cryptor = crate::crypto::SekaiCryptor::from_hex(KEY, IV).unwrap();
        let body = cryptor
            .pack(&serde_json::json!({"cards": [{"id": 1}]}))
            .unwrap();
        let (url, server) = spawn_server(Reply {
            status: 200,
            content_type: "application/octet-stream",
            body,
        })
        .await;
        let root = temp_dir();
        let updater = make_updater(ServerRegion::Cn, &url, &root, Vec::new()).await;
        let restored = updater.download_nuverse_master(&url).await.unwrap();
        assert_eq!(restored["cards"][0]["id"], 1);
        let mut login = login_response();
        login.cdn_version = 1;
        updater.update_master_data(None, &login).await.unwrap();
        assert!(root.join("master/cards.json").exists());
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn downloads_cp_splits_locally_and_through_remote_source() {
        let cryptor = crate::crypto::SekaiCryptor::from_hex(KEY, IV).unwrap();
        let encrypted = cryptor
            .pack(&serde_json::json!({"events": [{"id": 1}]}))
            .unwrap();
        let (url, server) = spawn_server(Reply {
            status: 200,
            content_type: "application/octet-stream",
            body: encrypted.clone(),
        })
        .await;
        let root = temp_dir();
        let updater = make_updater(ServerRegion::Jp, &url, &root, Vec::new()).await;
        let session = AccountSession::new(crate::client::AccountType::CP(
            crate::client::SekaiAccountCP {
                user_id: "1".to_string(),
                device_id: "device".to_string(),
                credential: "credential".to_string(),
            },
        ));
        let data = updater
            .download_cp_master_split(&session, "/split")
            .await
            .unwrap();
        assert_eq!(data["events"][0]["id"], 1);
        let mut login = login_response();
        login.suite_master_split_path = vec!["split".to_string(), "/split".to_string()];
        std::fs::create_dir_all(root.join("master")).unwrap();
        updater
            .download_master_files(
                Some(&session),
                &login,
                root.join("master").to_str().unwrap(),
            )
            .await
            .unwrap();
        assert!(root.join("master/events.json").exists());
        server.abort();

        let (url, server) = spawn_server(Reply {
            status: 200,
            content_type: "application/octet-stream",
            body: encrypted,
        })
        .await;
        let remote = RemoteMasterSource::new(
            ServerRegion::Jp,
            &MasterRemoteSourceConfig {
                url,
                token: String::new(),
            },
            crate::upstream::build_internal_http_client().unwrap(),
        );
        let data = updater
            .download_cp_master_split_remote(&remote, "/split")
            .await
            .unwrap();
        assert_eq!(data["events"][0]["id"], 1);
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn notifies_successful_and_failing_sync_peers() {
        for status in [200, 500] {
            let (url, server) = spawn_server(Reply {
                status,
                content_type: "application/json",
                body: Vec::new(),
            })
            .await;
            let root = temp_dir();
            let mut updater =
                make_updater(ServerRegion::Jp, "http://127.0.0.1:1", &root, Vec::new()).await;
            Arc::get_mut(&mut updater.client)
                .unwrap()
                .config
                .master_sync
                .notify
                .push(crate::config::MasterSyncPeer {
                    url,
                    token: "secret".to_string(),
                });
            updater.notify_sync_peers("data").await;
            server.abort();
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn remote_source_reports_bad_status_and_error_probe() {
        for (status, body) in [
            (500, Vec::new()),
            (
                200,
                serde_json::to_vec(&LoginProbeResponse {
                    ok: false,
                    kind: Some("no_account".to_string()),
                    message: Some("none".to_string()),
                    data_version: String::new(),
                    asset_version: String::new(),
                    asset_hash: String::new(),
                    cdn_version: 0,
                    suite_master_split_path: Vec::new(),
                })
                .unwrap(),
            ),
        ] {
            let (url, server) = spawn_server(Reply {
                status,
                content_type: "application/json",
                body,
            })
            .await;
            let source = RemoteMasterSource::new(
                ServerRegion::Jp,
                &MasterRemoteSourceConfig {
                    url,
                    token: String::new(),
                },
                crate::upstream::build_internal_http_client().unwrap(),
            );
            assert!(source.probe().await.is_err());
            server.abort();
        }
    }

    #[tokio::test]
    async fn check_update_runs_complete_remote_nuverse_pipeline() {
        let cryptor = crate::crypto::SekaiCryptor::from_hex(KEY, IV).unwrap();
        let probe = LoginProbeResponse {
            ok: true,
            kind: None,
            message: None,
            data_version: "6.0.0.49".to_string(),
            asset_version: "6.0.0.2".to_string(),
            asset_hash: "new-asset".to_string(),
            cdn_version: 160,
            suite_master_split_path: Vec::new(),
        };
        let (url, server) = spawn_update_server(UpdateReply {
            probe: serde_json::to_vec(&probe).unwrap(),
            master: cryptor
                .pack(&serde_json::json!({"cards": [{"id": 1}]}))
                .unwrap(),
        })
        .await;
        let root = temp_dir();
        let mut updater = make_updater(ServerRegion::Cn, &url, &root, Vec::new()).await;
        updater.remote_source = Some(RemoteMasterSource::new(
            ServerRegion::Cn,
            &MasterRemoteSourceConfig {
                url,
                token: "token".to_string(),
            },
            crate::upstream::build_internal_http_client().unwrap(),
        ));
        updater.check_update().await;
        assert!(root.join("master/cards.json").exists());
        let persisted: VersionInfo =
            sonic_rs::from_slice(&std::fs::read(root.join("version.json")).unwrap()).unwrap();
        assert_eq!(persisted.data_version, "6.0.0.49");
        assert_eq!(persisted.cdn_version, 160);
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn check_update_returns_cleanly_without_session_or_version_file() {
        let root = temp_dir();
        let updater = make_updater(ServerRegion::Jp, "http://127.0.0.1:1", &root, Vec::new()).await;
        updater.check_update().await;
        std::fs::remove_file(&updater.client.config.version_path).unwrap();
        updater.check_update().await;
        std::fs::remove_dir_all(root).unwrap();
    }
}
