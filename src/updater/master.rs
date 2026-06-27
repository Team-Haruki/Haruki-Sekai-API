use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tracing::{error, info, warn};

use super::git::GitHelper;
use crate::client::helper::{compare_version, VersionInfo};
use crate::client::SekaiClient;
use crate::config::{AssetUpdaterInfo, GitConfig, ServerRegion};

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

pub struct MasterUpdater {
    pub region: ServerRegion,
    pub client: Arc<SekaiClient>,
    pub git_helper: Option<GitHelper>,
    pub asset_updater_servers: Vec<AssetUpdaterInfo>,
    http_client: reqwest::Client,
    update_lock: tokio::sync::Mutex<()>,
    /// Serializes version-file writes with the AppHashUpdater for the same region
    /// so their read-modify-writes do not clobber each other's fields.
    version_lock: Arc<tokio::sync::Mutex<()>>,
    db: Option<sea_orm::DatabaseConnection>,
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
            git_helper,
            asset_updater_servers,
            http_client,
            update_lock: tokio::sync::Mutex::new(()),
            version_lock,
            db,
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
        let session = match self.client.get_session() {
            Some(c) => c,
            None => {
                error!(
                    "{} No session available",
                    self.region.as_str().to_uppercase()
                );
                return;
            }
        };
        let login_response = match self.client.login(&session).await {
            Ok(r) => r,
            Err(crate::error::AppError::UpgradeRequired) => {
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
                    return;
                }
                match self.client.login(&session).await {
                    Ok(r) => r,
                    Err(e) => {
                        error!(
                            "{} Failed to login after version refresh: {}",
                            self.region.as_str().to_uppercase(),
                            e
                        );
                        return;
                    }
                }
            }
            Err(e) => {
                error!(
                    "{} Failed to login: {}",
                    self.region.as_str().to_uppercase(),
                    e
                );
                return;
            }
        };
        let (need_master_update, need_asset_update, need_version_save) =
            if self.region.is_cp_server() {
                let (master, asset) = self.check_cp_versions(&login_response, &current_version);
                (master, asset, master || asset)
            } else {
                self.check_nuverse_versions(&login_response, &current_version)
            };
        if need_asset_update {
            self.call_all_asset_updaters(&login_response.asset_version, &login_response.asset_hash)
                .await;
        }
        if need_master_update {
            if self.region.is_cp_server() {
                info!(
                    "{} New master data version: {}",
                    self.region.as_str().to_uppercase(),
                    login_response.data_version
                );
            } else {
                info!(
                    "{} New master data version (cdnVersion: {})",
                    self.region.as_str().to_uppercase(),
                    login_response.cdn_version
                );
            }
            if let Err(e) = self.update_master_data(&session, &login_response).await {
                error!(
                    "{} Failed to update master data: {}",
                    self.region.as_str().to_uppercase(),
                    e
                );
                return;
            }
        }
        if need_version_save {
            let new_version = VersionInfo {
                app_version: current_version.app_version,
                app_hash: current_version.app_hash,
                data_version: login_response.data_version.clone(),
                asset_version: login_response.asset_version.clone(),
                asset_hash: login_response.asset_hash.clone(),
                cdn_version: login_response.cdn_version,
            };
            if let Err(e) = self.save_version(&new_version).await {
                error!(
                    "{} Failed to save version file: {}",
                    self.region.as_str().to_uppercase(),
                    e
                );
                return;
            }
            self.client.version_helper.update(new_version);
            if let Some(ref git_helper) = self.git_helper {
                // git shells out via std::process and the push is a network call;
                // run it on a blocking thread so it never stalls a tokio worker.
                let git_helper = git_helper.clone();
                let master_dir = self.client.config.master_dir.clone();
                let data_version = login_response.data_version.clone();
                let region_upper = self.region.as_str().to_uppercase();
                let push = tokio::task::spawn_blocking(move || {
                    git_helper.push_changes(&master_dir, &data_version)
                })
                .await;
                match push {
                    Ok(Ok(true)) => info!("{} Git pushed changes successfully", region_upper),
                    Ok(Ok(false)) => {}
                    Ok(Err(e)) => error!("{} Git push failed: {}", region_upper, e),
                    Err(e) => error!("{} Git push task failed: {}", region_upper, e),
                }
            }
        }
        info!(
            "{} Master data check complete",
            self.region.as_str().to_uppercase()
        );
    }

    fn check_cp_versions(
        &self,
        login: &crate::client::LoginResponse,
        current: &VersionInfo,
    ) -> (bool, bool) {
        let need_master =
            compare_version(&login.data_version, &current.data_version).unwrap_or(false);
        let need_asset =
            compare_version(&login.asset_version, &current.asset_version).unwrap_or(false);

        (need_master, need_asset)
    }

    fn check_nuverse_versions(
        &self,
        login: &crate::client::LoginResponse,
        current: &VersionInfo,
    ) -> (bool, bool, bool) {
        let need_cdn_update = login.cdn_version > current.cdn_version;
        let need_data_version_save = login.data_version != current.data_version;
        let need_version_save = need_cdn_update || need_data_version_save;
        (need_cdn_update, need_cdn_update, need_version_save)
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
        session: &crate::client::AccountSession,
        login: &crate::client::LoginResponse,
    ) -> Result<(), crate::error::AppError> {
        info!(
            "{} Downloading master data...",
            self.region.as_str().to_uppercase()
        );
        let master_dir = &self.client.config.master_dir;
        tokio::fs::create_dir_all(master_dir).await?;

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
                let data = self.download_cp_master_split(session, &api_path).await?;
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

        // Ingest into the DB synchronously (awaited so failures are visible and
        // engine-init errors are caught; CPU parsing is offloaded via
        // spawn_blocking inside the engine, so this does not starve the runtime).
        // It is BEST-EFFORT: a DB/ingest failure is logged loudly but must NOT
        // block the caller, because the version file and the git master-data
        // mirror track the downloaded files (already valid on disk) rather than DB
        // health. Coupling the mirror to ingest health would let one malformed
        // table freeze the mirror and the version forever (perpetual re-download).
        if let Some(db) = self.db.clone() {
            let region_upper = self.region.as_str().to_uppercase();
            info!(
                "{} Starting database ingestion for new master data...",
                region_upper
            );
            match crate::ingest_engine::IngestionEngine::new(db).await {
                Ok(engine) => {
                    let region_str = self.region.as_str().to_lowercase();
                    match engine.ingest_master_data(master_dir, &region_str).await {
                        Ok(()) => info!(
                            "{} Master Data successfully ingested into database",
                            region_upper
                        ),
                        Err(e) => error!(
                            "{} Master Data DB ingestion failed (files saved; git mirror and \
version unaffected): {e:#}",
                            region_upper
                        ),
                    }
                }
                Err(e) => error!(
                    "{} Failed to initialize ingestion engine (skipping DB ingest): {e:#}",
                    region_upper
                ),
            }
        }

        info!(
            "{} Master data updated",
            self.region.as_str().to_uppercase()
        );
        Ok(())
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
            match tokio::fs::write(&file_path, json).await {
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
        if fail_count > 0 && success_count == 0 {
            return Err(crate::error::AppError::ParseError(
                "All master file writes failed".to_string(),
            ));
        }
        Ok(())
    }

    async fn save_version(&self, version: &VersionInfo) -> Result<(), crate::error::AppError> {
        let path = &self.client.config.version_path;
        // Serialize with the AppHashUpdater so neither clobbers the other's fields.
        let _guard = self.version_lock.lock().await;
        let mut existing: serde_json::Map<String, serde_json::Value> = if Path::new(path).exists() {
            let data = tokio::fs::read(path).await?;
            sonic_rs::from_slice(&data).unwrap_or_default()
        } else {
            serde_json::Map::new()
        };
        existing.insert(
            "appVersion".to_string(),
            serde_json::Value::String(version.app_version.clone()),
        );
        existing.insert(
            "appHash".to_string(),
            serde_json::Value::String(version.app_hash.clone()),
        );
        existing.insert(
            "dataVersion".to_string(),
            serde_json::Value::String(version.data_version.clone()),
        );
        existing.insert(
            "assetVersion".to_string(),
            serde_json::Value::String(version.asset_version.clone()),
        );
        existing.insert(
            "assetHash".to_string(),
            serde_json::Value::String(version.asset_hash.clone()),
        );
        existing.insert(
            "cdnVersion".to_string(),
            serde_json::Value::Number(version.cdn_version.into()),
        );
        let json = sonic_rs::to_string_pretty(&existing)
            .map_err(|e| crate::error::AppError::ParseError(e.to_string()))?;
        crate::client::helper::write_file_atomic(Path::new(path), json.as_bytes()).await?;
        let dir = Path::new(path).parent().unwrap_or(Path::new("."));
        let versioned_path = dir.join(format!("{}.json", version.data_version));
        crate::client::helper::write_file_atomic(&versioned_path, json.as_bytes()).await?;
        Ok(())
    }
}
