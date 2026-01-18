use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexMap;
use parking_lot::Mutex;
use reqwest::{Client, Response};
use serde::de::DeserializeOwned;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::config::{ServerConfig, ServerRegion};
use crate::crypto::SekaiCryptor;
use crate::error::{AppError, SekaiHttpStatus};

use super::account::{AccountType, SekaiAccount};
use super::helper::{CookieHelper, VersionHelper, VersionInfo};

pub struct SekaiClient {
    pub region: ServerRegion,
    pub config: ServerConfig,
    pub account: Arc<Mutex<AccountType>>,
    pub cookie_helper: Option<Arc<CookieHelper>>,
    pub version_helper: Arc<VersionHelper>,
    pub proxy: Option<String>,
    pub cryptor: SekaiCryptor,
    pub headers: Arc<Mutex<HashMap<String, String>>>,
    pub session_token: Arc<Mutex<Option<String>>>,
    pub http_client: Client,
    api_lock: Arc<tokio::sync::Mutex<()>>,
}

impl SekaiClient {
    pub fn new(
        region: ServerRegion,
        config: ServerConfig,
        account: AccountType,
        cookie_helper: Option<Arc<CookieHelper>>,
        version_helper: Arc<VersionHelper>,
        proxy: Option<String>,
    ) -> Result<Self, AppError> {
        let cryptor = SekaiCryptor::from_hex(&config.aes_key_hex, &config.aes_iv_hex)?;
        let mut headers = HashMap::new();
        for (k, v) in &config.headers {
            headers.insert(k.clone(), v.clone());
        }
        let mut client_builder = Client::builder()
            .timeout(Duration::from_secs(45))
            .pool_max_idle_per_host(20)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60));
        if let Some(ref proxy_url) = proxy {
            if !proxy_url.is_empty() {
                client_builder =
                    client_builder
                        .proxy(reqwest::Proxy::all(proxy_url).map_err(|e| {
                            AppError::NetworkError(format!("Invalid proxy: {}", e))
                        })?);
            }
        }
        let http_client = client_builder
            .build()
            .map_err(|e| AppError::NetworkError(e.to_string()))?;
        Ok(Self {
            region,
            config,
            account: Arc::new(Mutex::new(account)),
            cookie_helper,
            version_helper,
            proxy,
            cryptor,
            headers: Arc::new(Mutex::new(headers)),
            session_token: Arc::new(Mutex::new(None)),
            http_client,
            api_lock: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    pub async fn init(&self) -> Result<(), AppError> {
        if let Some(ref helper) = self.cookie_helper {
            let cookie = helper.get_cookies(self.proxy.as_deref()).await?;
            self.headers.lock().insert("Cookie".to_string(), cookie);
        }
        let version = self.version_helper.load().await?;
        self.update_version_headers(&version);
        Ok(())
    }

    fn update_version_headers(&self, version: &VersionInfo) {
        let mut headers = self.headers.lock();
        headers.insert("X-App-Version".to_string(), version.app_version.clone());
        headers.insert("X-Data-Version".to_string(), version.data_version.clone());
        headers.insert("X-Asset-Version".to_string(), version.asset_version.clone());
        headers.insert("X-App-Hash".to_string(), version.app_hash.clone());
    }

    pub async fn refresh_version(&self) -> Result<(), AppError> {
        let version = self.version_helper.load().await?;
        self.update_version_headers(&version);
        Ok(())
    }

    pub async fn refresh_cookies(&self) -> Result<(), AppError> {
        if let Some(ref helper) = self.cookie_helper {
            let cookie = helper.get_cookies(self.proxy.as_deref()).await?;
            self.headers.lock().insert("Cookie".to_string(), cookie);
        }
        Ok(())
    }

    fn prepare_request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        let mut req = self.http_client.request(method, url);
        let headers = self.headers.lock();
        for (k, v) in headers.iter() {
            if k.to_lowercase() != "x-request-id" {
                req = req.header(k, v);
            }
        }
        if let Some(ref token) = *self.session_token.lock() {
            req = req.header("X-Session-Token", token);
        }
        req = req.header("X-Request-Id", Uuid::new_v4().to_string());

        req
    }

    fn update_session_token(&self, resp: &Response) {
        if let Some(token) = resp.headers().get("x-session-token") {
            if let Ok(token_str) = token.to_str() {
                let old_token = self.session_token.lock().clone();
                *self.session_token.lock() = Some(token_str.to_string());
                debug!(
                    "Account #{} session token updated (old: {:?}, new: {}...)",
                    self.account.lock().user_id(),
                    old_token.as_deref().map(|s| &s[..s.len().min(40)]),
                    &token_str[..token_str.len().min(40)]
                );
            }
        }
    }

    pub async fn call_api<T: serde::Serialize>(
        &self,
        method: &str,
        path: &str,
        data: Option<&T>,
        params: Option<&HashMap<String, String>>,
    ) -> Result<Response, AppError> {
        let _lock = self.api_lock.lock().await;
        let user_id = self.account.lock().user_id().to_string();
        let url = format!("{}/api{}", self.config.api_url, path).replace("{userId}", &user_id);
        info!("Account #{} {} {}", user_id, method.to_uppercase(), path);
        let max_retries = 4;
        let mut last_error = None;
        for attempt in 1..=max_retries {
            let method_enum = match method.to_uppercase().as_str() {
                "GET" => reqwest::Method::GET,
                "POST" => reqwest::Method::POST,
                "PUT" => reqwest::Method::PUT,
                "DELETE" => reqwest::Method::DELETE,
                "PATCH" => reqwest::Method::PATCH,
                _ => reqwest::Method::GET,
            };
            let mut req = self.prepare_request(method_enum, &url);
            if let Some(p) = params {
                req = req.query(p);
            }
            if let Some(body_data) = data {
                let packed = self.cryptor.pack(body_data)?;
                req = req.body(packed);
            }
            match req.send().await {
                Ok(resp) => {
                    self.update_session_token(&resp);
                    return Ok(resp);
                }
                Err(e) => {
                    if e.is_timeout() {
                        warn!(
                            "Account #{} request timed out (attempt {}), retrying...",
                            self.account.lock().user_id(),
                            attempt
                        );
                    } else {
                        error!(
                            "request error (attempt {}): server={}, err={}",
                            attempt,
                            self.region.as_str().to_uppercase(),
                            e
                        );
                    }
                    last_error = Some(AppError::NetworkError(e.to_string()));
                }
            }
            if attempt < max_retries {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        Err(last_error.unwrap_or(AppError::NetworkError(
            "Request failed after retries".to_string(),
        )))
    }

    pub async fn get(
        &self,
        path: &str,
        params: Option<&HashMap<String, String>>,
    ) -> Result<Response, AppError> {
        self.call_api::<()>("GET", path, None, params).await
    }

    pub async fn post<T: serde::Serialize>(
        &self,
        path: &str,
        data: Option<&T>,
        params: Option<&HashMap<String, String>>,
    ) -> Result<Response, AppError> {
        self.call_api("POST", path, data, params).await
    }

    pub async fn handle_response<T: DeserializeOwned>(
        &self,
        resp: Response,
    ) -> Result<T, AppError> {
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        let body = resp
            .bytes()
            .await
            .map_err(|e| AppError::NetworkError(e.to_string()))?;
        if content_type.contains("octet-stream") || content_type.contains("binary") {
            let sekai_status = SekaiHttpStatus::from_code(status)?;
            match sekai_status {
                SekaiHttpStatus::Ok
                | SekaiHttpStatus::ClientError
                | SekaiHttpStatus::NotFound
                | SekaiHttpStatus::Conflict => self.cryptor.unpack(&body),
                SekaiHttpStatus::SessionError => Err(AppError::SessionError),
                SekaiHttpStatus::GameUpgrade => Err(AppError::UpgradeRequired),
                SekaiHttpStatus::UnderMaintenance => Err(AppError::UnderMaintenance),
                _ => Err(AppError::Unknown {
                    status,
                    body: String::from_utf8_lossy(&body).to_string(),
                }),
            }
        } else {
            let sekai_status = SekaiHttpStatus::from_code(status)?;
            match sekai_status {
                SekaiHttpStatus::UnderMaintenance => Err(AppError::UnderMaintenance),
                SekaiHttpStatus::ServerError => Err(AppError::Unknown {
                    status,
                    body: String::from_utf8_lossy(&body).to_string(),
                }),
                SekaiHttpStatus::SessionError if content_type.contains("xml") => {
                    Err(AppError::CookieExpired)
                }
                _ => Err(AppError::Unknown {
                    status,
                    body: String::from_utf8_lossy(&body).to_string(),
                }),
            }
        }
    }

    pub async fn handle_response_ordered(
        &self,
        resp: Response,
    ) -> Result<IndexMap<String, serde_json::Value>, AppError> {
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        let body = resp
            .bytes()
            .await
            .map_err(|e| AppError::NetworkError(e.to_string()))?;
        if content_type.contains("octet-stream") || content_type.contains("binary") {
            let sekai_status = SekaiHttpStatus::from_code(status)?;

            match sekai_status {
                SekaiHttpStatus::Ok
                | SekaiHttpStatus::ClientError
                | SekaiHttpStatus::NotFound
                | SekaiHttpStatus::Conflict => self.cryptor.unpack_ordered(&body),
                SekaiHttpStatus::SessionError => Err(AppError::SessionError),
                SekaiHttpStatus::GameUpgrade => Err(AppError::UpgradeRequired),
                SekaiHttpStatus::UnderMaintenance => Err(AppError::UnderMaintenance),
                _ => Err(AppError::Unknown {
                    status,
                    body: String::from_utf8_lossy(&body).to_string(),
                }),
            }
        } else {
            Err(AppError::Unknown {
                status,
                body: String::from_utf8_lossy(&body).to_string(),
            })
        }
    }

    #[tracing::instrument(skip(self), fields(user_id = %self.account.lock().user_id()))]
    pub async fn login(&self) -> Result<LoginResponse, AppError> {
        let payload = self.account.lock().dump()?;
        let encrypted = self.cryptor.pack_bytes(&payload)?;
        let (url, method) = if self.region.is_cp_server() {
            let url = format!(
                "{}/api/user/{}/auth?refreshUpdatedResources=False",
                self.config.api_url,
                self.account.lock().user_id()
            );
            (url, reqwest::Method::PUT)
        } else {
            let url = format!("{}/api/user/auth", self.config.api_url);
            (url, reqwest::Method::POST)
        };
        let _lock = self.api_lock.lock().await;
        let mut req = self.prepare_request(method, &url);
        req = req.body(encrypted);
        info!("Account #{} logging in...", self.account.lock().user_id());
        let resp = req
            .send()
            .await
            .map_err(|e| AppError::NetworkError(e.to_string()))?;
        self.update_session_token(&resp);
        let login_resp: LoginResponse = self.handle_response(resp).await?;
        if !login_resp.session_token.is_empty() {
            *self.session_token.lock() = Some(login_resp.session_token.clone());
        }
        if !self.region.is_cp_server() {
            if let Some(ref user_reg) = login_resp.user_registration {
                if !user_reg.user_id.is_empty() && user_reg.user_id != "0" {
                    let old_uid = self.account.lock().user_id().to_string();
                    self.account.lock().set_user_id(user_reg.user_id.clone());
                    info!(
                        "Account #{} -> {} (from login response)",
                        old_uid, user_reg.user_id
                    );
                }
            }
        }
        info!(
            "Account #{} logged in successfully",
            self.account.lock().user_id()
        );
        Ok(login_resp)
    }

    pub async fn get_cp_mysekai_image(&self, path: &str) -> Result<Vec<u8>, AppError> {
        let path_clean = path.trim_start_matches('/');
        let image_url = format!("{}/image/mysekai-photo/{}", self.config.api_url, path_clean);
        let req = self.prepare_request(reqwest::Method::GET, &image_url);
        let resp = req
            .send()
            .await
            .map_err(|e| AppError::NetworkError(e.to_string()))?;

        let status = resp.status().as_u16();
        if status != 200 {
            return Err(AppError::Unknown {
                status,
                body: format!("Failed to fetch image from {}", image_url),
            });
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::NetworkError(e.to_string()))?;
        Ok(bytes.to_vec())
    }

    pub async fn get_nuverse_mysekai_image(
        &self,
        user_id: &str,
        index: &str,
    ) -> Result<Vec<u8>, AppError> {
        let path = format!("/user/{}/mysekai/photo/{}", user_id, index);
        let resp = self.get(&path, None).await?;
        let data: std::collections::HashMap<String, serde_json::Value> =
            self.handle_response(resp).await?;
        let thumbnail = data
            .get("thumbnail")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::ParseError("missing thumbnail in response".to_string()))?;
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(thumbnail)
            .map_err(|e| AppError::ParseError(format!("failed to decode base64: {}", e)))?;
        Ok(bytes)
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct LoginResponse {
    #[serde(rename = "sessionToken", default)]
    pub session_token: String,
    #[serde(rename = "dataVersion", default)]
    pub data_version: String,
    #[serde(rename = "assetVersion", default)]
    pub asset_version: String,
    #[serde(rename = "assetHash", default)]
    pub asset_hash: String,
    #[serde(rename = "suiteMasterSplitPath", default)]
    pub suite_master_split_path: Vec<String>,
    #[serde(rename = "cdnVersion", default)]
    pub cdn_version: i32,
    #[serde(rename = "userRegistration", default)]
    pub user_registration: Option<UserRegistration>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UserRegistration {
    #[serde(
        alias = "userId",
        alias = "userID",
        default,
        deserialize_with = "super::account::null_or_number_to_string"
    )]
    pub user_id: String,
}
