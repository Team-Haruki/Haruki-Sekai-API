use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::config::{AppHashSource, ServerRegion, StorageConfig};
use crate::error::AppError;
use crate::storage::StorageLocation;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    #[serde(rename = "appVersion")]
    pub app_version: String,
    #[serde(rename = "appHash")]
    pub app_hash: String,
}

pub struct AppHashUpdater {
    pub region: ServerRegion,
    pub sources: Vec<AppHashSource>,
    pub version_store: StorageLocation,
    pub proxy: Option<String>,
}

impl AppHashUpdater {
    pub fn new(
        region: ServerRegion,
        sources: Vec<AppHashSource>,
        version_path: String,
        version_storage: StorageConfig,
        proxy: Option<String>,
    ) -> Result<Self, AppError> {
        Ok(Self {
            region,
            sources,
            version_store: StorageLocation::file(&version_storage, &version_path, "version_path")?,
            proxy,
        })
    }

    pub async fn check_update(&self) {
        info!(
            "{} Checking for app hash updates...",
            self.region.as_str().to_uppercase()
        );
        let current = match self.load_current_version().await {
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
        for source in &self.sources {
            match self.fetch_from_source(source).await {
                Ok(Some(new_info)) => {
                    if new_info.app_version != current.app_version
                        || new_info.app_hash != current.app_hash
                    {
                        info!(
                            "{} Found new app version: {} (hash: {})",
                            self.region.as_str().to_uppercase(),
                            new_info.app_version,
                            &new_info.app_hash[..new_info.app_hash.len().min(16)]
                        );

                        if let Err(e) = self.update_version(&new_info).await {
                            error!(
                                "{} Failed to update version: {}",
                                self.region.as_str().to_uppercase(),
                                e
                            );
                        }
                    }
                    break;
                }
                Ok(None) => continue,
                Err(e) => {
                    warn!(
                        "{} Failed to fetch from source: {}",
                        self.region.as_str().to_uppercase(),
                        e
                    );
                    continue;
                }
            }
        }
        info!(
            "{} App hash check complete",
            self.region.as_str().to_uppercase()
        );
    }

    async fn load_current_version(&self) -> Result<AppInfo, AppError> {
        let data = self.version_store.read_base().await?;
        #[derive(Deserialize)]
        struct VersionFile {
            #[serde(rename = "appVersion")]
            app_version: String,
            #[serde(rename = "appHash")]
            app_hash: String,
        }
        let version: VersionFile = sonic_rs::from_slice(&data)?;
        Ok(AppInfo {
            app_version: version.app_version,
            app_hash: version.app_hash,
        })
    }

    async fn fetch_from_source(&self, source: &AppHashSource) -> Result<Option<AppInfo>, AppError> {
        match source.source_type.as_str() {
            "file" => self.fetch_from_file(source).await,
            "url" => self.fetch_from_url(source).await,
            _ => Ok(None),
        }
    }

    async fn fetch_from_file(&self, source: &AppHashSource) -> Result<Option<AppInfo>, AppError> {
        let store = StorageLocation::dir(&source.storage, &source.dir, false, "apphash_source")?;
        if !store.is_available() {
            return Ok(None);
        }
        let region_name = self.region.as_str();
        let data = match store.read_child(&format!("{}.json", region_name)).await {
            Ok(data) => data,
            Err(AppError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        let info: AppInfo = sonic_rs::from_slice(&data)?;
        Ok(Some(info))
    }

    async fn fetch_from_url(&self, source: &AppHashSource) -> Result<Option<AppInfo>, AppError> {
        let mut builder = Client::builder().timeout(std::time::Duration::from_secs(10));
        if let Some(ref proxy) = self.proxy {
            if !proxy.is_empty() {
                builder = builder.proxy(
                    reqwest::Proxy::all(proxy)
                        .map_err(|e| AppError::NetworkError(e.to_string()))?,
                );
            }
        }
        let client = builder.build()?;
        let url = source.url.replace("{region}", self.region.as_str());
        let resp = client.get(&url).send().await?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let body = resp.bytes().await?;
        let info: AppInfo = sonic_rs::from_slice(&body)?;
        Ok(Some(info))
    }

    async fn update_version(&self, info: &AppInfo) -> Result<(), AppError> {
        let data = self.version_store.read_base().await?;
        let mut version: serde_json::Map<String, serde_json::Value> = sonic_rs::from_slice(&data)?;
        version.insert(
            "appVersion".to_string(),
            serde_json::Value::String(info.app_version.clone()),
        );
        version.insert(
            "appHash".to_string(),
            serde_json::Value::String(info.app_hash.clone()),
        );
        let json = sonic_rs::to_string_pretty(&version)?;
        self.version_store.write_base(json.into_bytes()).await?;
        Ok(())
    }
}
