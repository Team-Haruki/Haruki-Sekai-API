use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::{error, info, warn};

use crate::client::helper::effective_app_version;
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
    /// Shared with the region's MasterUpdater to serialize version-file writes.
    version_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
}

impl AppHashUpdater {
    pub fn new(
        region: ServerRegion,
        sources: Vec<AppHashSource>,
        version_path: String,
        proxy: Option<String>,
        version_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    ) -> Self {
        Self {
            region,
            sources,
            version_path,
            proxy,
            version_lock,
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
                Ok(Some(mut new_info)) => {
                    let advertised_app_version = new_info.app_version.clone();
                    new_info.app_version = if new_info.app_version.trim().is_empty() {
                        effective_app_version(self.region, &current.app_version)
                    } else {
                        effective_app_version(self.region, &new_info.app_version)
                    };
                    if new_info.app_hash.trim().is_empty() {
                        new_info.app_hash = current.app_hash.clone();
                    }
                    if new_info.app_version != current.app_version
                        || new_info.app_hash != current.app_hash
                    {
                        if advertised_app_version == new_info.app_version {
                            info!(
                                "{} Found new app version: {} (hash: {})",
                                self.region.as_str().to_uppercase(),
                                new_info.app_version,
                                // chars(), not byte slicing: the hash is
                                // network-supplied and panic = "abort" in release.
                                new_info.app_hash.chars().take(16).collect::<String>()
                            );
                        } else {
                            info!(
                                "{} Found new app version: {} (effective: {}, hash: {})",
                                self.region.as_str().to_uppercase(),
                                advertised_app_version,
                                new_info.app_version,
                                new_info.app_hash.chars().take(16).collect::<String>()
                            );
                        }

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
        // Serialize with the MasterUpdater so neither clobbers the other's fields,
        // and read-modify-write the file under the lock.
        let _guard = self.version_lock.lock().await;
        let data = tokio::fs::read(&self.version_path).await?;
        let mut version: serde_json::Map<String, serde_json::Value> = sonic_rs::from_slice(&data)?;
        if !info.app_version.trim().is_empty() {
            let app_version = effective_app_version(self.region, &info.app_version);
            version.insert(
                "appVersion".to_string(),
                serde_json::Value::String(app_version),
            );
        }
        if !info.app_hash.trim().is_empty() {
            version.insert(
                "appHash".to_string(),
                serde_json::Value::String(info.app_hash.clone()),
            );
        }
        let json = sonic_rs::to_string_pretty(&version)?;
        crate::client::helper::write_file_atomic(
            std::path::Path::new(&self.version_path),
            json.as_bytes(),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn persists_zero_patch_for_nuverse_app_version() {
        let dir = std::env::temp_dir().join(format!(
            "haruki_nuverse_app_version_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let version_path = dir.join("current_version.json");
        std::fs::write(
            &version_path,
            br#"{"appVersion":"6.0.0","appHash":"old","dataVersion":"6.0.0.48"}"#,
        )
        .unwrap();

        let updater = AppHashUpdater::new(
            ServerRegion::Cn,
            Vec::new(),
            version_path.to_string_lossy().into_owned(),
            None,
            Arc::new(tokio::sync::Mutex::new(())),
        );
        updater
            .update_version(&AppInfo {
                app_version: "6.0.2".to_string(),
                app_hash: "new".to_string(),
            })
            .await
            .unwrap();

        let saved: serde_json::Value =
            sonic_rs::from_slice(&std::fs::read(&version_path).unwrap()).unwrap();
        assert_eq!(saved["appVersion"], "6.0.0");
        assert_eq!(saved["appHash"], "new");
        assert_eq!(saved["dataVersion"], "6.0.0.48");

        updater
            .update_version(&AppInfo {
                app_version: String::new(),
                app_hash: String::new(),
            })
            .await
            .unwrap();
        let saved: serde_json::Value =
            sonic_rs::from_slice(&std::fs::read(&version_path).unwrap()).unwrap();
        assert_eq!(saved["appVersion"], "6.0.0");
        assert_eq!(saved["appHash"], "new");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
