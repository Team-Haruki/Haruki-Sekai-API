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
use super::nuverse_schema::NuverseSchemaStore;
use super::session::AccountSession;
use super::token_utils;

pub struct SekaiClient {
    pub region: ServerRegion,
    pub config: ServerConfig,
    pub cookie_helper: Option<Arc<CookieHelper>>,
    pub version_helper: Arc<VersionHelper>,
    pub nuverse_schema_store: Option<Arc<NuverseSchemaStore>>,
    pub proxy: Option<String>,
    pub cryptor: SekaiCryptor,
    pub headers: Arc<Mutex<HashMap<String, String>>>,
    pub http_client: Client,

    sessions: Arc<RwLock<Vec<Arc<AccountSession>>>>,
    session_index: AtomicUsize,
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
        let nuverse_schema_store =
            if region.is_cp_server() || config.nuverse_schema_bundle_path.is_empty() {
                None
            } else {
                let data = fs::read(&config.nuverse_schema_bundle_path).map_err(|e| {
                    AppError::IoError(format!(
                        "Failed to read nuverse schema bundle {}: {}",
                        config.nuverse_schema_bundle_path, e
                    ))
                })?;
                Some(Arc::new(NuverseSchemaStore::from_slice(&data)?))
            };
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
            nuverse_schema_store,
            proxy,
            cryptor,
            headers: Arc::new(Mutex::new(headers)),
            http_client,
            sessions: Arc::new(RwLock::new(Vec::new())),
            session_index: AtomicUsize::new(0),
        };
        Ok(client)
    }

    pub fn restore_nuverse_master(
        &self,
        body: &[u8],
    ) -> Result<IndexMap<String, serde_json::Value>, AppError> {
        if let Some(store) = &self.nuverse_schema_store {
            let msgpack = self.cryptor.decrypt_msgpack(body)?;
            store.restore_master_msgpack(&msgpack)
        } else {
            self.cryptor.unpack_ordered(body)
        }
    }

    pub fn restore_nuverse_api_response(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, AppError> {
        if let Some(store) = &self.nuverse_schema_store {
            store.restore_api_json(path, body)
        } else {
            Ok(body)
        }
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

    fn update_version_headers_from_login(&self, login: &LoginResponse) {
        let mut headers = self.headers.lock();
        if !login.data_version.is_empty() {
            headers.insert("X-Data-Version".to_string(), login.data_version.clone());
        }
        if !login.asset_version.is_empty() {
            headers.insert("X-Asset-Version".to_string(), login.asset_version.clone());
        }
        info!(
            "{} Updated version headers from login: dataVersion={}, assetVersion={}",
            self.region.as_str().to_uppercase(),
            login.data_version,
            login.asset_version
        );
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
        let region = self.region.as_str().to_uppercase();
        info!("{} Reloading accounts...", region);
        let accounts = self.parse_accounts()?;

        // Log every account in concurrently, building the new session set off to
        // the side. The existing sessions keep serving traffic the whole time, so
        // there is no empty-pool window and no need to park inbound requests.
        let login_futs = accounts.into_iter().filter_map(|account| {
            if self.region.is_cp_server() && account.user_id().is_empty() {
                warn!("{} Skipping account with empty user_id", region);
                return None;
            }
            let region = region.clone();
            Some(async move {
                let session = Arc::new(AccountSession::new(account));
                match self.login(&session).await {
                    Ok(_) => Some(session),
                    Err(e) => {
                        error!("{} Failed to login account: {}", region, e);
                        None
                    }
                }
            })
        });
        let new_sessions: Vec<Arc<AccountSession>> = futures::future::join_all(login_futs)
            .await
            .into_iter()
            .flatten()
            .collect();

        let count = new_sessions.len();
        {
            let mut sessions = self.sessions.write();
            *sessions = new_sessions;
            self.session_index.store(0, Ordering::SeqCst);
        }
        info!("{} Accounts reloaded, {} sessions active", region, count);
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
            let debounce_duration = Duration::from_secs(2);
            let is_account_change = |kind: &notify::EventKind| {
                use notify::EventKind;
                matches!(
                    kind,
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                )
            };
            // Trailing-edge debounce: when a batch of account files is uploaded,
            // block for the first relevant change, then keep draining events until
            // the directory has been quiet for `debounce_duration`, and reload ONCE
            // for the whole batch instead of once per file.
            loop {
                match rx.recv() {
                    Ok(Ok(event)) if is_account_change(&event.kind) => {
                        info!(
                            "{} Account file change detected: {:?}",
                            region_str, event.paths
                        );
                    }
                    Ok(Ok(_)) => continue,
                    Ok(Err(e)) => {
                        error!("{} File watcher error: {}", region_str, e);
                        continue;
                    }
                    Err(_) => break, // watcher dropped, channel closed
                }
                // Coalesce the rest of the burst until the directory goes quiet.
                loop {
                    match rx.recv_timeout(debounce_duration) {
                        Ok(_) => continue,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
                info!(
                    "{} Account directory settled, reloading accounts once",
                    region_str
                );
                let client_clone = client.clone();
                rt.block_on(async {
                    if let Err(e) = client_clone.reload_accounts().await {
                        error!("{} Failed to reload accounts: {}", region_str, e);
                    }
                });
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
        let len = sessions.len();
        if len == 0 {
            return None;
        }
        let start = self.session_index.fetch_add(1, Ordering::Relaxed) % len;
        // Prefer an idle account: because the rolling one-time token forces each
        // account's calls to serialize (api_lock held across the request), blind
        // round-robin can queue a request behind an account that is mid-call or
        // retrying while others sit idle. Scan from the round-robin cursor and pick
        // the first account whose lock is free; fall back to the cursor slot if all
        // are busy. This is only a hint (the guard is dropped immediately and the
        // real lock is taken later), so there is no deadlock and the TOCTOU window
        // is negligible — there is no .await between here and lock acquisition.
        for i in 0..len {
            let idx = (start + i) % len;
            if sessions[idx].try_reserve() {
                return Some(sessions[idx].clone());
            }
        }
        Some(sessions[start].clone())
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
        self.call_api_with_timeout(session, method, path, data, params, None)
            .await
    }

    async fn call_api_with_timeout<T: serde::Serialize>(
        &self,
        session: &AccountSession,
        method: &str,
        path: &str,
        data: Option<&T>,
        params: Option<&HashMap<String, String>>,
        request_timeout: Option<Duration>,
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
            if let Some(timeout) = request_timeout {
                req = req.timeout(timeout);
            }
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

    pub async fn get_with_timeout(
        &self,
        session: &AccountSession,
        path: &str,
        params: Option<&HashMap<String, String>>,
        timeout: Duration,
    ) -> Result<Response, AppError> {
        self.call_api_with_timeout::<()>(session, "GET", path, None, params, Some(timeout))
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

    /// Read an octet-stream game response: classify the Sekai HTTP status, then
    /// decode the encrypted body with `decode`. Shared by the typed, ordered,
    /// and value response handlers so status/error classification lives once.
    async fn handle_octet_response<R>(
        &self,
        resp: Response,
        decode: impl FnOnce(&[u8]) -> Result<R, AppError>,
    ) -> Result<(R, u16), AppError> {
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        let content_encoding = resp
            .headers()
            .get("content-encoding")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body = resp
            .bytes()
            .await
            .map_err(|e| {
                let is_timeout = e.is_timeout();
                AppError::NetworkError(format!(
                    "failed to read response body (status={}, content-type={}, content-encoding={}, timeout={}): {}",
                    status, content_type, content_encoding, is_timeout, e
                ))
            })?;

        if content_type.contains("octet-stream") || content_type.contains("binary") {
            let sekai_status = SekaiHttpStatus::from_code(status)?;
            match sekai_status {
                SekaiHttpStatus::Ok
                | SekaiHttpStatus::ClientError
                | SekaiHttpStatus::NotFound
                | SekaiHttpStatus::Conflict => Ok((decode(&body)?, status)),
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

    pub async fn handle_response<T: DeserializeOwned>(
        &self,
        resp: Response,
    ) -> Result<T, AppError> {
        self.handle_octet_response(resp, |body| self.cryptor.unpack::<T>(body))
            .await
            .map(|(value, _status)| value)
    }

    pub async fn handle_response_ordered(
        &self,
        resp: Response,
    ) -> Result<(IndexMap<String, serde_json::Value>, u16), AppError> {
        self.handle_octet_response(resp, |body| self.cryptor.unpack_ordered(body))
            .await
    }

    /// Like `handle_response_ordered` but yields a `serde_json::Value` directly,
    /// avoiding the `IndexMap -> Value` rebuild on the hot game-API path.
    pub async fn handle_response_value(
        &self,
        resp: Response,
    ) -> Result<(JsonValue, u16), AppError> {
        self.handle_octet_response(resp, |body| self.cryptor.unpack_value(body))
            .await
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
        let session = self.get_session().ok_or(AppError::NoClientAvailable)?;
        self.drive_game_api::<()>(&session, "GET", path, None, params, true)
            .await
    }

    /// Shared driver for game-API calls: sends the request via `call_api`, decodes
    /// the response, and runs the retry / single-flight relogin / version-refresh
    /// state machine. `restore` enables Nuverse array->dict restoration on the
    /// response (GET path only); POST callers pass `false`.
    async fn drive_game_api<T: serde::Serialize>(
        &self,
        session: &AccountSession,
        method: &str,
        path: &str,
        body: Option<&T>,
        params: Option<&HashMap<String, String>>,
        restore: bool,
    ) -> Result<(JsonValue, u16), AppError> {
        let max_retries = 4;
        let mut retry_count = 0;
        while retry_count < max_retries {
            let resp = self.call_api(session, method, path, body, params).await?;
            match self.handle_response_value(resp).await {
                Ok((mut json_value, upstream_status)) => {
                    if restore && !self.region.is_cp_server() {
                        json_value = self.restore_nuverse_api_response(path, json_value)?;
                    }
                    return Ok((json_value, upstream_status));
                }
                Err(AppError::SessionError) => {
                    warn!(
                        "{} Session expired, re-logging in...",
                        self.region.as_str().to_uppercase()
                    );
                    // Single-flight: only the first caller to notice the expired
                    // token re-logs in; others wait on login_lock, then see the
                    // refreshed token and skip straight to the retry.
                    let token_before = session.get_session_token();
                    let guard = session.lock_login().await;
                    if session.get_session_token() == token_before {
                        if let Err(e) = self.login(session).await {
                            error!(
                                "{} Re-login failed: {}",
                                self.region.as_str().to_uppercase(),
                                e
                            );
                            return Err(AppError::SessionError);
                        }
                    }
                    drop(guard);
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
                        "{} Server upgrade required, refreshing version and re-logging in...",
                        self.region.as_str().to_uppercase()
                    );
                    // First attempt: refresh version from file and try login
                    self.refresh_version().await?;
                    match self.login(session).await {
                        Ok(login_resp) => {
                            self.update_version_headers_from_login(&login_resp);
                        }
                        Err(AppError::UpgradeRequired) => {
                            warn!(
                                "{} Login returned 426, waiting for app version update...",
                                self.region.as_str().to_uppercase()
                            );
                            tokio::time::sleep(Duration::from_secs(10)).await;
                            self.refresh_version().await?;
                            match self.login(session).await {
                                Ok(login_resp) => {
                                    self.update_version_headers_from_login(&login_resp);
                                }
                                Err(e) => {
                                    error!(
                                        "{} Re-login after waiting for app update failed: {}",
                                        self.region.as_str().to_uppercase(),
                                        e
                                    );
                                    return Err(AppError::UpgradeRequired);
                                }
                            }
                        }
                        Err(e) => {
                            error!(
                                "{} Re-login after version refresh failed: {}",
                                self.region.as_str().to_uppercase(),
                                e
                            );
                            return Err(AppError::UpgradeRequired);
                        }
                    }
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

    #[tracing::instrument(skip(self, body, params), fields(region = ?self.region))]
    pub async fn post_game_api_body<T: serde::Serialize>(
        &self,
        path: &str,
        body: &T,
        params: Option<&HashMap<String, String>>,
    ) -> Result<(JsonValue, u16), AppError> {
        let session = self.get_session().ok_or(AppError::NoClientAvailable)?;
        self.drive_game_api(&session, "POST", path, Some(body), params, false)
            .await
    }

    async fn get_cp_image(&self, relative_path: &str) -> Result<Vec<u8>, AppError> {
        let session = self.get_session().ok_or(AppError::NoClientAvailable)?;
        let path_clean = relative_path.trim_start_matches('/');
        let image_url = format!("{}/{}", self.config.api_url, path_clean);
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

    pub async fn get_cp_mysekai_image(&self, path: &str) -> Result<Vec<u8>, AppError> {
        self.get_cp_image(&format!(
            "image/mysekai-photo/{}",
            path.trim_start_matches('/')
        ))
        .await
    }

    pub async fn get_cp_custom_profile_card_thumbnail(
        &self,
        path: &str,
    ) -> Result<Vec<u8>, AppError> {
        self.get_cp_image(&format!(
            "image/custom-profile-card/thumbnail/{}",
            path.trim_start_matches('/')
        ))
        .await
    }

    pub async fn get_cp_custom_music_score(&self, path: &str) -> Result<Vec<u8>, AppError> {
        self.get_cp_image(&format!(
            "blob/custom-music-score/full/{}",
            path.trim_start_matches('/')
        ))
        .await
    }

    pub async fn get_cp_mysekai_housing_competition_thumbnail(
        &self,
        path: &str,
    ) -> Result<Vec<u8>, AppError> {
        self.get_cp_image(&format!(
            "image/mysekai-housing-competition/thumbnail/{}",
            path.trim_start_matches('/')
        ))
        .await
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
