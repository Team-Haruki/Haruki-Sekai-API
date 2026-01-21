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

#[derive(Debug, Serialize, Deserialize)]
struct AssetUpdaterPayload {
    server: String,
    #[serde(rename = "assetVersion")]
    asset_version: String,
    #[serde(rename = "assetHash")]
    asset_hash: String,
}

pub struct MasterUpdater {
    pub region: ServerRegion,
    pub client: Arc<SekaiClient>,
    pub git_helper: Option<GitHelper>,
    pub asset_updater_servers: Vec<AssetUpdaterInfo>,
    http_client: reqwest::Client,
}

impl MasterUpdater {
    pub fn new(
        region: ServerRegion,
        client: Arc<SekaiClient>,
        git_config: Option<&GitConfig>,
        proxy: Option<String>,
        asset_updater_servers: Vec<AssetUpdaterInfo>,
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
        }
    }

    pub async fn check_update(&self) {
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
            Err(e) => {
                error!(
                    "{} Failed to login: {}",
                    self.region.as_str().to_uppercase(),
                    e
                );
                return;
            }
        };
        let (need_master_update, need_asset_update) = if self.region.is_cp_server() {
            self.check_cp_versions(&login_response, &current_version)
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
            }
        }
        if need_master_update || need_asset_update {
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
            }
            self.client.version_helper.update(new_version);
            if let Some(ref git_helper) = self.git_helper {
                let master_dir = &self.client.config.master_dir;
                match git_helper.push_changes(master_dir, &login_response.data_version) {
                    Ok(true) => info!(
                        "{} Git pushed changes successfully",
                        self.region.as_str().to_uppercase()
                    ),
                    Ok(false) => {}
                    Err(e) => error!(
                        "{} Git push failed: {}",
                        self.region.as_str().to_uppercase(),
                        e
                    ),
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
    ) -> (bool, bool) {
        let need_update = login.cdn_version > current.cdn_version;
        (need_update, need_update)
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
            server: self.region.as_str().to_string(),
            asset_version: asset_version.to_string(),
            asset_hash: asset_hash.to_string(),
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
                        tokio::time::sleep(Duration::from_secs(60)).await;
                        continue;
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
            use futures::stream::{self, StreamExt};
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
            let results: Vec<_> = stream::iter(paths)
                .map(|api_path| {
                    let client = self.client.clone();
                    let session = session.clone();
                    async move {
                        match client.get(&session, &api_path, None).await {
                            Ok(resp) => client.handle_response_ordered(resp).await.ok(),
                            Err(_) => None,
                        }
                    }
                })
                .buffer_unordered(3)
                .collect()
                .await;
            for data in results.into_iter().flatten() {
                self.save_master_files(&data, master_dir).await?;
            }
        } else {
            let url = format!(
                "{}/master-data-{}.info",
                self.client.config.nuverse_master_data_url, login.cdn_version
            );
            let http_client = &self.client.http_client;
            let resp = http_client.get(&url).send().await?;
            let body = resp.bytes().await?;
            let data = self.client.cryptor.unpack_ordered(&body)?;
            let structures = self.load_structures().await?;
            let restored = crate::client::nuverse::nuverse_master_restorer(&data, &structures)?;
            self.save_master_files(&restored, master_dir).await?;
        }

        info!(
            "{} Master data updated",
            self.region.as_str().to_uppercase()
        );
        Ok(())
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

    async fn load_structures(&self) -> Result<IndexMap<String, JsonValue>, crate::error::AppError> {
        let path = &self.client.config.nuverse_structure_file_path;
        if path.is_empty() {
            return Ok(IndexMap::new());
        }
        let data = tokio::fs::read(path).await?;
        let structures: IndexMap<String, JsonValue> = sonic_rs::from_slice(&data)
            .map_err(|e| crate::error::AppError::ParseError(e.to_string()))?;
        Ok(structures)
    }

    async fn save_version(&self, version: &VersionInfo) -> Result<(), crate::error::AppError> {
        let path = &self.client.config.version_path;
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
        tokio::fs::write(path, &json).await?;
        let dir = Path::new(path).parent().unwrap_or(Path::new("."));
        let versioned_path = dir.join(format!("{}.json", version.data_version));
        tokio::fs::write(versioned_path, &json).await?;
        Ok(())
    }
}
