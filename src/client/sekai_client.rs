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
use super::helper::{effective_app_version, CookieHelper, VersionHelper, VersionInfo};
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
    /// Serializes cookie refresh (client-wide state) so concurrent requests that
    /// all observe expired cookies do not each hit the cookie service.
    cookie_lock: tokio::sync::Mutex<()>,
}

impl SekaiClient {
    /// Build the shared reqwest client. Every region uses identical settings (only
    /// the global proxy varies), so one client is built once in init_app_state and
    /// cloned into each SekaiClient (reqwest::Client is Arc-internal; clones share
    /// the connection pool / TLS config).
    pub fn build_http_client(proxy: Option<&str>) -> Result<Client, AppError> {
        let mut builder = Client::builder()
            .timeout(Duration::from_secs(45))
            .pool_max_idle_per_host(20)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60));
        if let Some(proxy_url) = proxy {
            if !proxy_url.is_empty() {
                builder =
                    builder
                        .proxy(reqwest::Proxy::all(proxy_url).map_err(|e| {
                            AppError::NetworkError(format!("Invalid proxy: {}", e))
                        })?);
            }
        }
        builder
            .build()
            .map_err(|e| AppError::NetworkError(e.to_string()))
    }

    /// Load a Nuverse schema bundle from disk. Call once per distinct bundle path
    /// in init_app_state and share the resulting Arc across regions on that path.
    pub fn load_nuverse_schema_store(path: &str) -> Result<NuverseSchemaStore, AppError> {
        let data = fs::read(path).map_err(|e| {
            AppError::IoError(format!(
                "Failed to read nuverse schema bundle {}: {}",
                path, e
            ))
        })?;
        NuverseSchemaStore::from_slice(&data)
    }

    pub async fn new(
        region: ServerRegion,
        config: ServerConfig,
        proxy: Option<String>,
        jp_cookie_url: Option<String>,
        http_client: Client,
        nuverse_schema_store: Option<Arc<NuverseSchemaStore>>,
    ) -> Result<Self, AppError> {
        let cryptor = SekaiCryptor::from_hex(&config.aes_key_hex, &config.aes_iv_hex)?;
        let mut headers = HashMap::new();
        for (k, v) in &config.headers {
            headers.insert(k.clone(), v.clone());
        }
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
            nuverse_schema_store,
            proxy,
            cryptor,
            headers: Arc::new(Mutex::new(headers)),
            http_client,
            sessions: Arc::new(RwLock::new(Vec::new())),
            session_index: AtomicUsize::new(0),
            cookie_lock: tokio::sync::Mutex::new(()),
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
            // A failed initial fetch must not abort init: initialize_clients
            // drops a client whose init errored for the process lifetime, which
            // turns a transient cookie-service outage at startup into a
            // permanent one. Start without cookies instead; requests heal
            // through the CookieExpired recovery path once the service is
            // reachable again.
            match helper.get_cookies(self.proxy.as_deref()).await {
                Ok(cookie) => {
                    self.headers.lock().insert("Cookie".to_string(), cookie);
                }
                Err(e) => warn!(
                    "{} Initial cookie fetch failed ({}); continuing without cookies",
                    self.region.as_str().to_uppercase(),
                    e
                ),
            }
        }
        let mut version = self.version_helper.load().await?;
        self.normalize_app_version(&mut version);
        self.version_helper.update(version.clone());
        self.update_version_headers(&version);
        self.reload_accounts().await
    }

    fn normalize_app_version(&self, version: &mut VersionInfo) {
        let effective = effective_app_version(self.region, &version.app_version);
        if effective != version.app_version {
            info!(
                "{} Normalized appVersion {} to {}",
                self.region.as_str().to_uppercase(),
                version.app_version,
                effective
            );
            version.app_version = effective;
        }
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
        let mut version = self.version_helper.load().await?;
        self.normalize_app_version(&mut version);
        self.version_helper.update(version.clone());
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
        let (accounts, json_file_count) = self.parse_accounts()?;
        if accounts.is_empty() {
            if json_file_count > 0 {
                // Account files exist but none parsed — likely a transient state
                // (mid-upload, bad deploy). Keep serving with the existing pool
                // instead of swapping in an empty one.
                let existing = self.sessions.read().len();
                error!(
                    "{} All {} account file(s) in {} failed to parse; keeping {} existing session(s)",
                    region, json_file_count, self.config.account_dir, existing
                );
                return Ok(());
            }
            warn!(
                "{} No accounts found in {}",
                region, self.config.account_dir
            );
        }

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
                    Err(e) if login_failure_is_transient(&e) => {
                        // The failure indicts the route to the game server
                        // (line down, maintenance, stale version/cookies), not
                        // the account. Keep the session without a token: its
                        // first request relogins through the SessionError
                        // recovery path, so the pool heals itself when the
                        // upstream comes back instead of staying empty until
                        // the next account-file change or a restart.
                        warn!(
                            "{} Account #{} login failed transiently ({}), keeping for lazy relogin",
                            region,
                            session.user_id(),
                            e
                        );
                        Some(session)
                    }
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
        // Run reloads on the main multi-threaded runtime via its Handle, instead of
        // building a dedicated current-thread runtime per region (which left 5 idle
        // runtimes alive for the process lifetime).
        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            watch_account_directory(watcher, rx, client, handle, region_str, account_dir);
        });
        Ok(())
    }

    /// Parse every account JSON file in the account directory. Returns the
    /// parsed accounts and the number of `.json` files seen, so callers can
    /// distinguish "directory is empty" from "every file failed to parse".
    fn parse_accounts(&self) -> Result<(Vec<AccountType>, usize), AppError> {
        let mut accounts = Vec::new();
        let mut json_file_count = 0usize;
        let account_dir = Path::new(&self.config.account_dir);
        if !account_dir.exists() {
            return Ok((accounts, json_file_count));
        }
        let entries = fs::read_dir(account_dir)
            .map_err(|e| AppError::ParseError(format!("Failed to read account dir: {}", e)))?;
        // Propagate entry errors instead of flattening them away: a transiently
        // unreadable directory must abort the reload, not read as "no accounts"
        // (which would clear the live session pool).
        for entry in entries {
            let entry = entry.map_err(|e| {
                AppError::ParseError(format!("Failed to read account entry: {}", e))
            })?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            json_file_count += 1;
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
        Ok((accounts, json_file_count))
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
            parse_cp_account(value, &log_prefix)
        } else {
            parse_nuverse_account(value, &log_prefix)
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
                // Log only rotation metadata — never token contents.
                debug!(
                    "{} Account #{} session token updated (had_previous: {})",
                    self.region.as_str().to_uppercase(),
                    session.user_id(),
                    old_token.is_some()
                );
            }
        }
    }

    pub async fn call_api<T: serde::Serialize>(
        &self,
        session: &AccountSession,
        method: reqwest::Method,
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
        method: reqwest::Method,
        path: &str,
        data: Option<&T>,
        params: Option<&HashMap<String, String>>,
        request_timeout: Option<Duration>,
    ) -> Result<Response, AppError> {
        let _lock = session.lock_api().await;
        let user_id = session.user_id();
        let url = format!("{}/api{}", self.config.api_url, path).replace("{userId}", &user_id);
        info!("Account #{} {} {}", user_id, method, path);
        // Only idempotent requests are retried at the network layer: a timed-out
        // POST/PUT may already have been executed server-side, so resending it
        // risks double-executing the game action.
        let max_attempts = if method == reqwest::Method::GET { 2 } else { 1 };
        let packed = match data {
            Some(body_data) => Some(self.cryptor.pack(body_data)?),
            None => None,
        };
        let mut last_error = None;
        for attempt in 1..=max_attempts {
            let request = self.build_api_request(
                session,
                method.clone(),
                &url,
                params,
                packed.as_ref(),
                request_timeout,
            );
            match request.send().await {
                Ok(resp) => {
                    self.update_session_token(session, &resp);
                    return Ok(resp);
                }
                Err(e) => {
                    self.log_request_error(&e, &user_id, attempt, max_attempts);
                    last_error = Some(AppError::NetworkError(e.to_string()));
                }
            }
            if attempt < max_attempts {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        Err(last_error.unwrap_or(AppError::NetworkError(
            "Request failed after retries".to_string(),
        )))
    }

    fn build_api_request(
        &self,
        session: &AccountSession,
        method: reqwest::Method,
        url: &str,
        params: Option<&HashMap<String, String>>,
        body: Option<&Vec<u8>>,
        timeout: Option<Duration>,
    ) -> reqwest::RequestBuilder {
        let mut request = self.prepare_request(session, method, url);
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        if let Some(params) = params {
            request = request.query(params);
        }
        if let Some(body) = body {
            request = request.body(body.clone());
        }
        request
    }

    fn log_request_error(
        &self,
        error_value: &reqwest::Error,
        user_id: &str,
        attempt: usize,
        max_attempts: usize,
    ) {
        let region = self.region.as_str().to_uppercase();
        if error_value.is_timeout() {
            warn!(
                "{} Account #{} request timed out (attempt {}/{})",
                region, user_id, attempt, max_attempts
            );
        } else {
            error!(
                "{} Request error (attempt {}/{}): {}",
                region, attempt, max_attempts, error_value
            );
        }
    }

    pub async fn get(
        &self,
        session: &AccountSession,
        path: &str,
        params: Option<&HashMap<String, String>>,
    ) -> Result<Response, AppError> {
        self.call_api::<()>(session, reqwest::Method::GET, path, None, params)
            .await
    }

    pub async fn get_with_timeout(
        &self,
        session: &AccountSession,
        path: &str,
        params: Option<&HashMap<String, String>>,
        timeout: Duration,
    ) -> Result<Response, AppError> {
        self.call_api_with_timeout::<()>(
            session,
            reqwest::Method::GET,
            path,
            None,
            params,
            Some(timeout),
        )
        .await
    }

    pub async fn post<T: serde::Serialize>(
        &self,
        session: &AccountSession,
        path: &str,
        data: Option<&T>,
        params: Option<&HashMap<String, String>>,
    ) -> Result<Response, AppError> {
        self.call_api(session, reqwest::Method::POST, path, data, params)
            .await
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
        self.drive_game_api::<()>(&session, reqwest::Method::GET, path, None, params, true)
            .await
    }

    /// Shared driver for game-API calls: sends the request via `call_api`, decodes
    /// the response, and runs the retry / single-flight relogin / version-refresh
    /// state machine. `restore` enables Nuverse array->dict restoration on the
    /// response (GET path only); POST callers pass `false`.
    async fn drive_game_api<T: serde::Serialize>(
        &self,
        session: &AccountSession,
        method: reqwest::Method,
        path: &str,
        body: Option<&T>,
        params: Option<&HashMap<String, String>>,
        restore: bool,
    ) -> Result<(JsonValue, u16), AppError> {
        let max_retries = 4;
        let mut retry_count = 0;
        while retry_count < max_retries {
            let resp = self
                .call_api(session, method.clone(), path, body, params)
                .await?;
            match self.handle_response_value(resp).await {
                Ok((mut json_value, upstream_status)) => {
                    if restore && !self.region.is_cp_server() {
                        json_value = self.restore_nuverse_api_response(path, json_value)?;
                    }
                    return Ok((json_value, upstream_status));
                }
                Err(AppError::SessionError) => self.recover_session(session).await?,
                Err(AppError::CookieExpired) => self.recover_cookies().await?,
                Err(AppError::UpgradeRequired) => self.recover_upgrade(session).await?,
                Err(AppError::UnderMaintenance) => {
                    return Err(AppError::UnderMaintenance);
                }
                Err(e) => {
                    return Err(e);
                }
            }
            retry_count += 1;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Err(AppError::Internal(format!(
            "Game API call gave up after {} session recovery attempts",
            max_retries
        )))
    }

    async fn recover_session(&self, session: &AccountSession) -> Result<(), AppError> {
        warn!(
            "{} Session expired, re-logging in...",
            self.region.as_str().to_uppercase()
        );
        let token_before = session.get_session_token();
        let _guard = session.lock_login().await;
        if session.get_session_token() != token_before {
            return Ok(());
        }
        self.login(session).await.map(|_| ()).map_err(|e| {
            error!(
                "{} Re-login failed: {}",
                self.region.as_str().to_uppercase(),
                e
            );
            AppError::SessionError
        })
    }

    async fn recover_cookies(&self) -> Result<(), AppError> {
        if !self.config.require_cookies {
            return Err(AppError::CookieExpired);
        }
        warn!(
            "{} Cookies expired, refreshing...",
            self.region.as_str().to_uppercase()
        );
        let cookie_before = self.headers.lock().get("Cookie").cloned();
        let _guard = self.cookie_lock.lock().await;
        if self.headers.lock().get("Cookie").cloned() == cookie_before {
            self.refresh_cookies().await?;
        }
        Ok(())
    }

    async fn recover_upgrade(&self, session: &AccountSession) -> Result<(), AppError> {
        warn!(
            "{} Server upgrade required, refreshing version and re-logging in...",
            self.region.as_str().to_uppercase()
        );
        let token_before = session.get_session_token();
        let _guard = session.lock_login().await;
        if session.get_session_token() != token_before {
            return Ok(());
        }
        self.refresh_version().await?;
        match self.login(session).await {
            Ok(login) => self.update_version_headers_from_login(&login),
            Err(AppError::UpgradeRequired) => self.retry_upgrade_login(session).await?,
            Err(e) => {
                error!(
                    "{} Re-login after version refresh failed: {}",
                    self.region.as_str().to_uppercase(),
                    e
                );
                return Err(AppError::UpgradeRequired);
            }
        }
        Ok(())
    }

    async fn retry_upgrade_login(&self, session: &AccountSession) -> Result<(), AppError> {
        warn!(
            "{} Login returned 426, waiting for app version update...",
            self.region.as_str().to_uppercase()
        );
        tokio::time::sleep(Duration::from_secs(10)).await;
        self.refresh_version().await?;
        self.login(session)
            .await
            .map(|login| self.update_version_headers_from_login(&login))
            .map_err(|e| {
                error!(
                    "{} Re-login after waiting for app update failed: {}",
                    self.region.as_str().to_uppercase(),
                    e
                );
                AppError::UpgradeRequired
            })
    }

    #[tracing::instrument(skip(self, body, params), fields(region = ?self.region))]
    pub async fn post_game_api_body<T: serde::Serialize>(
        &self,
        path: &str,
        body: &T,
        params: Option<&HashMap<String, String>>,
    ) -> Result<(JsonValue, u16), AppError> {
        let session = self.get_session().ok_or(AppError::NoClientAvailable)?;
        self.drive_game_api(
            &session,
            reqwest::Method::POST,
            path,
            Some(body),
            params,
            false,
        )
        .await
    }

    /// Authenticated game GET that returns the raw [`Response`] with its
    /// encrypted body untouched, for relaying to a peer node as a byte stream.
    /// Runs a status-code-only version of `drive_game_api`'s recovery (relogin
    /// on 403, cookie refresh on XML-403, version refresh + relogin on 426) —
    /// the body is never read on the success path, so classification that
    /// requires decoding is out of scope; game-level statuses that the decode
    /// path accepts (200/400/404/409 octet-stream) are returned as-is.
    pub async fn get_game_api_raw(
        &self,
        path: &str,
        timeout: Duration,
    ) -> Result<Response, AppError> {
        let session = self.get_session().ok_or(AppError::NoClientAvailable)?;
        let max_retries = 4;
        let mut retry_count = 0;
        while retry_count < max_retries {
            let resp = self
                .call_api_with_timeout::<()>(
                    &session,
                    reqwest::Method::GET,
                    path,
                    None,
                    None,
                    Some(timeout),
                )
                .await?;
            match self.classify_raw_response(resp).await? {
                RawResponse::Success(resp) => return Ok(resp),
                RawResponse::Recover { error, status } => {
                    self.recover_raw_response(&session, error, status).await?;
                }
            }
            retry_count += 1;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Err(AppError::Internal(format!(
            "Raw game GET gave up after {} recovery attempts",
            max_retries
        )))
    }

    async fn classify_raw_response(&self, resp: Response) -> Result<RawResponse, AppError> {
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|header| header.to_str().ok())
            .unwrap_or_default()
            .to_lowercase();
        if content_type.contains("octet-stream") || content_type.contains("binary") {
            return match SekaiHttpStatus::from_code(status)? {
                SekaiHttpStatus::Ok
                | SekaiHttpStatus::ClientError
                | SekaiHttpStatus::NotFound
                | SekaiHttpStatus::Conflict => Ok(RawResponse::Success(resp)),
                SekaiHttpStatus::SessionError => {
                    Ok(RawResponse::recover(AppError::SessionError, status))
                }
                SekaiHttpStatus::GameUpgrade => {
                    Ok(RawResponse::recover(AppError::UpgradeRequired, status))
                }
                SekaiHttpStatus::UnderMaintenance => Err(AppError::UnderMaintenance),
                _ => Err(AppError::Unknown {
                    status,
                    body: String::new(),
                }),
            };
        }
        if status == 403 && content_type.contains("xml") {
            return Ok(RawResponse::recover(AppError::CookieExpired, status));
        }
        if status == 503 {
            return Err(AppError::UnderMaintenance);
        }
        let body = resp.bytes().await.unwrap_or_default();
        Err(AppError::Unknown {
            status,
            body: String::from_utf8_lossy(&body).to_string(),
        })
    }

    async fn recover_raw_response(
        &self,
        session: &AccountSession,
        error_value: AppError,
        status: u16,
    ) -> Result<(), AppError> {
        match error_value {
            AppError::SessionError => self.recover_raw_session(session, false, status).await,
            AppError::UpgradeRequired => self.recover_raw_session(session, true, status).await,
            AppError::CookieExpired => self.recover_cookies().await,
            other => Err(other),
        }
    }

    async fn recover_raw_session(
        &self,
        session: &AccountSession,
        refresh_version: bool,
        status: u16,
    ) -> Result<(), AppError> {
        warn!(
            "{} Raw game GET hit {} — recovering session...",
            self.region.as_str().to_uppercase(),
            status
        );
        let token_before = session.get_session_token();
        let _guard = session.lock_login().await;
        if session.get_session_token() != token_before {
            return Ok(());
        }
        if refresh_version {
            self.refresh_version().await?;
        }
        self.login(session).await.map(|_| ()).map_err(|e| {
            error!(
                "{} Re-login during raw GET failed: {}",
                self.region.as_str().to_uppercase(),
                e
            );
            if refresh_version {
                AppError::UpgradeRequired
            } else {
                AppError::SessionError
            }
        })
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

enum RawResponse {
    Success(Response),
    Recover { error: AppError, status: u16 },
}

impl RawResponse {
    fn recover(error: AppError, status: u16) -> Self {
        Self::Recover { error, status }
    }
}

/// Whether a login failure during an account reload indicts the path to the
/// game server rather than the account itself. Transient failures keep the
/// account in the session pool for lazy relogin on first use; anything else
/// (403s, malformed responses) is treated as bad account data and dropped,
/// since a dead account kept in rotation would fail requests that healthy
/// accounts could serve.
fn login_failure_is_transient(e: &AppError) -> bool {
    matches!(
        e,
        AppError::NetworkError(_)
            | AppError::InvalidHttpStatus(_)
            | AppError::UnderMaintenance
            | AppError::UpgradeRequired
            | AppError::CookieExpired
            | AppError::IoError(_)
            | AppError::Internal(_)
    )
}

fn watch_account_directory(
    _watcher: notify::PollWatcher,
    events: std::sync::mpsc::Receiver<notify::Result<notify::Event>>,
    client: Arc<SekaiClient>,
    handle: tokio::runtime::Handle,
    region: String,
    account_dir: String,
) {
    info!(
        "{} File watcher started for {} (polling mode, 5s interval)",
        region, account_dir
    );
    while wait_for_account_change(&events, &region) {
        if !wait_for_quiet_directory(&events, Duration::from_secs(2)) {
            return;
        }
        info!(
            "{} Account directory settled, reloading accounts once",
            region
        );
        let client = client.clone();
        handle.block_on(async {
            if let Err(e) = client.reload_accounts().await {
                error!("{} Failed to reload accounts: {}", region, e);
            }
        });
    }
}

fn wait_for_account_change(
    events: &std::sync::mpsc::Receiver<notify::Result<notify::Event>>,
    region: &str,
) -> bool {
    loop {
        match events.recv() {
            Ok(Ok(event)) if is_account_change(&event.kind) => {
                info!("{} Account file change detected: {:?}", region, event.paths);
                return true;
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => error!("{} File watcher error: {}", region, e),
            Err(_) => return false,
        }
    }
}

fn is_account_change(kind: &notify::EventKind) -> bool {
    use notify::EventKind;
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

fn wait_for_quiet_directory(
    events: &std::sync::mpsc::Receiver<notify::Result<notify::Event>>,
    debounce: Duration,
) -> bool {
    loop {
        match events.recv_timeout(debounce) {
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return true,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
}

fn parse_cp_account(value: JsonValue, log_prefix: &str) -> Option<AccountType> {
    let json = serde_json::to_string(&value).ok()?;
    let mut account = match sonic_rs::from_str::<SekaiAccountCP>(&json) {
        Ok(account) => account,
        Err(e) => {
            warn!("{} CP unmarshal error: {}", log_prefix, e);
            return None;
        }
    };
    if let Ok(user_id) = token_utils::extract_user_id_from_jwt(&account.credential) {
        debug!("{} Extracted user_id from JWT: {}", log_prefix, user_id);
        account.user_id = user_id;
    } else if account.user_id.is_empty() {
        warn!(
            "{} Failed to extract user_id from JWT and no fallback",
            log_prefix
        );
    }
    Some(AccountType::CP(account))
}

fn parse_nuverse_account(value: JsonValue, log_prefix: &str) -> Option<AccountType> {
    let json = serde_json::to_string(&value).ok()?;
    let mut account = match sonic_rs::from_str::<SekaiAccountNuverse>(&json) {
        Ok(account) => account,
        Err(e) => {
            warn!("{} Nuverse unmarshal error: {}", log_prefix, e);
            return None;
        }
    };
    if let Ok(user_id) = token_utils::extract_user_id_from_nuverse_token(&account.access_token) {
        debug!(
            "{} Extracted user_id from Nuverse token: {}",
            log_prefix, user_id
        );
        account.user_id = user_id;
    } else if account.user_id.is_empty() || account.user_id == "0" {
        warn!(
            "{} Failed to extract user_id from Nuverse token and no fallback",
            log_prefix
        );
    }
    Some(AccountType::Nuverse(account))
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::Body;
    use axum::extract::State;
    use axum::http::{HeaderValue, Response as AxumResponse, StatusCode};
    use axum::Router;
    use notify::event::{CreateKind, ModifyKind, RemoveKind};
    use serde_json::json;

    use super::*;

    const KEY: &str = "00112233445566778899aabbccddeeff";
    const IV: &str = "ffeeddccbbaa99887766554433221100";

    #[derive(Clone)]
    struct StaticResponse {
        status: u16,
        content_type: &'static str,
        body: Vec<u8>,
        session_token: Option<&'static str>,
    }

    async fn static_handler(State(response): State<StaticResponse>) -> AxumResponse<Body> {
        let mut builder = AxumResponse::builder()
            .status(response.status)
            .header("content-type", response.content_type);
        if let Some(token) = response.session_token {
            builder = builder.header("x-session-token", token);
        }
        builder.body(Body::from(response.body)).unwrap()
    }

    async fn spawn_server(response: StaticResponse) -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().fallback(static_handler).with_state(response);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{address}"), task)
    }

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("haruki_{label}_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn config(api_url: &str, root: &Path) -> ServerConfig {
        let mut config: ServerConfig = serde_yaml::from_str("{}").unwrap();
        config.api_url = api_url.to_string();
        config.account_dir = root.join("accounts").to_string_lossy().into_owned();
        config.version_path = root.join("version.json").to_string_lossy().into_owned();
        config.aes_key_hex = KEY.to_string();
        config.aes_iv_hex = IV.to_string();
        config
            .headers
            .insert("X-Test".to_string(), "yes".to_string());
        std::fs::create_dir_all(&config.account_dir).unwrap();
        std::fs::write(
            &config.version_path,
            r#"{"appVersion":"6.0.2","appHash":"hash","dataVersion":"data","assetVersion":"asset"}"#,
        )
        .unwrap();
        config
    }

    async fn make_client(region: ServerRegion, api_url: &str) -> (SekaiClient, std::path::PathBuf) {
        let root = temp_dir("client");
        let client = SekaiClient::new(
            region,
            config(api_url, &root),
            None,
            None,
            SekaiClient::build_http_client(None).unwrap(),
            None,
        )
        .await
        .unwrap();
        (client, root)
    }

    fn session(region: ServerRegion, user_id: &str) -> Arc<AccountSession> {
        let account = if region.is_cp_server() {
            AccountType::CP(SekaiAccountCP {
                user_id: user_id.to_string(),
                device_id: "device".to_string(),
                credential: "credential".to_string(),
            })
        } else {
            AccountType::Nuverse(SekaiAccountNuverse {
                user_id: user_id.to_string(),
                device_id: "device".to_string(),
                access_token: "access".to_string(),
            })
        };
        Arc::new(AccountSession::new(account))
    }

    fn encrypted(value: serde_json::Value) -> Vec<u8> {
        SekaiCryptor::from_hex(KEY, IV)
            .unwrap()
            .pack(&value)
            .unwrap()
    }

    #[tokio::test]
    async fn constructs_initializes_and_refreshes_client_state() {
        assert!(SekaiClient::build_http_client(Some("://bad proxy")).is_err());
        assert!(SekaiClient::load_nuverse_schema_store("/missing/schema.json").is_err());

        let (client, root) = make_client(ServerRegion::Cn, "http://127.0.0.1:1").await;
        assert_eq!(client.region, ServerRegion::Cn);
        assert_eq!(client.headers.lock().get("X-Test").unwrap(), "yes");
        client.init().await.unwrap();
        assert_eq!(client.version_helper.get().app_version, "6.0.0");
        assert_eq!(client.headers.lock().get("X-App-Version").unwrap(), "6.0.0");

        std::fs::write(
            &client.config.version_path,
            r#"{"appVersion":"6.1.9","appHash":"h2","dataVersion":"d2","assetVersion":"a2"}"#,
        )
        .unwrap();
        client.refresh_version().await.unwrap();
        assert_eq!(client.headers.lock().get("X-App-Version").unwrap(), "6.1.0");
        client.refresh_cookies().await.unwrap();
        Arc::new(client).start_file_watcher().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn parses_account_files_and_preserves_pool_on_all_invalid_files() {
        let (cp, cp_root) = make_client(ServerRegion::En, "http://127.0.0.1:1").await;
        std::fs::write(
            cp_root.join("accounts/accounts.json"),
            r#"[{"userId":"10","credential":"bad"},{"userId":11,"deviceId":"d","credential":"bad"},null]"#,
        )
        .unwrap();
        std::fs::write(cp_root.join("accounts/ignored.txt"), "ignored").unwrap();
        let (accounts, count) = cp.parse_accounts().unwrap();
        assert_eq!(count, 1);
        assert_eq!(accounts.len(), 2);
        assert!(matches!(accounts[0], AccountType::CP(_)));

        let (nuverse, nuverse_root) = make_client(ServerRegion::Tw, "http://127.0.0.1:1").await;
        std::fs::write(
            nuverse_root.join("accounts/account.json"),
            r#"{"userId":12,"deviceId":null,"accessToken":"bad"}"#,
        )
        .unwrap();
        let (accounts, count) = nuverse.parse_accounts().unwrap();
        assert_eq!(count, 1);
        assert_eq!(accounts.len(), 1);
        assert!(matches!(accounts[0], AccountType::Nuverse(_)));

        let retained = session(ServerRegion::Tw, "99");
        nuverse.sessions.write().push(retained.clone());
        std::fs::write(nuverse_root.join("accounts/account.json"), "invalid").unwrap();
        nuverse.reload_accounts().await.unwrap();
        assert_eq!(nuverse.get_session().unwrap().user_id(), "99");

        std::fs::remove_dir_all(cp_root).unwrap();
        std::fs::remove_dir_all(nuverse_root).unwrap();
    }

    #[test]
    fn classifies_login_failure_transience() {
        // Transient: the route to the game server is at fault, keep the account.
        for e in [
            AppError::NetworkError("refused".into()),
            AppError::InvalidHttpStatus(502),
            AppError::UnderMaintenance,
            AppError::UpgradeRequired,
            AppError::CookieExpired,
        ] {
            assert!(login_failure_is_transient(&e), "{e:?} should be kept");
        }
        // Account-level: likely bad credentials or data, drop as before.
        for e in [
            AppError::SessionError,
            AppError::ParseError("x".into()),
            AppError::Unknown {
                status: 400,
                body: String::new(),
            },
        ] {
            assert!(!login_failure_is_transient(&e), "{e:?} should be dropped");
        }
    }

    // A reload while the upstream is unreachable must not empty the session
    // pool: the account is kept without a token and relogins lazily, so the
    // node recovers on its own once the line is back instead of serving
    // NoClientAvailable until the next account-file change or a restart.
    #[tokio::test]
    async fn reload_keeps_accounts_when_login_fails_transiently() {
        let (client, root) = make_client(ServerRegion::Tw, "http://127.0.0.1:1").await;
        std::fs::write(
            root.join("accounts/account.json"),
            r#"{"userId":12,"deviceId":"d","accessToken":"bad"}"#,
        )
        .unwrap();
        client.reload_accounts().await.unwrap();
        let session = client.get_session().expect("session kept despite outage");
        assert_eq!(session.user_id(), "12");
        assert_eq!(session.get_session_token(), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    // An account the game itself rejects (403 -> SessionError) is still dropped:
    // keeping it in rotation would fail requests healthy accounts could serve.
    #[tokio::test]
    async fn reload_drops_accounts_rejected_by_the_game() {
        let (url, server) = spawn_server(StaticResponse {
            status: 403,
            content_type: "application/octet-stream",
            body: vec![0],
            session_token: None,
        })
        .await;
        let (client, root) = make_client(ServerRegion::En, &url).await;
        std::fs::write(
            root.join("accounts/account.json"),
            r#"{"userId":"10","deviceId":"d","credential":"bad"}"#,
        )
        .unwrap();
        client.reload_accounts().await.unwrap();
        assert!(client.get_session().is_none());
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    // A cookie-service outage at startup must not abort init: the client would
    // be dropped for the process lifetime. Cookies heal lazily instead.
    #[tokio::test]
    async fn init_survives_cookie_service_outage() {
        let root = temp_dir("cookie_outage");
        let mut server_config = config("http://127.0.0.1:1", &root);
        server_config.require_cookies = true;
        let client = SekaiClient::new(
            ServerRegion::Jp,
            server_config,
            None,
            Some("http://127.0.0.1:1".to_string()),
            SekaiClient::build_http_client(None).unwrap(),
            None,
        )
        .await
        .unwrap();
        assert!(client.cookie_helper.is_some());
        client.init().await.unwrap();
        assert!(!client.headers.lock().contains_key("Cookie"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn routes_sessions_and_builds_authenticated_requests() {
        let body = encrypted(json!({"ok": true}));
        let (url, server) = spawn_server(StaticResponse {
            status: 200,
            content_type: "application/octet-stream",
            body,
            session_token: Some("rotated"),
        })
        .await;
        let (client, root) = make_client(ServerRegion::En, &url).await;
        assert!(client.get_session().is_none());
        let first = session(ServerRegion::En, "1");
        let second = session(ServerRegion::En, "2");
        first.set_session_token(Some("old".to_string()));
        client.sessions.write().extend([first.clone(), second]);

        let mut params = HashMap::new();
        params.insert("q".to_string(), "value".to_string());
        let response = client
            .get_with_timeout(
                &first,
                "/user/{userId}/profile",
                Some(&params),
                Duration::from_secs(2),
            )
            .await
            .unwrap();
        let value: serde_json::Value = client.handle_response(response).await.unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(first.get_session_token().as_deref(), Some("rotated"));

        let response = client
            .post(&first, "/post", Some(&json!({"x": 1})), None)
            .await
            .unwrap();
        let (ordered, status) = client.handle_response_ordered(response).await.unwrap();
        assert_eq!(status, 200);
        assert_eq!(ordered["ok"], true);

        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn classifies_binary_and_text_responses() {
        let cases = [
            (403, "application/octet-stream", "session"),
            (426, "application/octet-stream", "upgrade"),
            (503, "application/octet-stream", "maintenance"),
            (500, "application/octet-stream", "unknown"),
            (403, "application/xml", "cookie"),
            (503, "text/plain", "maintenance"),
            (500, "text/plain", "unknown"),
            (418, "text/plain", "invalid"),
        ];

        for (status, content_type, expected) in cases {
            let (url, server) = spawn_server(StaticResponse {
                status,
                content_type,
                body: b"failure".to_vec(),
                session_token: None,
            })
            .await;
            let (client, root) = make_client(ServerRegion::En, &url).await;
            let response = client.http_client.get(&url).send().await.unwrap();
            let error = client
                .handle_response::<serde_json::Value>(response)
                .await
                .unwrap_err();
            match expected {
                "session" => assert!(matches!(error, AppError::SessionError)),
                "upgrade" => assert!(matches!(error, AppError::UpgradeRequired)),
                "maintenance" => assert!(matches!(error, AppError::UnderMaintenance)),
                "cookie" => assert!(matches!(error, AppError::CookieExpired)),
                "invalid" => assert!(matches!(error, AppError::InvalidHttpStatus(418))),
                _ => assert!(matches!(error, AppError::Unknown { .. })),
            }
            server.abort();
            std::fs::remove_dir_all(root).unwrap();
        }

        for status in [200, 400, 404, 409] {
            let (url, server) = spawn_server(StaticResponse {
                status,
                content_type: "application/octet-stream",
                body: encrypted(json!({"status": status})),
                session_token: None,
            })
            .await;
            let (client, root) = make_client(ServerRegion::En, &url).await;
            let response = client.http_client.get(&url).send().await.unwrap();
            let (value, actual) = client.handle_response_value(response).await.unwrap();
            assert_eq!(actual, status);
            assert_eq!(value["status"], status);
            server.abort();
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn logs_in_and_drives_cp_and_nuverse_calls() {
        let login = json!({
            "sessionToken": "login-token",
            "dataVersion": "d2",
            "assetVersion": "a2",
            "userRegistration": {"userId": 777}
        });
        let (login_url, login_server) = spawn_server(StaticResponse {
            status: 200,
            content_type: "application/octet-stream",
            body: encrypted(login),
            session_token: Some("header-token"),
        })
        .await;
        let (cp, cp_root) = make_client(ServerRegion::En, &login_url).await;
        let cp_session = session(ServerRegion::En, "12");
        let response = cp.login(&cp_session).await.unwrap();
        assert_eq!(response.data_version, "d2");
        assert_eq!(cp_session.user_id(), "12");
        assert_eq!(
            cp_session.get_session_token().as_deref(),
            Some("login-token")
        );

        let (nuverse, nuverse_root) = make_client(ServerRegion::Tw, &login_url).await;
        let nuverse_session = session(ServerRegion::Tw, "0");
        nuverse.login(&nuverse_session).await.unwrap();
        assert_eq!(nuverse_session.user_id(), "777");
        login_server.abort();

        let (game_url, game_server) = spawn_server(StaticResponse {
            status: 200,
            content_type: "application/octet-stream",
            body: encrypted(json!({"result": "ok"})),
            session_token: None,
        })
        .await;
        let (game, game_root) = make_client(ServerRegion::En, &game_url).await;
        game.sessions.write().push(session(ServerRegion::En, "1"));
        let (value, status) = game.get_game_api("/game", None).await.unwrap();
        assert_eq!(status, 200);
        assert_eq!(value["result"], "ok");
        let (value, status) = game
            .post_game_api_body("/game", &json!({"request": true}), None)
            .await
            .unwrap();
        assert_eq!(status, 200);
        assert_eq!(value["result"], "ok");
        let raw = game
            .get_game_api_raw("/raw", Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(raw.status(), StatusCode::OK);
        game_server.abort();

        for root in [cp_root, nuverse_root, game_root] {
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn fetches_cp_images_and_nuverse_thumbnail() {
        let (image_url, image_server) = spawn_server(StaticResponse {
            status: 200,
            content_type: "image/png",
            body: vec![1, 2, 3],
            session_token: None,
        })
        .await;
        let (client, root) = make_client(ServerRegion::En, &image_url).await;
        client.sessions.write().push(session(ServerRegion::En, "1"));
        assert_eq!(
            client.get_cp_mysekai_image("/a.png").await.unwrap(),
            vec![1, 2, 3]
        );
        assert_eq!(
            client
                .get_cp_custom_profile_card_thumbnail("b.png")
                .await
                .unwrap(),
            vec![1, 2, 3]
        );
        assert_eq!(
            client.get_cp_custom_music_score("c").await.unwrap(),
            vec![1, 2, 3]
        );
        assert_eq!(
            client
                .get_cp_mysekai_housing_competition_thumbnail("d")
                .await
                .unwrap(),
            vec![1, 2, 3]
        );
        image_server.abort();

        let (error_url, error_server) = spawn_server(StaticResponse {
            status: 404,
            content_type: "text/plain",
            body: Vec::new(),
            session_token: None,
        })
        .await;
        let (error_client, error_root) = make_client(ServerRegion::En, &error_url).await;
        error_client
            .sessions
            .write()
            .push(session(ServerRegion::En, "1"));
        assert!(matches!(
            error_client.get_cp_mysekai_image("x").await,
            Err(AppError::Unknown { status: 404, .. })
        ));
        error_server.abort();

        let (nuverse_url, nuverse_server) = spawn_server(StaticResponse {
            status: 200,
            content_type: "application/octet-stream",
            body: encrypted(json!({"thumbnail": "AQID"})),
            session_token: None,
        })
        .await;
        let (nuverse, nuverse_root) = make_client(ServerRegion::Tw, &nuverse_url).await;
        nuverse
            .sessions
            .write()
            .push(session(ServerRegion::Tw, "1"));
        assert_eq!(
            nuverse.get_nuverse_mysekai_image("1", "2").await.unwrap(),
            vec![1, 2, 3]
        );
        nuverse_server.abort();

        for root in [root, error_root, nuverse_root] {
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn handles_empty_pools_network_errors_and_restore_fallbacks() {
        let (client, root) = make_client(ServerRegion::Tw, "http://127.0.0.1:1").await;
        assert!(matches!(
            client.get_game_api("/x", None).await,
            Err(AppError::NoClientAvailable)
        ));
        assert!(matches!(
            client.post_game_api_body("/x", &json!({}), None).await,
            Err(AppError::NoClientAvailable)
        ));
        assert!(matches!(
            client
                .get_game_api_raw("/x", Duration::from_millis(1))
                .await,
            Err(AppError::NoClientAvailable)
        ));
        assert!(matches!(
            client.get_cp_mysekai_image("x").await,
            Err(AppError::NoClientAvailable)
        ));
        assert!(matches!(
            client.get_nuverse_mysekai_image("1", "2").await,
            Err(AppError::NoClientAvailable)
        ));
        assert!(matches!(
            client.recover_cookies().await,
            Err(AppError::CookieExpired)
        ));

        let value = json!({"plain": true});
        assert_eq!(
            client
                .restore_nuverse_api_response("/x", value.clone())
                .unwrap(),
            value
        );
        let packed = client.cryptor.pack(&json!({"master": 1})).unwrap();
        assert_eq!(client.restore_nuverse_master(&packed).unwrap()["master"], 1);

        let bad_session = session(ServerRegion::Tw, "1");
        assert!(client
            .post(&bad_session, "/network", Some(&json!({})), None)
            .await
            .is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn recovery_helpers_relogin_refresh_versions_and_map_errors() {
        let login = json!({
            "sessionToken": "fresh",
            "dataVersion": "new-data",
            "assetVersion": "new-asset"
        });
        let (url, server) = spawn_server(StaticResponse {
            status: 200,
            content_type: "application/octet-stream",
            body: encrypted(login),
            session_token: None,
        })
        .await;
        let (client, root) = make_client(ServerRegion::En, &url).await;
        let account = session(ServerRegion::En, "1");
        account.set_session_token(Some("old".to_string()));
        client.recover_session(&account).await.unwrap();
        assert_eq!(account.get_session_token().as_deref(), Some("fresh"));

        account.set_session_token(Some("old-2".to_string()));
        client.recover_upgrade(&account).await.unwrap();
        assert_eq!(
            client.headers.lock().get("X-Data-Version").unwrap(),
            "new-data"
        );
        account.set_session_token(Some("old-3".to_string()));
        client
            .recover_raw_session(&account, false, 403)
            .await
            .unwrap();
        account.set_session_token(Some("old-4".to_string()));
        client
            .recover_raw_session(&account, true, 426)
            .await
            .unwrap();
        assert!(client
            .recover_raw_response(&account, AppError::ParseError("bad".to_string()), 400)
            .await
            .is_err());
        assert!(client
            .recover_raw_response(&account, AppError::CookieExpired, 403)
            .await
            .is_err());
        server.abort();

        let (broken, broken_root) = make_client(ServerRegion::En, "http://127.0.0.1:1").await;
        let broken_session = session(ServerRegion::En, "1");
        assert!(matches!(
            broken.recover_session(&broken_session).await,
            Err(AppError::SessionError)
        ));
        assert!(matches!(
            broken
                .recover_raw_session(&broken_session, false, 403)
                .await,
            Err(AppError::SessionError)
        ));
        for path in [root, broken_root] {
            std::fs::remove_dir_all(path).unwrap();
        }
    }

    #[tokio::test]
    async fn classifies_all_raw_response_statuses() {
        let cases = [
            (403, "application/octet-stream", "recover-session"),
            (426, "application/octet-stream", "recover-upgrade"),
            (503, "application/octet-stream", "maintenance"),
            (500, "application/octet-stream", "unknown"),
            (403, "application/xml", "recover-cookie"),
            (503, "text/plain", "maintenance"),
            (500, "text/plain", "unknown"),
        ];
        for (status, content_type, expected) in cases {
            let (url, server) = spawn_server(StaticResponse {
                status,
                content_type,
                body: b"error".to_vec(),
                session_token: None,
            })
            .await;
            let (client, root) = make_client(ServerRegion::En, &url).await;
            let response = client.http_client.get(&url).send().await.unwrap();
            let result = client.classify_raw_response(response).await;
            match expected {
                "recover-session" => assert!(matches!(
                    result,
                    Ok(RawResponse::Recover {
                        error: AppError::SessionError,
                        status: 403
                    })
                )),
                "recover-upgrade" => assert!(matches!(
                    result,
                    Ok(RawResponse::Recover {
                        error: AppError::UpgradeRequired,
                        status: 426
                    })
                )),
                "recover-cookie" => assert!(matches!(
                    result,
                    Ok(RawResponse::Recover {
                        error: AppError::CookieExpired,
                        status: 403
                    })
                )),
                "maintenance" => assert!(matches!(result, Err(AppError::UnderMaintenance))),
                _ => assert!(matches!(result, Err(AppError::Unknown { .. }))),
            }
            server.abort();
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn account_watcher_helpers_classify_and_debounce_events() {
        assert!(is_account_change(&notify::EventKind::Create(
            CreateKind::File
        )));
        assert!(is_account_change(&notify::EventKind::Modify(
            ModifyKind::Any
        )));
        assert!(is_account_change(&notify::EventKind::Remove(
            RemoveKind::File
        )));
        assert!(!is_account_change(&notify::EventKind::Access(
            notify::event::AccessKind::Any
        )));

        let (sender, receiver) = std::sync::mpsc::channel();
        sender
            .send(Ok(notify::Event::new(notify::EventKind::Access(
                notify::event::AccessKind::Any,
            ))))
            .unwrap();
        sender
            .send(Ok(notify::Event::new(notify::EventKind::Modify(
                ModifyKind::Any,
            ))))
            .unwrap();
        assert!(wait_for_account_change(&receiver, "TEST"));
        drop(sender);
        assert!(!wait_for_account_change(&receiver, "TEST"));

        let (sender, receiver) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
        assert!(wait_for_quiet_directory(
            &receiver,
            Duration::from_millis(1)
        ));
        sender
            .send(Ok(notify::Event::new(notify::EventKind::Any)))
            .unwrap();
        drop(sender);
        assert!(!wait_for_quiet_directory(
            &receiver,
            Duration::from_millis(1)
        ));

        let raw = RawResponse::recover(AppError::SessionError, 403);
        assert!(matches!(raw, RawResponse::Recover { status: 403, .. }));
        let header = HeaderValue::from_static("test");
        assert_eq!(header.to_str().unwrap(), "test");
    }
}
