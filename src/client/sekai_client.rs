use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};
use reqwest::{Client, Response};
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::config::{ServerConfig, ServerRegion};
use crate::crypto::SekaiCryptor;
use crate::error::{AppError, SekaiHttpStatus};

use super::account::{AccountType, SekaiAccount, SekaiAccountCP, SekaiAccountNuverse};
use super::helper::{CookieHelper, VersionHelper, VersionInfo};
use super::session::AccountSession;
use super::token_utils;

pub struct SekaiClient {
    pub region: ServerRegion,
    pub config: ServerConfig,
    pub cookie_helper: Option<Arc<CookieHelper>>,
    pub version_helper: Arc<VersionHelper>,
    pub proxy: Option<String>,
    pub cryptor: SekaiCryptor,
    pub headers: Arc<Mutex<HashMap<String, String>>>,
    pub http_client: Client,

    sessions: Arc<RwLock<Vec<Arc<AccountSession>>>>,
    session_index: AtomicUsize,
    reload_in_progress: Arc<std::sync::atomic::AtomicBool>,
}

impl SekaiClient {
    pub async fn new(
        region: ServerRegion,
        config: ServerConfig,
        proxy: Option<String>,
        jp_cookie_url: Option<String>,
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
        let version_helper = Arc::new(VersionHelper::new(&config.version_path));
        let cookie_helper = if region == ServerRegion::Jp && config.require_cookies {
            jp_cookie_url
                .filter(|url| !url.is_empty())
                .map(|url| Arc::new(CookieHelper::new(&url)))
        } else {
            None
        };
        let client = Self {
            region,
            config,
            cookie_helper,
            version_helper,
            proxy,
            cryptor,
            headers: Arc::new(Mutex::new(headers)),
            http_client,
            sessions: Arc::new(RwLock::new(Vec::new())),
            session_index: AtomicUsize::new(0),
            reload_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        Ok(client)
    }

    pub async fn init(&self) -> Result<(), AppError> {
        info!(
            "{} Initializing client...",
            self.region.as_str().to_uppercase()
        );
        if let Some(ref helper) = self.cookie_helper {
            let cookie = helper.get_cookies(self.proxy.as_deref()).await?;
            self.headers.lock().insert("Cookie".to_string(), cookie);
        }
        let version = self.version_helper.load().await?;
        self.update_version_headers(&version);
        let accounts = self.parse_accounts()?;
        if accounts.is_empty() {
            warn!(
                "{} No accounts found in {}",
                self.region.as_str().to_uppercase(),
                self.config.account_dir
            );
            return Ok(());
        }
        for account in accounts {
            if self.region.is_cp_server() && account.user_id().is_empty() {
                warn!(
                    "{} Skipping account with empty user_id",
                    self.region.as_str().to_uppercase()
                );
                continue;
            }
            let session = Arc::new(AccountSession::new(account));
            match self.login(&session).await {
                Ok(_) => {
                    self.sessions.write().push(session);
                }
                Err(e) => {
                    error!(
                        "{} Failed to login account: {}",
                        self.region.as_str().to_uppercase(),
                        e
                    );
                }
            }
        }
        info!(
            "{} Client initialized with {} sessions",
            self.region.as_str().to_uppercase(),
            self.sessions.read().len()
        );
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

    pub async fn reload_accounts(&self) -> Result<(), AppError> {
        info!(
            "{} Reloading accounts...",
            self.region.as_str().to_uppercase()
        );
        self.reload_in_progress.store(true, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(3)).await;
        {
            let mut sessions = self.sessions.write();
            sessions.clear();
            self.session_index.store(0, Ordering::SeqCst);
        }
        let accounts = self.parse_accounts()?;
        for account in accounts {
            if self.region.is_cp_server() && account.user_id().is_empty() {
                warn!(
                    "{} Skipping account with empty user_id",
                    self.region.as_str().to_uppercase()
                );
                continue;
            }
            let session = Arc::new(AccountSession::new(account));
            match self.login(&session).await {
                Ok(_) => {
                    self.sessions.write().push(session);
                }
                Err(e) => {
                    error!(
                        "{} Failed to login account: {}",
                        self.region.as_str().to_uppercase(),
                        e
                    );
                }
            }
        }
        self.reload_in_progress.store(false, Ordering::SeqCst);
        info!(
            "{} Accounts reloaded, {} sessions active",
            self.region.as_str().to_uppercase(),
            self.sessions.read().len()
        );
        Ok(())
    }

    pub fn start_file_watcher(self: Arc<Self>) -> Result<(), AppError> {
        use notify::{Config, PollWatcher, RecursiveMode, Watcher};
        use std::sync::mpsc::channel;

        let account_dir = self.config.account_dir.clone();
        if account_dir.is_empty() || !Path::new(&account_dir).exists() {
            warn!(
                "{} Account directory not found: {}, skipping file watcher",
                self.region.as_str().to_uppercase(),
                account_dir
            );
            return Ok(());
        }
        let (tx, rx) = channel();
        let config = Config::default().with_poll_interval(Duration::from_secs(5));
        let mut watcher = PollWatcher::new(tx, config)
            .map_err(|e| AppError::Internal(format!("Failed to create file watcher: {}", e)))?;
        watcher
            .watch(Path::new(&account_dir), RecursiveMode::NonRecursive)
            .map_err(|e| AppError::Internal(format!("Failed to watch directory: {}", e)))?;
        let client = self.clone();
        let region_str = self.region.as_str().to_uppercase();
        std::thread::spawn(move || {
            let _watcher = watcher;
            info!(
                "{} File watcher started for {} (polling mode, 5s interval)",
                region_str, account_dir
            );
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime for file watcher");
            let mut last_reload = std::time::Instant::now();
            let debounce_duration = Duration::from_secs(2);
            for res in rx {
                match res {
                    Ok(event) => {
                        use notify::EventKind;
                        match event.kind {
                            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                                if last_reload.elapsed() < debounce_duration {
                                    debug!(
                                        "{} Skipping reload (debounce), last reload was {:?} ago",
                                        region_str,
                                        last_reload.elapsed()
                                    );
                                    continue;
                                }
                                info!(
                                    "{} Account file change detected: {:?}",
                                    region_str, event.paths
                                );
                                last_reload = std::time::Instant::now();
                                let client_clone = client.clone();
                                rt.block_on(async {
                                    if let Err(e) = client_clone.reload_accounts().await {
                                        error!("{} Failed to reload accounts: {}", region_str, e);
                                    }
                                });
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        error!("{} File watcher error: {}", region_str, e);
                    }
                }
            }
        });
        Ok(())
    }

    fn parse_accounts(&self) -> Result<Vec<AccountType>, AppError> {
        let mut accounts = Vec::new();
        let account_dir = Path::new(&self.config.account_dir);
        if !account_dir.exists() {
            return Ok(accounts);
        }
        let entries = fs::read_dir(account_dir)
            .map_err(|e| AppError::ParseError(format!("Failed to read account dir: {}", e)))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let data = match fs::read(&path) {
                Ok(d) => d,
                Err(e) => {
                    warn!("Failed to read {}: {}", path.display(), e);
                    continue;
                }
            };
            match self.parse_account_file(&path, &data) {
                Ok(mut accs) => accounts.append(&mut accs),
                Err(e) => {
                    warn!("Failed to parse {}: {}", path.display(), e);
                }
            }
        }
        Ok(accounts)
    }

    fn parse_account_file(&self, path: &Path, data: &[u8]) -> Result<Vec<AccountType>, AppError> {
        let value: serde_json::Value = sonic_rs::from_slice(data)
            .map_err(|e| AppError::ParseError(format!("JSON parse error: {}", e)))?;
        let mut accounts = Vec::new();
        match value {
            serde_json::Value::Array(arr) => {
                for (idx, item) in arr.into_iter().enumerate() {
                    if let Some(acc) = self.parse_account_value(item, path, Some(idx)) {
                        accounts.push(acc);
                    }
                }
            }
            serde_json::Value::Object(_) => {
                if let Some(acc) = self.parse_account_value(value, path, None) {
                    accounts.push(acc);
                }
            }
            _ => {}
        }
        Ok(accounts)
    }

    fn parse_account_value(
        &self,
        value: serde_json::Value,
        path: &Path,
        idx: Option<usize>,
    ) -> Option<AccountType> {
        let log_prefix = if let Some(i) = idx {
            format!("[{}][{}]", path.display(), i)
        } else {
            format!("[{}]", path.display())
        };

        if self.region.is_cp_server() {
            let json_str = serde_json::to_string(&value).ok()?;
            match sonic_rs::from_str::<SekaiAccountCP>(&json_str) {
                Ok(mut acc) => {
                    if let Ok(user_id) = token_utils::extract_user_id_from_jwt(&acc.credential) {
                        debug!("{} Extracted user_id from JWT: {}", log_prefix, user_id);
                        acc.user_id = user_id;
                    } else if acc.user_id.is_empty() {
                        warn!(
                            "{} Failed to extract user_id from JWT and no fallback",
                            log_prefix
                        );
                    }
                    Some(AccountType::CP(acc))
                }
                Err(e) => {
                    warn!("{} CP unmarshal error: {}", log_prefix, e);
                    None
                }
            }
        } else {
            let json_str = serde_json::to_string(&value).ok()?;
            match sonic_rs::from_str::<SekaiAccountNuverse>(&json_str) {
                Ok(mut acc) => {
                    if let Ok(user_id) =
                        token_utils::extract_user_id_from_nuverse_token(&acc.access_token)
                    {
                        debug!(
                            "{} Extracted user_id from Nuverse token: {}",
                            log_prefix, user_id
                        );
                        acc.user_id = user_id;
                    } else if acc.user_id.is_empty() || acc.user_id == "0" {
                        warn!(
                            "{} Failed to extract user_id from Nuverse token and no fallback",
                            log_prefix
                        );
                    }
                    Some(AccountType::Nuverse(acc))
                }
                Err(e) => {
                    warn!("{} Nuverse unmarshal error: {}", log_prefix, e);
                    None
                }
            }
        }
    }

    #[must_use]
    pub fn get_session(&self) -> Option<Arc<AccountSession>> {
        let sessions = self.sessions.read();
        if sessions.is_empty() {
            return None;
        }
        let idx = self.session_index.fetch_add(1, Ordering::SeqCst) % sessions.len();
        Some(sessions[idx].clone())
    }

    fn prepare_request(
        &self,
        session: &AccountSession,
        method: reqwest::Method,
        url: &str,
    ) -> reqwest::RequestBuilder {
        let mut req = self.http_client.request(method, url);
        let headers = self.headers.lock();
        for (k, v) in headers.iter() {
            if k.to_lowercase() != "x-request-id" {
                req = req.header(k, v);
            }
        }
        if let Some(ref token) = session.get_session_token() {
            req = req.header("X-Session-Token", token);
        }
        req = req.header("X-Request-Id", Uuid::new_v4().to_string());
        req
    }

    fn update_session_token(&self, session: &AccountSession, resp: &Response) {
        if let Some(token) = resp.headers().get("x-session-token") {
            if let Ok(token_str) = token.to_str() {
                let old_token = session.get_session_token();
                session.set_session_token(Some(token_str.to_string()));
                debug!(
                    "Account #{} session token updated (old: {:?}, new: {}...)",
                    session.user_id(),
                    old_token.as_deref().map(|s| &s[..s.len().min(40)]),
                    &token_str[..token_str.len().min(40)]
                );
            }
        }
    }

    pub async fn call_api<T: serde::Serialize>(
        &self,
        session: &AccountSession,
        method: &str,
        path: &str,
        data: Option<&T>,
        params: Option<&HashMap<String, String>>,
    ) -> Result<Response, AppError> {
        let _lock = session.lock_api().await;
        let user_id = session.user_id().to_string();
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
            let mut req = self.prepare_request(session, method_enum, &url);
            if let Some(p) = params {
                req = req.query(p);
            }
            if let Some(body_data) = data {
                let packed = self.cryptor.pack(body_data)?;
                req = req.body(packed);
            }
            match req.send().await {
                Ok(resp) => {
                    self.update_session_token(session, &resp);
                    return Ok(resp);
                }
                Err(e) => {
                    if e.is_timeout() {
                        warn!(
                            "Account #{} request timed out (attempt {}), retrying...",
                            session.user_id(),
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
        session: &AccountSession,
        path: &str,
        params: Option<&HashMap<String, String>>,
    ) -> Result<Response, AppError> {
        self.call_api::<()>(session, "GET", path, None, params)
            .await
    }

    pub async fn post<T: serde::Serialize>(
        &self,
        session: &AccountSession,
        path: &str,
        data: Option<&T>,
        params: Option<&HashMap<String, String>>,
    ) -> Result<Response, AppError> {
        self.call_api(session, "POST", path, data, params).await
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

    pub async fn login(&self, session: &AccountSession) -> Result<LoginResponse, AppError> {
        let payload = session.dump_account()?;
        let encrypted = self.cryptor.pack_bytes(&payload)?;
        let (url, method) = if self.region.is_cp_server() {
            let url = format!(
                "{}/api/user/{}/auth?refreshUpdatedResources=False",
                self.config.api_url,
                session.user_id()
            );
            (url, reqwest::Method::PUT)
        } else {
            let url = format!("{}/api/user/auth", self.config.api_url);
            (url, reqwest::Method::POST)
        };
        let mut req = self.prepare_request(session, method, &url);
        req = req.body(encrypted);
        info!("Account #{} logging in...", session.user_id());
        let resp = req
            .send()
            .await
            .map_err(|e| AppError::NetworkError(e.to_string()))?;
        self.update_session_token(session, &resp);
        let login_resp: LoginResponse = self.handle_response(resp).await?;
        if !login_resp.session_token.is_empty() {
            session.set_session_token(Some(login_resp.session_token.clone()));
        }
        if !self.region.is_cp_server() {
            if let Some(ref user_reg) = login_resp.user_registration {
                if !user_reg.user_id.is_empty() && user_reg.user_id != "0" {
                    let old_uid = session.user_id();
                    session.set_user_id(user_reg.user_id.clone());
                    info!(
                        "Account #{} -> {} (from login response)",
                        old_uid, user_reg.user_id
                    );
                }
            }
        }
        info!("Account #{} logged in successfully", session.user_id());
        Ok(login_resp)
    }

    #[tracing::instrument(skip(self, params), fields(region = ?self.region))]
    pub async fn get_game_api(
        &self,
        path: &str,
        params: Option<&HashMap<String, String>>,
    ) -> Result<(JsonValue, u16), AppError> {
        while self.reload_in_progress.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let session = self.get_session().ok_or(AppError::NoClientAvailable)?;
        let max_retries = 4;
        let mut retry_count = 0;
        while retry_count < max_retries {
            let resp = self.get(&session, path, params).await?;
            match self.handle_response_ordered(resp).await {
                Ok(result) => {
                    let json_value: JsonValue = serde_json::to_value(&result)
                        .map_err(|e| AppError::ParseError(e.to_string()))?;
                    return Ok((json_value, 200));
                }
                Err(AppError::SessionError) => {
                    warn!(
                        "{} Session expired, re-logging in...",
                        self.region.as_str().to_uppercase()
                    );
                    if let Err(e) = self.login(&session).await {
                        error!(
                            "{} Re-login failed: {}",
                            self.region.as_str().to_uppercase(),
                            e
                        );
                        return Err(AppError::SessionError);
                    }
                    retry_count += 1;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(AppError::CookieExpired) => {
                    if self.config.require_cookies {
                        warn!(
                            "{} Cookies expired, refreshing...",
                            self.region.as_str().to_uppercase()
                        );
                        self.refresh_cookies().await?;
                        retry_count += 1;
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    } else {
                        return Err(AppError::CookieExpired);
                    }
                }
                Err(AppError::UpgradeRequired) => {
                    warn!(
                        "{} Server upgrade required, refreshing version...",
                        self.region.as_str().to_uppercase()
                    );
                    self.refresh_version().await?;
                    retry_count += 1;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(AppError::UnderMaintenance) => {
                    return Err(AppError::UnderMaintenance);
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
        Err(AppError::NetworkError(
            "Max retry attempts reached".to_string(),
        ))
    }

    pub async fn get_cp_mysekai_image(&self, path: &str) -> Result<Vec<u8>, AppError> {
        let session = self.get_session().ok_or(AppError::NoClientAvailable)?;
        let path_clean = path.trim_start_matches('/');
        let image_url = format!("{}/image/mysekai-photo/{}", self.config.api_url, path_clean);
        let req = self.prepare_request(&session, reqwest::Method::GET, &image_url);
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
        let session = self.get_session().ok_or(AppError::NoClientAvailable)?;
        let path = format!("/user/{}/mysekai/photo/{}", user_id, index);
        let resp = self.get(&session, &path, None).await?;
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
