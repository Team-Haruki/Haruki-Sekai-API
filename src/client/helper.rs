use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

pub struct CookieHelper {
    url: String,
    cookies: Arc<Mutex<String>>,
}

impl CookieHelper {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            cookies: Arc::new(Mutex::new(String::new())),
        }
    }

    pub async fn get_cookies(&self, proxy: Option<&str>) -> Result<String, AppError> {
        let mut client_builder = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("ProductName/134 CFNetwork/1408.0.4 Darwin/22.5.0");
        if let Some(proxy_url) = proxy {
            if !proxy_url.is_empty() {
                client_builder =
                    client_builder
                        .proxy(reqwest::Proxy::all(proxy_url).map_err(|e| {
                            AppError::NetworkError(format!("Invalid proxy: {}", e))
                        })?);
            }
        }
        let client = client_builder
            .build()
            .map_err(|e| AppError::NetworkError(e.to_string()))?;

        let mut last_error = None;
        for _ in 0..4 {
            let result = client
                .post(&self.url)
                .header("Accept", "*/*")
                .header("Connection", "keep-alive")
                .header("Accept-Language", "zh-CN,zh-Hans;q=0.9")
                .header("Accept-Encoding", "gzip, deflate, br")
                .header("X-Unity-Version", "2022.3.21f1")
                .send()
                .await;

            match result {
                Ok(resp) => {
                    if resp.status().is_success() {
                        if let Some(cookie) = resp.headers().get("set-cookie") {
                            let cookie_str = cookie.to_str().unwrap_or("").to_string();
                            *self.cookies.lock() = cookie_str.clone();
                            return Ok(cookie_str);
                        }
                    }
                    last_error = Some(AppError::NetworkError("No cookie in response".to_string()));
                }
                Err(e) => {
                    last_error = Some(AppError::NetworkError(e.to_string()));
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        Err(last_error
            .unwrap_or_else(|| AppError::NetworkError("Failed to fetch cookies".to_string())))
    }
    pub fn cached_cookies(&self) -> String {
        self.cookies.lock().clone()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VersionInfo {
    #[serde(rename = "appVersion")]
    pub app_version: String,
    #[serde(rename = "appHash")]
    pub app_hash: String,
    #[serde(rename = "dataVersion")]
    pub data_version: String,
    #[serde(rename = "assetVersion")]
    pub asset_version: String,
    #[serde(rename = "assetHash", default)]
    pub asset_hash: String,
    #[serde(rename = "cdnVersion", default)]
    pub cdn_version: i32,
}
pub struct VersionHelper {
    version_file_path: String,
    version_info: Arc<Mutex<VersionInfo>>,
}

impl VersionHelper {
    pub fn new(version_file_path: &str) -> Self {
        Self {
            version_file_path: version_file_path.to_string(),
            version_info: Arc::new(Mutex::new(VersionInfo::default())),
        }
    }

    pub async fn load(&self) -> Result<VersionInfo, AppError> {
        let path = Path::new(&self.version_file_path);
        let data = tokio::fs::read(path)
            .await
            .map_err(|e| AppError::ParseError(format!("Failed to read version file: {}", e)))?;

        let info: VersionInfo = sonic_rs::from_slice(&data)
            .map_err(|e| AppError::ParseError(format!("Failed to parse version file: {}", e)))?;

        *self.version_info.lock() = info.clone();
        Ok(info)
    }

    pub fn get(&self) -> VersionInfo {
        self.version_info.lock().clone()
    }

    pub fn update(&self, info: VersionInfo) {
        *self.version_info.lock() = info;
    }
}

pub fn compare_version(new_version: &str, current_version: &str) -> Result<bool, AppError> {
    let parse_segments = |v: &str| -> Result<Vec<u32>, AppError> {
        v.split('.')
            .map(|s| {
                s.parse::<u32>().map_err(|e| {
                    AppError::ParseError(format!("Invalid version segment '{}': {}", s, e))
                })
            })
            .collect()
    };
    let new_segments = parse_segments(new_version)?;
    let current_segments = parse_segments(current_version)?;
    let max_len = new_segments.len().max(current_segments.len());
    for i in 0..max_len {
        let new_seg = new_segments.get(i).copied().unwrap_or(0);
        let cur_seg = current_segments.get(i).copied().unwrap_or(0);

        if new_seg > cur_seg {
            return Ok(true);
        } else if new_seg < cur_seg {
            return Ok(false);
        }
    }
    Ok(false)
}
