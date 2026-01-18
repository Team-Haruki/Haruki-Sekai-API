use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::{error, info, warn};

use crate::config::{AppHashSource, ServerRegion};
use crate::error::AppError;

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
    pub version_path: String,
    pub proxy: Option<String>,
}

impl AppHashUpdater {
    pub fn new(
        region: ServerRegion,
        sources: Vec<AppHashSource>,
        version_path: String,
        proxy: Option<String>,
    ) -> Self {
        Self {
            region,
            sources,
            version_path,
            proxy,
        }
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
        let data = tokio::fs::read(&self.version_path).await?;
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
        let dir = Path::new(&source.dir);
        if !tokio::fs::try_exists(dir).await.unwrap_or(false) {
            return Ok(None);
        }
        let region_name = self.region.as_str();
        let file_path = dir.join(format!("{}.json", region_name));
        if !tokio::fs::try_exists(&file_path).await.unwrap_or(false) {
            return Ok(None);
        }
        let data = tokio::fs::read(&file_path).await?;
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
        let data = tokio::fs::read(&self.version_path).await?;
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
        tokio::fs::write(&self.version_path, json).await?;
        Ok(())
    }
}
