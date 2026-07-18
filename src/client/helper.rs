use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Write `contents` to `path` atomically: write a uniquely-named temp file in the
/// same directory and rename it over the target. A concurrent reader therefore
/// never observes a truncated/partial file (e.g. version_helper.load on the
/// request path while an updater rewrites the version file).
pub async fn write_file_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp, contents).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::write_file_atomic;

    // Atomic write: target ends with the new contents, overwrite works, and no
    // temp file is left behind in the directory.
    #[tokio::test]
    async fn write_file_atomic_overwrites_and_leaves_no_temp() {
        let dir = std::env::temp_dir().join(format!("haruki_atomic_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("current_version.json");

        write_file_atomic(&target, b"{\"v\":1}").await.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"{\"v\":1}");

        write_file_atomic(&target, b"{\"v\":2}").await.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"{\"v\":2}");

        let temp_leftovers = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(temp_leftovers, 0, "temp file must be renamed away");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

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
        for attempt in 0..4 {
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
                        // Collect every Set-Cookie header (multi-cookie auth like
                        // CDN signed cookies sets several), keep only the
                        // name=value pair of each (attributes such as Path/Expires
                        // do not belong in a Cookie request header), and treat an
                        // unparseable/empty result as a failure instead of caching
                        // an empty cookie as success.
                        let cookie_str = resp
                            .headers()
                            .get_all("set-cookie")
                            .iter()
                            .filter_map(|v| v.to_str().ok())
                            .filter_map(|v| {
                                let pair = v.split(';').next().unwrap_or("").trim();
                                (!pair.is_empty()).then(|| pair.to_string())
                            })
                            .collect::<Vec<_>>()
                            .join("; ");
                        if !cookie_str.is_empty() {
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
            if attempt < 3 {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
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
