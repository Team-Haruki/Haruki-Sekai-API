use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
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
    use axum::{http::StatusCode, response::IntoResponse, routing::post, Router};
    use reqwest::header::{HeaderMap, HeaderValue, SET_COOKIE};
    use serde_json::json;

    use crate::config::ServerRegion;

    use super::{
        compare_version, effective_app_version, extract_request_cookies, write_file_atomic,
        CookieHelper, VersionHelper, VersionInfo,
    };

    async fn spawn_cookie_server(with_cookie: bool) -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().route(
            "/cookie",
            post(move || async move {
                if with_cookie {
                    (
                        StatusCode::OK,
                        [(SET_COOKIE, "session=abc; Path=/; HttpOnly")],
                        "ok",
                    )
                        .into_response()
                } else {
                    (StatusCode::OK, "missing").into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{address}/cookie"), task)
    }

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

    #[test]
    fn extracts_cookies_from_multiple_set_cookie_headers() {
        let mut headers = HeaderMap::new();
        headers.append(
            SET_COOKIE,
            HeaderValue::from_static("session=abc; Path=/; HttpOnly"),
        );
        headers.append(
            SET_COOKIE,
            HeaderValue::from_static("token=def==; Secure; SameSite=None"),
        );

        assert_eq!(
            extract_request_cookies(&headers),
            "session=abc; token=def=="
        );
    }

    #[test]
    fn preserves_multiple_cookie_pairs_in_one_set_cookie_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            SET_COOKIE,
            HeaderValue::from_static(
                "cloudfront_policy=one; cloudfront_signature=two; cloudfront_key=three; Path=/",
            ),
        );

        assert_eq!(
            extract_request_cookies(&headers),
            "cloudfront_policy=one; cloudfront_signature=two; cloudfront_key=three"
        );
    }

    #[test]
    fn ignores_set_cookie_attributes_case_insensitively() {
        let mut headers = HeaderMap::new();
        headers.insert(
            SET_COOKIE,
            HeaderValue::from_static(
                "session=abc; DOMAIN=example.com; expires=Wed, 21 Oct 2026 07:28:00 GMT; \
                 MAX-AGE=3600; samesite=Lax; Priority=High",
            ),
        );

        assert_eq!(extract_request_cookies(&headers), "session=abc");
    }

    #[test]
    fn forces_nuverse_app_version_patch_to_zero() {
        for region in [ServerRegion::Tw, ServerRegion::Kr, ServerRegion::Cn] {
            assert_eq!(effective_app_version(region, "6.0.2"), "6.0.0");
            assert_eq!(effective_app_version(region, "3.4.99"), "3.4.0");
        }
    }

    #[test]
    fn preserves_cp_and_malformed_app_versions() {
        assert_eq!(effective_app_version(ServerRegion::Jp, "6.0.2"), "6.0.2");
        assert_eq!(effective_app_version(ServerRegion::En, "3.4.99"), "3.4.99");
        assert_eq!(effective_app_version(ServerRegion::Cn, "6.0"), "6.0");
        assert_eq!(
            effective_app_version(ServerRegion::Cn, "invalid"),
            "invalid"
        );
    }

    #[tokio::test]
    async fn cookie_helper_fetches_and_caches_cookie() {
        let (url, server) = spawn_cookie_server(true).await;
        let helper = CookieHelper::new(&url);

        assert!(helper.cached_cookies().is_empty());
        assert_eq!(helper.get_cookies(None).await.unwrap(), "session=abc");
        assert_eq!(helper.cached_cookies(), "session=abc");

        server.abort();
    }

    #[tokio::test]
    async fn cookie_helper_rejects_invalid_proxy() {
        let helper = CookieHelper::new("http://127.0.0.1/unused");
        let error = helper.get_cookies(Some("://invalid")).await.unwrap_err();
        assert!(error.to_string().contains("Invalid proxy"));
    }

    #[tokio::test]
    async fn version_helper_loads_gets_and_updates() {
        let dir = std::env::temp_dir().join(format!("haruki_version_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("version.json");
        std::fs::write(
            &path,
            json!({
                "appVersion": "6.0.1",
                "appHash": "hash",
                "dataVersion": "data",
                "assetVersion": "asset"
            })
            .to_string(),
        )
        .unwrap();

        let helper = VersionHelper::new(path.to_str().unwrap());
        let loaded = helper.load().await.unwrap();
        assert_eq!(loaded.app_version, "6.0.1");
        assert_eq!(helper.get().asset_hash, "");

        helper.update(VersionInfo {
            app_version: "6.1.0".to_string(),
            cdn_version: 7,
            ..VersionInfo::default()
        });
        assert_eq!(helper.get().app_version, "6.1.0");
        assert_eq!(helper.get().cdn_version, 7);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn version_helper_reports_read_and_parse_errors() {
        let missing = VersionHelper::new("/definitely/missing/version.json");
        assert!(missing
            .load()
            .await
            .unwrap_err()
            .to_string()
            .contains("read"));

        let path =
            std::env::temp_dir().join(format!("haruki_bad_version_{}.json", uuid::Uuid::new_v4()));
        std::fs::write(&path, "not json").unwrap();
        let invalid = VersionHelper::new(path.to_str().unwrap());
        assert!(invalid
            .load()
            .await
            .unwrap_err()
            .to_string()
            .contains("parse"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn compares_versions_with_different_lengths() {
        assert!(compare_version("2.0", "1.9.9").unwrap());
        assert!(compare_version("1.2.1", "1.2").unwrap());
        assert!(!compare_version("1.2", "1.2.0").unwrap());
        assert!(!compare_version("1.1.9", "1.2").unwrap());
        assert!(compare_version("1.bad", "1.0").is_err());
        assert!(compare_version("1.0", "bad").is_err());
    }
}

/// Nuverse may advertise a patch release before its login endpoint accepts that
/// exact version. Patch releases do not affect the Nuverse protocol, so requests
/// and persisted version state use the corresponding `.0` patch as a fallback.
pub fn effective_app_version(region: ServerRegion, app_version: &str) -> String {
    if region.is_cp_server() {
        return app_version.to_string();
    }

    let segments: Vec<&str> = app_version.split('.').collect();
    if segments.len() != 3
        || segments
            .iter()
            .any(|segment| segment.parse::<u32>().is_err())
    {
        return app_version.to_string();
    }

    format!("{}.{}.0", segments[0], segments[1])
}

fn extract_request_cookies(headers: &reqwest::header::HeaderMap) -> String {
    headers
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|segment| {
            let (name, value) = segment.trim().split_once('=')?;
            let name = name.trim();
            let value = value.trim();
            if name.is_empty() || is_set_cookie_attribute(name) {
                return None;
            }
            Some(format!("{name}={value}"))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn is_set_cookie_attribute(name: &str) -> bool {
    [
        "domain",
        "path",
        "expires",
        "max-age",
        "samesite",
        "priority",
        "version",
        "comment",
        "commenturl",
        "port",
    ]
    .iter()
    .any(|attribute| name.eq_ignore_ascii_case(attribute))
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
                        // The JP cookie service may combine several cookie pairs in
                        // one Set-Cookie header, so parse every semicolon-delimited
                        // pair while dropping response-only attributes.
                        let cookie_str = extract_request_cookies(resp.headers());
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
