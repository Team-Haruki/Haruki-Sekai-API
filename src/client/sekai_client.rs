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
use super::egress::{EgressPool, EgressRoute, Evaluation, FailureKind};
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
    /// The node's default HTTP client (top-level `proxy`), used for traffic
    /// that is not the game API: CDN master downloads, public thumbnails.
    pub http_client: Client,
    /// Routes to the game API host; requests go out through the active one.
    pub egress: EgressPool,

    sessions: Arc<RwLock<Vec<Arc<AccountSession>>>>,
    session_index: AtomicUsize,
    /// The version-file state last put into the headers. `VersionHelper`
    /// cannot serve as the "before" side of a comparison: `load()` already
    /// stores what it read.
    applied_version: Mutex<VersionInfo>,
    /// Signalled after every account reload; the account pusher
    /// (`updater::account_sync`) waits on it.
    accounts_changed: tokio::sync::Notify,
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
        let mut builder = Self::http_client_builder();
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

    fn http_client_builder() -> reqwest::ClientBuilder {
        Client::builder()
            .timeout(Duration::from_secs(45))
            .pool_max_idle_per_host(20)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60))
    }

    /// HTTP client for one configured egress route: `""` leaves directly from
    /// this node (proxy environment variables ignored), anything else is the
    /// route's proxy URL. Unlike the default client it bounds the connect
    /// phase, so a dead relay node is noticed in seconds, not after the full
    /// request timeout.
    pub fn build_egress_http_client(url: &str) -> Result<Client, AppError> {
        let builder = Self::http_client_builder().connect_timeout(EGRESS_CONNECT_TIMEOUT);
        let builder = if url.is_empty() {
            builder.no_proxy()
        } else {
            // The URL may carry the relay token, so the error names no part of it.
            builder.proxy(
                reqwest::Proxy::all(url)
                    .map_err(|_| AppError::NetworkError("Invalid egress proxy URL".to_string()))?,
            )
        };
        builder
            .build()
            .map_err(|e| AppError::NetworkError(e.to_string()))
    }

    /// Build the region's egress pool. Without `egress` entries the region
    /// keeps the single default route, which is the pre-failover behaviour.
    fn build_egress_pool(
        region: ServerRegion,
        config: &ServerConfig,
        default_client: &Client,
    ) -> Result<EgressPool, AppError> {
        if config.egress.is_empty() {
            return Ok(EgressPool::single(region, default_client.clone()));
        }
        let mut routes: Vec<EgressRoute> = Vec::with_capacity(config.egress.len());
        for (index, egress) in config.egress.iter().enumerate() {
            let name = if egress.name.is_empty() {
                format!("egress{index}")
            } else {
                egress.name.clone()
            };
            if routes.iter().any(|route| route.name == name) {
                return Err(AppError::ParseError(format!(
                    "{} egress name '{}' is used twice",
                    region.as_str(),
                    name
                )));
            }
            let client = match egress.url.as_deref() {
                None => default_client.clone(),
                Some(url) => Self::build_egress_http_client(url)?,
            };
            routes.push(EgressRoute { name, client });
        }
        Ok(EgressPool::new(region, routes, config.egress_failback))
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
        let egress = Self::build_egress_pool(region, &config, &http_client)?;
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
            egress,
            sessions: Arc::new(RwLock::new(Vec::new())),
            session_index: AtomicUsize::new(0),
            applied_version: Mutex::new(VersionInfo::default()),
            accounts_changed: tokio::sync::Notify::new(),
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
        self.update_version_headers(&version, true);
        *self.applied_version.lock() = version;
        if self.config.account_sync.receive && !self.config.account_dir.is_empty() {
            // The directory may not exist before the first push, and the
            // account watcher only starts on an existing directory.
            tokio::fs::create_dir_all(&self.config.account_dir).await?;
        }
        self.reload_accounts().await
    }

    fn normalize_app_version(&self, version: &mut VersionInfo) {
        let effective = effective_app_version(
            self.config.pin_app_version_patch_to_zero,
            &version.app_version,
        );
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

    /// Put a version-file state into the request headers. The app identity
    /// always follows the file. The data/asset versions follow it only when
    /// `data_too` is set (the file's values are new) or the header is missing:
    /// a login may have supplied newer ones than a file the master pipeline
    /// has not caught up on yet, and those must not be taken back.
    fn update_version_headers(&self, version: &VersionInfo, data_too: bool) {
        let mut headers = self.headers.lock();
        headers.insert("X-App-Version".to_string(), version.app_version.clone());
        headers.insert("X-App-Hash".to_string(), version.app_hash.clone());
        for (name, value) in [
            ("X-Data-Version", &version.data_version),
            ("X-Asset-Version", &version.asset_version),
        ] {
            if data_too || !headers.contains_key(name) {
                headers.insert(name.to_string(), value.clone());
            }
        }
    }

    fn data_version_header(&self) -> Option<String> {
        self.headers.lock().get("X-Data-Version").cloned()
    }

    /// Snapshot what the next call on `session` will be sent with.
    fn sent(&self, session: &AccountSession) -> Sent {
        Sent {
            token: session.get_session_token(),
            data_version: self.data_version_header(),
        }
    }

    /// Login metadata only ever moves the data/asset version headers. The app
    /// identity (`X-App-Version`, `X-App-Hash`) comes solely from the version
    /// file, which only the app-identity push and the registry write: an
    /// appVersion reported by a game server is never adopted.
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
        self.apply_version(version);
        Ok(())
    }

    /// Make `version` (as just written to the version file) the in-memory
    /// state and the login headers, so the next request uses it.
    pub fn apply_version(&self, mut version: VersionInfo) {
        self.normalize_app_version(&mut version);
        let previous = std::mem::replace(&mut *self.applied_version.lock(), version.clone());
        if previous.app_version != version.app_version || previous.app_hash != version.app_hash {
            // A new app identity invalidates whatever the login gate concluded
            // about the old one.
            self.egress.reset_gate();
        }
        // Recoveries re-read the file on every 426. While the file still says
        // what it said last time, its data/asset versions are not news, and
        // re-applying them would undo the versions another request's relogin
        // just put in the headers (which restarts the 426/relogin cycle for
        // every call in flight).
        let data_changed = previous.data_version != version.data_version
            || previous.asset_version != version.asset_version;
        self.version_helper.update(version.clone());
        self.update_version_headers(&version, data_changed);
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

        // The account set changed (or the node just started): give logins a
        // fresh attempt regardless of what the gate concluded before.
        self.egress.reset_gate();
        let epoch_before = self.egress.epoch();

        // Log every account in concurrently, building the new session set off to
        // the side. The existing sessions keep serving traffic the whole time, so
        // there is no empty-pool window and no need to park inbound requests.
        let candidates: Vec<Arc<AccountSession>> = accounts
            .into_iter()
            .filter_map(|account| {
                if self.region.is_cp_server() && account.user_id().is_empty() {
                    warn!("{} Skipping account with empty user_id", region);
                    return None;
                }
                Some(Arc::new(AccountSession::new(account)))
            })
            .collect();
        let new_sessions = if self.config.account_sync.receive {
            // Standby: the master is logged in with these accounts, and a
            // second login would displace its sessions. Stay logged out until
            // a request actually needs an account.
            for session in &candidates {
                session.invalidate();
            }
            candidates
        } else {
            self.login_candidates(candidates, epoch_before, &region)
                .await
        };

        let count = new_sessions.len();
        {
            let mut sessions = self.sessions.write();
            *sessions = new_sessions;
            self.session_index.store(0, Ordering::SeqCst);
        }
        info!("{} Accounts reloaded, {} sessions active", region, count);
        self.accounts_changed.notify_one();
        Ok(())
    }

    /// Log freshly loaded accounts in and decide which ones enter the pool.
    async fn login_candidates(
        &self,
        candidates: Vec<Arc<AccountSession>>,
        epoch_before: u64,
        region: &str,
    ) -> Vec<Arc<AccountSession>> {
        let mut outcomes: Vec<Option<AppError>> = futures::future::join_all(
            candidates
                .iter()
                .map(|session| async move { self.login(session).await.err() }),
        )
        .await;
        if self.egress.epoch() != epoch_before {
            // The egress moved while the reload was in flight: the failures
            // were observed on the route just left, so retry them on the new
            // one before judging the accounts.
            outcomes = futures::future::join_all(candidates.iter().zip(outcomes).map(
                |(session, outcome)| async move {
                    match outcome {
                        Some(_) => self.login(session).await.err(),
                        None => None,
                    }
                },
            ))
            .await;
        }

        // An account is dropped only when its failure is its own: some other
        // account logged in, and the error is not a transient one. When every
        // login fails, the cause is the route or our app identity (a blocked
        // egress IP answers 403 for all of them), so the accounts are kept
        // without a token and log in on first use. Dropping them would leave
        // the pool empty until the next account-file change or a restart.
        let any_logged_in = outcomes.iter().any(Option::is_none);
        let mut new_sessions: Vec<Arc<AccountSession>> = Vec::with_capacity(candidates.len());
        for (session, outcome) in candidates.into_iter().zip(outcomes) {
            match outcome {
                None => new_sessions.push(session),
                Some(e) if login_failure_is_transient(&e) || !any_logged_in => {
                    warn!(
                        "{} Account #{} login failed ({}), keeping for lazy relogin",
                        region,
                        session.user_id(),
                        e
                    );
                    session.invalidate();
                    new_sessions.push(session);
                }
                Some(e) => error!(
                    "{} Failed to login account #{}: {}",
                    region,
                    session.user_id(),
                    e
                ),
            }
        }
        new_sessions
    }

    /// Resolves after the next account reload.
    pub async fn accounts_changed(&self) {
        self.accounts_changed.notified().await;
    }

    /// The raw JSON objects of this node's account files, for pushing to
    /// standby nodes. Holds credentials: never log the result.
    pub fn export_accounts(&self) -> Result<Vec<JsonValue>, AppError> {
        let account_dir = Path::new(&self.config.account_dir);
        let mut accounts = Vec::new();
        if !account_dir.exists() {
            return Ok(accounts);
        }
        let mut paths: Vec<_> = fs::read_dir(account_dir)?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("json"))
            .collect();
        // A stable order keeps the pushed bytes identical between runs, which
        // is what lets the standby skip an unchanged set.
        paths.sort();
        for path in paths {
            let value: JsonValue = match sonic_rs::from_slice(&fs::read(&path)?) {
                Ok(value) => value,
                Err(e) => {
                    warn!("Skipping unparsable account file {}: {}", path.display(), e);
                    continue;
                }
            };
            let items = match value {
                JsonValue::Array(items) => items,
                JsonValue::Object(_) => vec![value],
                _ => continue,
            };
            // Push exactly what this node would load itself. The standby
            // refuses a set with one bad entry, so a stray object here must
            // not freeze every standby's copy.
            accounts.extend(items.into_iter().enumerate().filter_map(|(index, item)| {
                let usable = item.is_object()
                    && self
                        .parse_account_value(item.clone(), &path, Some(index))
                        .is_some_and(|parsed| !parsed.token().is_empty());
                usable.then_some(item)
            }));
        }
        Ok(accounts)
    }

    /// Store an account set pushed by the master (`POST /internal/accounts`)
    /// as this region's only account file. Returns whether it differed from
    /// what was stored; the account watcher picks the change up.
    pub async fn import_accounts(&self, accounts: &[JsonValue]) -> Result<bool, AppError> {
        if !self.config.account_sync.receive {
            return Err(AppError::Forbidden(format!(
                "{} does not accept pushed accounts on this node",
                self.region.as_str()
            )));
        }
        if accounts.is_empty() {
            return Err(AppError::ParseError(
                "refusing an empty account set".to_string(),
            ));
        }
        for (index, account) in accounts.iter().enumerate() {
            // An object with no credential deserializes too (every field has
            // a default), so require the token the login is built from.
            let valid = account.is_object()
                && self
                    .parse_account_value(account.clone(), Path::new("pushed"), Some(index))
                    .is_some_and(|parsed| !parsed.token().is_empty());
            if !valid {
                return Err(AppError::ParseError(format!(
                    "pushed account #{index} is not a {} account",
                    self.region.as_str()
                )));
            }
        }
        let bytes = serde_json::to_vec(accounts)
            .map_err(|e| AppError::ParseError(format!("account serialization: {}", e)))?;
        let dir = Path::new(&self.config.account_dir);
        let target = dir.join(SYNCED_ACCOUNTS_FILE);
        if tokio::fs::read(&target)
            .await
            .is_ok_and(|current| current == bytes)
        {
            return Ok(false);
        }
        tokio::fs::create_dir_all(dir).await?;
        // Credentials: the file is created owner-only before any byte is
        // written, then renamed into place so a reload never sees half of it.
        let tmp = dir.join(format!(".{}.tmp", Uuid::new_v4()));
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&tmp).await?;
        let written = async {
            use tokio::io::AsyncWriteExt;
            file.write_all(&bytes).await?;
            file.flush().await?;
            drop(file);
            tokio::fs::rename(&tmp, &target).await
        }
        .await;
        if let Err(e) = written {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e.into());
        }
        info!(
            "{} Stored {} account(s) pushed by the master",
            self.region.as_str().to_uppercase(),
            accounts.len()
        );
        Ok(true)
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
            if self.config.account_sync.receive
                && path.file_name().and_then(|name| name.to_str()) != Some(SYNCED_ACCOUNTS_FILE)
            {
                // A standby serves the master's set only; a hand-copied file
                // next to it would put the same account in the pool twice.
                warn!(
                    "{} Ignoring {}: this region only loads the pushed account set",
                    self.region.as_str().to_uppercase(),
                    path.display()
                );
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

    pub fn session_count(&self) -> usize {
        self.sessions.read().len()
    }

    /// Require every session except `keep` to log in again before its next
    /// call. Used after an egress switch: a token minted on the old route is
    /// not reused from the new IP.
    fn invalidate_sessions_except(&self, keep: Option<&AccountSession>) {
        for session in self.sessions.read().iter() {
            if keep.is_none_or(|kept| kept.key() != session.key()) {
                session.invalidate();
            }
        }
    }

    /// Log a session in before use when it has no usable token (kept through
    /// a failed reload, or invalidated by an egress switch).
    async fn ensure_logged_in(&self, session: &AccountSession) -> Result<(), AppError> {
        if !session.needs_login() {
            return Ok(());
        }
        let _guard = session.lock_login().await;
        if !session.needs_login() {
            return Ok(());
        }
        self.login(session).await.map(|_| ()).map_err(|e| {
            error!(
                "{} Login before first use failed: {}",
                self.region.as_str().to_uppercase(),
                e
            );
            AppError::SessionError
        })
    }

    fn prepare_request(
        &self,
        http: &Client,
        session: &AccountSession,
        method: reqwest::Method,
        url: &str,
    ) -> reqwest::RequestBuilder {
        let mut req = http.request(method, url);
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
        self.send_api(session, method, path, data, params, request_timeout)
            .await
            .map(|(response, _sent)| response)
    }

    /// Send one game API call and report what it was sent with (token and
    /// data version, read under the account's api lock, so they are exactly
    /// what went out even when the call queued behind others).
    async fn send_api<T: serde::Serialize>(
        &self,
        session: &AccountSession,
        method: reqwest::Method,
        path: &str,
        data: Option<&T>,
        params: Option<&HashMap<String, String>>,
        request_timeout: Option<Duration>,
    ) -> Result<(Response, Sent), AppError> {
        self.ensure_logged_in(session).await?;
        let api_guard = session.lock_api().await;
        let sent = self.sent(session);
        let (epoch, http) = self.egress.active();
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
            let request = Self::build_api_request(
                self.prepare_request(&http, session, method.clone(), &url),
                params,
                packed.as_ref(),
                request_timeout,
            );
            match request.send().await {
                Ok(resp) => {
                    self.egress.note_route_alive(epoch);
                    self.update_session_token(session, &resp);
                    return Ok((resp, sent));
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
        let error = last_error.unwrap_or(AppError::NetworkError(
            "Request failed after retries".to_string(),
        ));
        drop(api_guard);
        self.handle_route_failure(session, epoch, &error).await;
        Err(error)
    }

    fn build_api_request(
        mut request: reqwest::RequestBuilder,
        params: Option<&HashMap<String, String>>,
        body: Option<&Vec<u8>>,
        timeout: Option<Duration>,
    ) -> reqwest::RequestBuilder {
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

    /// Log `session` in through the active egress route.
    ///
    /// Fails fast while the login gate is closed. A failure that indicts the
    /// route (see `client::egress`) is settled here: the same account logs in
    /// through standby routes, and the region moves to the first one that
    /// accepts it; if none does, the gate closes.
    pub async fn login(&self, session: &AccountSession) -> Result<LoginResponse, AppError> {
        self.egress.check_gate()?;
        let (epoch, http) = self.egress.active();
        let mut result = self.login_via(session, &http, None).await;
        if matches!(result, Err(AppError::CookieExpired)) && self.recover_cookies().await.is_ok() {
            result = self.login_via(session, &http, None).await;
        }
        let error = match result {
            Ok(login) => {
                self.egress.note_login_success(epoch);
                return Ok(login);
            }
            // Maintenance and cookie expiry are not the route's doing.
            Err(e @ (AppError::UnderMaintenance | AppError::CookieExpired)) => return Err(e),
            Err(e) => e,
        };
        let Some(evaluation) = self.egress.note_failure(
            epoch,
            FailureKind::Login,
            session.key(),
            &error,
            self.session_count(),
        ) else {
            return Err(error);
        };
        match self.probe_standbys(session, evaluation, &error).await {
            Ok(login) => Ok(login),
            Err(evaluation) => {
                if matches!(error, AppError::NetworkError(_)) {
                    // The login never reached the game, so there is no
                    // verdict to act on and no login load to shed. Closing
                    // the gate would only keep the region down after the
                    // network is back.
                    evaluation.rest();
                } else {
                    evaluation.reject(&error);
                }
                Err(error)
            }
        }
    }

    /// Log `session` in through each standby route of `evaluation`. The first
    /// one that accepts the login becomes the active route; otherwise the
    /// evaluation is handed back for the caller's verdict.
    async fn probe_standbys<'a>(
        &self,
        session: &AccountSession,
        evaluation: Evaluation<'a>,
        cause: &AppError,
    ) -> Result<LoginResponse, Evaluation<'a>> {
        let region = self.region.as_str().to_uppercase();
        let from = self.egress.active_name().to_string();
        let mut found = None;
        for (index, route) in evaluation.standbys() {
            match self
                .login_via(session, &route.client, Some(EGRESS_PROBE_TIMEOUT))
                .await
            {
                Ok(login) => {
                    found = Some((*index, route.name.clone(), login));
                    break;
                }
                Err(e) => warn!(
                    "{} Egress probe through '{}' failed as well: {}",
                    region, route.name, e
                ),
            }
        }
        match found {
            Some((index, name, login)) => {
                evaluation.switch(
                    index,
                    format!("'{from}' failed ({cause}) while '{name}' logged in"),
                );
                self.invalidate_sessions_except(Some(session));
                Ok(login)
            }
            None => Err(evaluation),
        }
    }

    /// A game call got no HTTP response on `epoch`'s route. When that keeps
    /// happening, see whether a standby route reaches the game.
    async fn handle_route_failure(&self, session: &AccountSession, epoch: u64, error: &AppError) {
        if self.egress.len() < 2 {
            // Nowhere else to go: nothing to count, nothing to decide.
            return;
        }
        let Some(evaluation) = self.egress.note_failure(
            epoch,
            FailureKind::Call,
            session.key(),
            error,
            self.session_count(),
        ) else {
            return;
        };
        if evaluation.standbys().is_empty() {
            return;
        }
        let _guard = session.lock_login().await;
        if let Err(evaluation) = self.probe_standbys(session, evaluation, error).await {
            evaluation.rest();
        }
    }

    /// Operator switch to the named egress route (`PUT /internal/egress`).
    pub fn switch_egress(&self, name: &str, hold: Option<bool>) -> Result<(), AppError> {
        if self.egress.switch_to(name, hold)? {
            self.invalidate_sessions_except(None);
        }
        Ok(())
    }

    /// Periodic failback: when the region runs on a standby and the primary
    /// is due for a check, log one account in through the primary and return
    /// to it on success.
    pub async fn egress_tick(&self) {
        let Some((primary, evaluation)) = self.egress.failback_due() else {
            return;
        };
        let Some(session) = self.get_session() else {
            return;
        };
        // The probe rotates the session token, so keep serving calls off this
        // account until it is done. Lock order is login, then api, everywhere
        // both are held.
        let _login_guard = session.lock_login().await;
        let _api_guard = session.lock_api().await;
        match self
            .login_via(&session, &primary.client, Some(EGRESS_PROBE_TIMEOUT))
            .await
        {
            Ok(_) => {
                evaluation.switch(
                    0,
                    format!("primary '{}' accepts logins again", primary.name),
                );
                self.invalidate_sessions_except(Some(&session));
            }
            Err(e) => evaluation.failback_failed(&e),
        }
    }

    /// Run `egress_tick` in the background. A no-op for a single-route region.
    pub fn start_egress_monitor(self: Arc<Self>) {
        if self.egress.len() < 2 {
            return;
        }
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(EGRESS_TICK_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                self.egress_tick().await;
            }
        });
    }

    /// One login attempt through `http`, with no gate and no failover.
    async fn login_via(
        &self,
        session: &AccountSession,
        http: &Client,
        timeout: Option<Duration>,
    ) -> Result<LoginResponse, AppError> {
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
        let mut req = self.prepare_request(http, session, method, &url);
        if let Some(timeout) = timeout {
            req = req.timeout(timeout);
        }
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
        session.mark_logged_in();
        info!("Account #{} logged in successfully", session.user_id());
        Ok(login_resp)
    }

    pub async fn resolve_login_version_metadata(
        &self,
        session: &AccountSession,
        login: LoginResponse,
    ) -> Result<LoginResponse, AppError> {
        if self.region.is_cp_server()
            || (!login.data_version.is_empty()
                && !login.asset_version.is_empty()
                && login.cdn_version > 0)
        {
            return Ok(login);
        }
        self.fetch_nuverse_version_metadata(session).await
    }

    /// Fetch the Nuverse login metadata used by the master updater. CN/TW 6.4 moved
    /// data/asset/CDN versions out of the access-token response and into this
    /// endpoint. This is a metadata probe, not an additional authentication
    /// step, so normal account login and game API calls do not use it.
    pub async fn fetch_nuverse_version_metadata(
        &self,
        session: &AccountSession,
    ) -> Result<LoginResponse, AppError> {
        if self.region.is_cp_server() {
            return Err(AppError::InvalidServerRegion(
                self.region.as_str().to_string(),
            ));
        }

        let url = format!(
            "{}/api/user/{}/login",
            self.config.api_url,
            session.user_id()
        );
        let (_, http) = self.egress.active();
        let mut req = http.post(&url);
        let metadata_headers: Vec<(String, String)> = {
            let headers = self.headers.lock();
            headers
                .iter()
                .filter(|(key, _)| {
                    !matches!(
                        key.to_lowercase().as_str(),
                        "x-request-id" | "x-data-version" | "x-asset-version"
                    )
                })
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        };
        for (key, value) in metadata_headers {
            req = req.header(key, value);
        }
        if let Some(token) = session.get_session_token() {
            req = req.header("X-Session-Token", token);
        }
        req = req.header("X-Request-Id", Uuid::new_v4().to_string());

        let empty_payload: HashMap<String, String> = HashMap::new();
        let resp = req
            .body(self.cryptor.pack(&empty_payload)?)
            .send()
            .await
            .map_err(|e| AppError::NetworkError(e.to_string()))?;
        self.update_session_token(session, &resp);
        let metadata: LoginResponse = self.handle_response(resp).await?;
        self.update_version_headers_from_login(&metadata);
        Ok(metadata)
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
        // Kind of the error the previous attempt recovered from. Meeting it
        // again right after a successful relogin means relogin does not help,
        // and repeating it would only add logins.
        let mut recovered: Option<&'static str> = None;
        while retry_count < max_retries {
            let epoch = self.egress.epoch();
            let (resp, sent) = self
                .send_api(session, method.clone(), path, body, params, None)
                .await?;
            match self.handle_response_value(resp).await {
                Ok((mut json_value, upstream_status)) => {
                    self.egress.note_api_success(epoch);
                    if restore && !self.region.is_cp_server() {
                        json_value = self.restore_nuverse_api_response(path, json_value)?;
                    }
                    return Ok((json_value, upstream_status));
                }
                Err(e @ (AppError::SessionError | AppError::UpgradeRequired)) => {
                    let kind = e.kind();
                    if recovered == Some(kind) {
                        return Err(self.futile_recovery(session, e));
                    }
                    let logged_in = if matches!(e, AppError::SessionError) {
                        self.recover_session(session, &sent).await?
                    } else {
                        self.recover_upgrade(session, &sent).await?
                    };
                    // Only a relogin this request performed can be futile; a
                    // retry riding on another request's recovery is not.
                    if logged_in {
                        recovered = Some(kind);
                    }
                }
                Err(AppError::CookieExpired) => self.recover_cookies().await?,
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

    /// The game rejected a call with `error` again right after the session
    /// recovered from it. Count it toward the login gate and give up on this
    /// request.
    fn futile_recovery(&self, session: &AccountSession, error: AppError) -> AppError {
        warn!(
            "{} Account #{} still rejected ({}) right after recovering",
            self.region.as_str().to_uppercase(),
            session.user_id(),
            error
        );
        self.egress.note_futile(
            self.egress.epoch(),
            session.key(),
            &error,
            self.session_count(),
        );
        error
    }

    /// Recover from a 403 on a call sent with `sent`. Returns whether this
    /// call logged in; `false` means another request already replaced the
    /// token the failed call used, so a plain retry is enough.
    async fn recover_session(
        &self,
        session: &AccountSession,
        sent: &Sent,
    ) -> Result<bool, AppError> {
        let _guard = session.lock_login().await;
        if session.get_session_token() != sent.token {
            return Ok(false);
        }
        warn!(
            "{} Session expired, re-logging in...",
            self.region.as_str().to_uppercase()
        );
        self.login(session).await.map(|_| true).map_err(|e| {
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

    /// Recover from a 426 on a call sent with `sent`. Returns whether this
    /// call logged in; `false` means the failed call went out with versions
    /// or a token that another request has replaced since, so a plain retry
    /// is enough. That keeps a data update to about one relogin per account
    /// in flight instead of one per request.
    async fn recover_upgrade(
        &self,
        session: &AccountSession,
        sent: &Sent,
    ) -> Result<bool, AppError> {
        let _guard = session.lock_login().await;
        if self.data_version_header() != sent.data_version
            || session.get_session_token() != sent.token
        {
            return Ok(false);
        }
        warn!(
            "{} Server upgrade required, refreshing version and re-logging in...",
            self.region.as_str().to_uppercase()
        );
        self.refresh_version().await?;
        match self.login(session).await {
            Ok(login) => self.adopt_login_versions(session, login).await,
            // With the gate closed the login was not attempted, so waiting
            // for an app version update here would only stall the request.
            Err(AppError::UpgradeRequired) if self.egress.is_gated() => {
                return Err(AppError::UpgradeRequired);
            }
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
        Ok(true)
    }

    /// Put the data/asset versions of a fresh login into the request headers.
    /// Nuverse logins since 6.4 no longer carry them, so they are read from
    /// the login metadata endpoint instead. Without this a data update leaves
    /// the headers stale: the game answers 426, the relogin succeeds, the
    /// retry is 426 again, and every request repeats that.
    async fn adopt_login_versions(&self, session: &AccountSession, login: LoginResponse) {
        let complete = !login.data_version.is_empty() && !login.asset_version.is_empty();
        if self.region.is_cp_server() || complete {
            self.update_version_headers_from_login(&login);
            return;
        }
        // The fetch spends the session's one-time token like any call, so it
        // takes the account's api lock (callers hold only the login lock).
        // It updates the headers itself.
        let _api_guard = session.lock_api().await;
        if let Err(e) = self.fetch_nuverse_version_metadata(session).await {
            warn!(
                "{} Could not read versions after re-login: {}",
                self.region.as_str().to_uppercase(),
                e
            );
        }
    }

    async fn retry_upgrade_login(&self, session: &AccountSession) -> Result<(), AppError> {
        warn!(
            "{} Login returned 426, waiting for app version update...",
            self.region.as_str().to_uppercase()
        );
        tokio::time::sleep(Duration::from_secs(10)).await;
        self.refresh_version().await?;
        match self.login(session).await {
            Ok(login) => {
                self.adopt_login_versions(session, login).await;
                Ok(())
            }
            Err(e) => {
                error!(
                    "{} Re-login after waiting for app update failed: {}",
                    self.region.as_str().to_uppercase(),
                    e
                );
                Err(AppError::UpgradeRequired)
            }
        }
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
        self.get_game_api_raw_with_session(&session, path, timeout)
            .await
    }

    /// `get_game_api_raw` on a caller-chosen session (the master updater
    /// reuses the account it just logged in with).
    pub async fn get_game_api_raw_with_session(
        &self,
        session: &AccountSession,
        path: &str,
        timeout: Duration,
    ) -> Result<Response, AppError> {
        let max_retries = 4;
        let mut retry_count = 0;
        let mut recovered: Option<&'static str> = None;
        while retry_count < max_retries {
            let epoch = self.egress.epoch();
            let (resp, sent) = self
                .send_api::<()>(
                    session,
                    reqwest::Method::GET,
                    path,
                    None,
                    None,
                    Some(timeout),
                )
                .await?;
            match self.classify_raw_response(resp).await? {
                RawResponse::Success(resp) => {
                    self.egress.note_api_success(epoch);
                    return Ok(resp);
                }
                RawResponse::Recover { error, status } => {
                    let kind = matches!(error, AppError::SessionError | AppError::UpgradeRequired)
                        .then(|| error.kind());
                    if kind.is_some() && recovered == kind {
                        return Err(self.futile_recovery(session, error));
                    }
                    if self
                        .recover_raw_response(session, error, status, &sent)
                        .await?
                    {
                        recovered = kind;
                    }
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

    /// Returns whether the recovery logged this session in.
    async fn recover_raw_response(
        &self,
        session: &AccountSession,
        error_value: AppError,
        status: u16,
        sent: &Sent,
    ) -> Result<bool, AppError> {
        match error_value {
            AppError::SessionError => self.recover_raw_session(session, false, status, sent).await,
            AppError::UpgradeRequired => {
                self.recover_raw_session(session, true, status, sent).await
            }
            AppError::CookieExpired => self.recover_cookies().await.map(|_| false),
            other => Err(other),
        }
    }

    async fn recover_raw_session(
        &self,
        session: &AccountSession,
        refresh_version: bool,
        status: u16,
        sent: &Sent,
    ) -> Result<bool, AppError> {
        let _guard = session.lock_login().await;
        if session.get_session_token() != sent.token
            || (refresh_version && self.data_version_header() != sent.data_version)
        {
            return Ok(false);
        }
        warn!(
            "{} Raw game GET hit {} — recovering session...",
            self.region.as_str().to_uppercase(),
            status
        );
        if refresh_version {
            self.refresh_version().await?;
        }
        match self.login(session).await {
            Ok(login) => {
                if refresh_version {
                    self.adopt_login_versions(session, login).await;
                }
                Ok(true)
            }
            Err(e) => {
                error!(
                    "{} Re-login during raw GET failed: {}",
                    self.region.as_str().to_uppercase(),
                    e
                );
                Err(if refresh_version {
                    AppError::UpgradeRequired
                } else {
                    AppError::SessionError
                })
            }
        }
    }

    async fn get_cp_image(&self, relative_path: &str) -> Result<Vec<u8>, AppError> {
        let session = self.get_session().ok_or(AppError::NoClientAvailable)?;
        self.ensure_logged_in(&session).await?;
        let path_clean = relative_path.trim_start_matches('/');
        let image_url = format!("{}/{}", self.config.api_url, path_clean);
        let (_, http) = self.egress.active();
        let req = self.prepare_request(&http, &session, reqwest::Method::GET, &image_url);
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

    pub async fn get_nuverse_housing_thumbnail(&self, path: &str) -> Result<Vec<u8>, AppError> {
        use super::housing::{thumbnail_origin, valid_thumbnail_path, THUMBNAIL_PREFIX};
        let origin = thumbnail_origin(self.region)
            .ok_or_else(|| AppError::ParseError("Expected a Nuverse region".to_string()))?;
        if !valid_thumbnail_path(path) {
            return Err(AppError::ParseError(
                "Invalid housing thumbnail path".to_string(),
            ));
        }
        // Public objects need no game session headers or internal API credentials.
        let url = format!("{origin}{THUMBNAIL_PREFIX}{path}");
        self.fetch_public_housing_thumbnail(&url).await
    }

    async fn fetch_public_housing_thumbnail(&self, url: &str) -> Result<Vec<u8>, AppError> {
        let response = self
            .http_client
            .get(url)
            .send()
            .await
            .map_err(|e| AppError::NetworkError(e.to_string()))?;
        if response.status().as_u16() != 200 {
            return Err(AppError::Unknown {
                status: response.status().as_u16(),
                body: "Failed to fetch housing thumbnail".to_string(),
            });
        }
        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| AppError::NetworkError(e.to_string()))
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

/// What a call was about to be sent with. A recovery compares it with the
/// current state to tell whether another request already did the work.
struct Sent {
    token: Option<String>,
    data_version: Option<String>,
}

/// The one account file a standby region loads: the set pushed by the master.
pub const SYNCED_ACCOUNTS_FILE: &str = "synced-accounts.json";

/// Per-route timeout of an egress probe login: a dead standby must not hold
/// the failing request for the full request timeout.
const EGRESS_PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const EGRESS_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How often a region on a standby route checks whether failback is due.
const EGRESS_TICK_INTERVAL: Duration = Duration::from_secs(30);

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
/// (403s, malformed responses) is treated as bad account data and dropped
/// when another account did log in, since a dead account kept in rotation
/// would fail requests that healthy accounts could serve.
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

/// What a login (or the Nuverse metadata probe) reports. It deliberately has
/// no appVersion/appHash: a server-reported app version is never trusted.
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
    use std::sync::atomic::AtomicU16;
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
    async fn public_housing_images_need_no_session_and_preserve_failures() {
        for status in [200, 404, 503] {
            let (url, server) = spawn_server(StaticResponse {
                status,
                content_type: "image/png",
                body: vec![1, 2, 3],
                session_token: None,
            })
            .await;
            let (client, root) = make_client(ServerRegion::Cn, &url).await;
            // No accounts have been added, so authenticated image loading would fail.
            let result = client.fetch_public_housing_thumbnail(&url).await;
            if status == 200 {
                assert_eq!(result.unwrap(), vec![1, 2, 3]);
            } else {
                assert!(
                    matches!(result,Err(AppError::Unknown{status:actual,..}) if actual == status)
                );
            }
            assert!(client
                .get_nuverse_housing_thumbnail("../../private")
                .await
                .is_err());
            server.abort();
            std::fs::remove_dir_all(root).unwrap();
        }
        let (client, root) = make_client(ServerRegion::Jp, "http://127.0.0.1:1").await;
        assert!(client
            .get_nuverse_housing_thumbnail("invalid")
            .await
            .is_err());
        assert!(client
            .fetch_public_housing_thumbnail("http://127.0.0.1:1")
            .await
            .is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn constructs_initializes_and_refreshes_client_state() {
        assert!(SekaiClient::build_http_client(Some("://bad proxy")).is_err());
        assert!(SekaiClient::load_nuverse_schema_store("/missing/schema.json").is_err());

        let (mut client, root) = make_client(ServerRegion::Cn, "http://127.0.0.1:1").await;
        assert_eq!(client.region, ServerRegion::Cn);
        assert_eq!(client.headers.lock().get("X-Test").unwrap(), "yes");
        client.init().await.unwrap();
        assert_eq!(client.version_helper.get().app_version, "6.0.2");
        assert_eq!(client.headers.lock().get("X-App-Version").unwrap(), "6.0.2");

        std::fs::write(
            &client.config.version_path,
            r#"{"appVersion":"6.1.9","appHash":"h2","dataVersion":"d2","assetVersion":"a2"}"#,
        )
        .unwrap();
        // By default the version file's patch release is sent as is.
        client.refresh_version().await.unwrap();
        assert_eq!(client.headers.lock().get("X-App-Version").unwrap(), "6.1.9");
        assert_eq!(client.version_helper.get().app_version, "6.1.9");
        // The opt-in pin cuts it to major.minor.0.
        client.config.pin_app_version_patch_to_zero = true;
        client.refresh_version().await.unwrap();
        assert_eq!(client.headers.lock().get("X-App-Version").unwrap(), "6.1.0");
        assert_eq!(client.version_helper.get().app_version, "6.1.0");
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

    /// A scripted game endpoint. Used directly as `api_url`, or as an egress
    /// route: reqwest sends plain-HTTP requests for a proxied URL to the proxy
    /// in absolute form, so the same handler stands in for "the game as seen
    /// through this route".
    struct Route {
        url: String,
        login_status: Arc<AtomicU16>,
        api_status: Arc<AtomicU16>,
        logins: Arc<AtomicUsize>,
        /// CP user ids whose login is rejected with 403 whatever `login_status` says.
        rejected_users: Arc<Mutex<Vec<String>>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Route {
        fn logins(&self) -> usize {
            self.logins.load(Ordering::SeqCst)
        }

        fn set_login_status(&self, status: u16) {
            self.login_status.store(status, Ordering::SeqCst);
        }

        fn set_api_status(&self, status: u16) {
            self.api_status.store(status, Ordering::SeqCst);
        }
    }

    fn scripted(status: u16, ok_body: serde_json::Value) -> AxumResponse<Body> {
        let body = if status == 200 {
            encrypted(ok_body)
        } else {
            vec![0]
        };
        AxumResponse::builder()
            .status(status)
            .header("content-type", "application/octet-stream")
            .body(Body::from(body))
            .unwrap()
    }

    async fn spawn_route(login_status: u16, api_status: u16) -> Route {
        let login_status = Arc::new(AtomicU16::new(login_status));
        let api_status = Arc::new(AtomicU16::new(api_status));
        let logins = Arc::new(AtomicUsize::new(0));
        let rejected_users = Arc::new(Mutex::new(Vec::<String>::new()));
        let app = {
            let (login_status, api_status, logins, rejected_users) = (
                login_status.clone(),
                api_status.clone(),
                logins.clone(),
                rejected_users.clone(),
            );
            Router::new().fallback(move |uri: axum::http::Uri| {
                let path = uri.path().to_string();
                let response = if path.ends_with("/auth") {
                    logins.fetch_add(1, Ordering::SeqCst);
                    let rejected = rejected_users
                        .lock()
                        .iter()
                        .any(|user| path.contains(&format!("/user/{user}/")));
                    let status = if rejected {
                        403
                    } else {
                        login_status.load(Ordering::SeqCst)
                    };
                    scripted(status, json!({"sessionToken": "token"}))
                } else {
                    scripted(api_status.load(Ordering::SeqCst), json!({"result": "ok"}))
                };
                async move { response }
            })
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Route {
            url: format!("http://{address}"),
            login_status,
            api_status,
            logins,
            rejected_users,
            task,
        }
    }

    /// An EN client whose game host is only reachable through the given
    /// egress routes (name, proxy URL), in preference order.
    async fn make_egress_client(routes: &[(&str, &str)]) -> (SekaiClient, std::path::PathBuf) {
        let root = temp_dir("egress");
        let mut config = config("http://game.test", &root);
        config.egress = routes
            .iter()
            .map(|(name, url)| crate::config::EgressConfig {
                name: name.to_string(),
                url: Some(url.to_string()),
            })
            .collect();
        let client = SekaiClient::new(
            ServerRegion::En,
            config,
            None,
            None,
            SekaiClient::build_http_client(None).unwrap(),
            None,
        )
        .await
        .unwrap();
        (client, root)
    }

    fn write_accounts(root: &Path, user_ids: &[&str]) {
        for user_id in user_ids {
            std::fs::write(
                root.join(format!("accounts/{user_id}.json")),
                format!(r#"{{"userId":"{user_id}","deviceId":"d","credential":"c"}}"#),
            )
            .unwrap();
        }
    }

    const PROFILE: &str = "/user/{userId}/profile";

    // An account the game rejects (403 -> SessionError) while another one logs
    // in is dropped: keeping it in rotation would fail requests the healthy
    // account could serve.
    #[tokio::test]
    async fn reload_drops_only_accounts_that_fail_on_their_own() {
        let game = spawn_route(200, 200).await;
        game.rejected_users.lock().push("11".to_string());
        let (client, root) = make_client(ServerRegion::En, &game.url).await;
        write_accounts(&root, &["10", "11"]);
        client.reload_accounts().await.unwrap();
        assert_eq!(client.session_count(), 1);
        assert_eq!(client.get_session().unwrap().user_id(), "10");
        assert!(!client.egress.is_gated());
        game.task.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    // When every login is rejected the cause is the route or the app identity
    // (a blocked egress IP answers 403 for all accounts), not the accounts:
    // the pool must survive and heal on its own once logins work again.
    #[tokio::test]
    async fn reload_keeps_every_account_when_all_logins_are_rejected() {
        let game = spawn_route(403, 200).await;
        let (client, root) = make_client(ServerRegion::En, &game.url).await;
        write_accounts(&root, &["10", "11"]);
        client.reload_accounts().await.unwrap();
        assert_eq!(client.session_count(), 2);
        assert!(client.get_session().unwrap().needs_login());
        assert!(
            client.egress.is_gated(),
            "uniform rejection closes the gate"
        );
        let logins_while_blocked = game.logins();
        assert_eq!(logins_while_blocked, 2);

        // Requests fail fast behind the gate instead of logging in again.
        assert!(matches!(
            client.get_game_api(PROFILE, None).await,
            Err(AppError::SessionError)
        ));
        assert_eq!(game.logins(), logins_while_blocked);

        game.set_login_status(200);
        client.egress.expire_timers_for_test();
        let (value, status) = client.get_game_api(PROFILE, None).await.unwrap();
        assert_eq!((status, &value["result"]), (200, &json!("ok")));
        assert!(!client.egress.is_gated());
        assert_eq!(game.logins(), logins_while_blocked + 1);
        game.task.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    // The incident this exists for: the primary egress IP is rejected at login
    // while the same accounts log in fine through another node. The region
    // must move to the standby by itself, keep its accounts, and return to the
    // primary once it accepts logins again.
    #[tokio::test]
    async fn blocked_egress_fails_over_to_a_standby_and_back() {
        let primary = spawn_route(403, 200).await;
        let standby = spawn_route(200, 200).await;
        let (client, root) =
            make_egress_client(&[("primary", &primary.url), ("standby", &standby.url)]).await;
        write_accounts(&root, &["10", "11"]);
        client.reload_accounts().await.unwrap();

        let status = client.egress.status();
        assert_eq!(status.active, "standby");
        assert!(!status.primary && !status.gated);
        assert_eq!(status.switches, 1);
        assert_eq!(client.session_count(), 2, "no account is lost");
        assert_eq!((primary.logins(), standby.logins()), (2, 2));
        for _ in 0..2 {
            client.get_game_api(PROFILE, None).await.unwrap();
        }
        assert_eq!(standby.logins(), 2, "sessions are already logged in");

        // Not due yet: the tick leaves the primary alone.
        client.egress_tick().await;
        assert_eq!(primary.logins(), 2);

        // Still blocked when due: stay on the standby.
        client.egress.expire_timers_for_test();
        client.egress_tick().await;
        assert_eq!(primary.logins(), 3);
        assert_eq!(client.egress.active_name(), "standby");

        primary.set_login_status(200);
        client.egress.expire_timers_for_test();
        client.egress_tick().await;
        assert_eq!(client.egress.active_name(), "primary");
        assert_eq!(primary.logins(), 4);
        // The other session logs in again through the primary before its
        // next call; the probe session is already there.
        for _ in 0..2 {
            client.get_game_api(PROFILE, None).await.unwrap();
        }
        assert_eq!((primary.logins(), standby.logins()), (5, 2));
        primary.task.abort();
        standby.task.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    // A stale app identity is rejected on every route. The region must stay
    // put, stop logging in, and recover as soon as a new identity is applied.
    #[tokio::test]
    async fn identity_fault_closes_the_gate_instead_of_switching() {
        let primary = spawn_route(426, 200).await;
        let standby = spawn_route(426, 200).await;
        let (client, root) =
            make_egress_client(&[("primary", &primary.url), ("standby", &standby.url)]).await;
        write_accounts(&root, &["10", "11"]);
        // `init` as at startup: it records the version file as applied.
        client.init().await.unwrap();

        let status = client.egress.status();
        assert_eq!(status.active, "primary");
        assert!(status.gated);
        assert_eq!(status.switches, 0);
        assert_eq!(client.session_count(), 2);
        assert_eq!((primary.logins(), standby.logins()), (2, 1));

        let session = client.get_session().unwrap();
        assert!(matches!(
            client.login(&session).await,
            Err(AppError::UpgradeRequired)
        ));
        assert!(client.get_game_api(PROFILE, None).await.is_err());
        assert_eq!(
            (primary.logins(), standby.logins()),
            (2, 1),
            "no login leaves the node while the gate is closed"
        );

        // Re-reading an unchanged version file changes nothing.
        client.refresh_version().await.unwrap();
        assert!(client.egress.is_gated());

        // A new identity resets the gate, whether it is applied directly
        // (the app-identity push) or found in the file by a recovery.
        primary.set_login_status(200);
        std::fs::write(
            root.join("version.json"),
            r#"{"appVersion":"6.1.0","appHash":"hash","dataVersion":"data","assetVersion":"asset"}"#,
        )
        .unwrap();
        client.refresh_version().await.unwrap();
        assert!(!client.egress.is_gated(), "a new identity resets the gate");
        assert_eq!(client.headers.lock()["X-App-Version"], "6.1.0");
        client.get_game_api(PROFILE, None).await.unwrap();
        assert_eq!(client.egress.active_name(), "primary");
        primary.task.abort();
        standby.task.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    // The 2026-11-12 CN outage: a data update made the game answer 426, the
    // relogin succeeded but (Nuverse 6.4) carried no versions, so the headers
    // stayed stale and every request relogged in, about five logins a second
    // until the egress IP was refused. One relogin must now pick the new
    // versions up from the metadata endpoint and the call must go through.
    #[tokio::test]
    async fn nuverse_data_update_recovers_with_one_relogin() {
        let logins = Arc::new(AtomicUsize::new(0));
        let app = {
            let logins = logins.clone();
            Router::new().fallback(
                move |uri: axum::http::Uri, headers: axum::http::HeaderMap| {
                    let path = uri.path().to_string();
                    let response = if path == "/api/user/auth" {
                        logins.fetch_add(1, Ordering::SeqCst);
                        scripted(200, json!({"sessionToken": "token"}))
                    } else if path.ends_with("/login") {
                        scripted(
                            200,
                            json!({"dataVersion": "new", "assetVersion": "new-asset", "cdnVersion": 1}),
                        )
                    } else if headers.get("x-data-version").is_some_and(|v| v == "new") {
                        scripted(200, json!({"result": "ok"}))
                    } else {
                        scripted(426, json!({}))
                    };
                    async move { response }
                },
            )
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (client, root) = make_client(ServerRegion::Cn, &url).await;
        std::fs::write(
            root.join("accounts/account.json"),
            r#"{"userId":12,"deviceId":"d","accessToken":"a"}"#,
        )
        .unwrap();
        client.init().await.unwrap();
        assert_eq!(logins.load(Ordering::SeqCst), 1);
        assert_eq!(client.headers.lock()["X-Data-Version"], "data");

        let (value, status) = client.get_game_api(PROFILE, None).await.unwrap();
        assert_eq!((status, &value["result"]), (200, &json!("ok")));
        assert_eq!(logins.load(Ordering::SeqCst), 2, "one relogin, not a loop");
        assert_eq!(client.headers.lock()["X-Data-Version"], "new");
        assert!(!client.egress.is_gated());
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    // The same data update with calls in flight on several accounts. One
    // request's recovery must not undo another's: re-reading the (still
    // stale) version file used to put the old data version back, so every
    // call in flight got a 426 and relogged in, at about the request rate.
    #[tokio::test]
    async fn concurrent_calls_through_a_data_update_relogin_once_per_account() {
        let logins = Arc::new(AtomicUsize::new(0));
        let app = {
            let logins = logins.clone();
            Router::new().fallback(
                move |uri: axum::http::Uri, headers: axum::http::HeaderMap| {
                    let logins = logins.clone();
                    async move {
                        let path = uri.path();
                        if path == "/api/user/auth" {
                            logins.fetch_add(1, Ordering::SeqCst);
                            scripted(200, json!({"sessionToken": "token"}))
                        } else if path.ends_with("/login") {
                            scripted(
                                200,
                                json!({"dataVersion": "new", "assetVersion": "new-asset", "cdnVersion": 1}),
                            )
                        } else {
                            // Keep calls overlapping.
                            tokio::time::sleep(Duration::from_millis(30)).await;
                            if headers.get("x-data-version").is_some_and(|v| v == "new") {
                                scripted(200, json!({"result": "ok"}))
                            } else {
                                scripted(426, json!({}))
                            }
                        }
                    }
                },
            )
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (client, root) = make_client(ServerRegion::Cn, &url).await;
        for user in [12, 13, 14] {
            std::fs::write(
                root.join(format!("accounts/{user}.json")),
                format!(r#"{{"userId":{user},"deviceId":"d","accessToken":"a"}}"#),
            )
            .unwrap();
        }
        client.init().await.unwrap();
        assert_eq!(logins.load(Ordering::SeqCst), 3);

        let results =
            futures::future::join_all((0..12).map(|_| client.get_game_api(PROFILE, None))).await;
        for result in results {
            let (value, status) = result.expect("every call rides out the update");
            assert_eq!((status, &value["result"]), (200, &json!("ok")));
        }
        let relogins = logins.load(Ordering::SeqCst) - 3;
        assert!(
            (1..=3).contains(&relogins),
            "at most one relogin per account, got {relogins}"
        );
        assert!(!client.egress.is_gated());

        // The version file still holds the old data version. Re-reading it
        // (every later 426 recovery does) must not take the headers back.
        client.refresh_version().await.unwrap();
        assert_eq!(client.headers.lock()["X-Data-Version"], "new");
        client.get_game_api(PROFILE, None).await.unwrap();
        assert_eq!(logins.load(Ordering::SeqCst) - 3, relogins);

        // A version file that does move on is applied.
        std::fs::write(
            root.join("version.json"),
            r#"{"appVersion":"6.0.2","appHash":"hash","dataVersion":"newer","assetVersion":"asset"}"#,
        )
        .unwrap();
        client.refresh_version().await.unwrap();
        assert_eq!(client.headers.lock()["X-Data-Version"], "newer");
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    // Logins that succeed while the game keeps answering 426 must not loop:
    // one relogin per request, then the gate, then no logins at all.
    #[tokio::test]
    async fn futile_relogins_are_cut_off() {
        let game = spawn_route(200, 426).await;
        let (client, root) = make_client(ServerRegion::En, &game.url).await;
        write_accounts(&root, &["10", "11"]);
        client.reload_accounts().await.unwrap();
        assert_eq!(game.logins(), 2);

        for expected_logins in [3, 4] {
            assert!(matches!(
                client.get_game_api(PROFILE, None).await,
                Err(AppError::UpgradeRequired)
            ));
            assert_eq!(game.logins(), expected_logins, "one relogin per request");
        }
        assert!(client.egress.is_gated());
        assert!(matches!(
            client.get_game_api(PROFILE, None).await,
            Err(AppError::UpgradeRequired)
        ));
        assert_eq!(game.logins(), 4, "gated: no further logins");

        game.set_api_status(200);
        client.get_game_api(PROFILE, None).await.unwrap();
        assert!(!client.egress.is_gated(), "a working call reopens the gate");
        game.task.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    // Calls that never get an HTTP response (the route itself is down) move
    // the region to a standby that reaches the game.
    #[tokio::test]
    async fn unreachable_egress_fails_over_on_game_calls() {
        let dead_url = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            format!("http://{}", listener.local_addr().unwrap())
        };
        let standby = spawn_route(200, 200).await;
        let (client, root) =
            make_egress_client(&[("primary", &dead_url), ("standby", &standby.url)]).await;
        let sessions = vec![
            session(ServerRegion::En, "10"),
            session(ServerRegion::En, "11"),
        ];
        for session in &sessions {
            session.set_session_token(Some("token".to_string()));
        }
        *client.sessions.write() = sessions;

        for _ in 0..2 {
            assert!(matches!(
                client.get_game_api(PROFILE, None).await,
                Err(AppError::NetworkError(_))
            ));
        }
        assert_eq!(client.egress.active_name(), "standby");
        assert!(!client.egress.is_gated());
        for _ in 0..2 {
            client.get_game_api(PROFILE, None).await.unwrap();
        }
        assert_eq!(
            standby.logins(),
            2,
            "each session logged in on the new route"
        );
        standby.task.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    // A standby must hold the master's accounts without using them: a login
    // there would displace the master's sessions. It loads only the pushed
    // set, and an account logs in the first time a request needs it.
    #[tokio::test]
    async fn standby_stores_pushed_accounts_and_logs_in_only_on_use() {
        let game = spawn_route(200, 200).await;
        let root = temp_dir("standby");
        let mut config = config(&game.url, &root);
        config.account_sync.receive = true;
        // A hand-copied file next to the pushed set would double an account.
        write_accounts(&root, &["99"]);
        let client = SekaiClient::new(
            ServerRegion::En,
            config,
            None,
            None,
            SekaiClient::build_http_client(None).unwrap(),
            None,
        )
        .await
        .unwrap();
        client.init().await.unwrap();
        assert_eq!(client.session_count(), 0, "nothing pushed yet");

        let pushed = vec![
            json!({"userId": "10", "deviceId": "d", "credential": "c"}),
            json!({"userId": "11", "deviceId": "d", "credential": "c"}),
        ];
        assert!(client.import_accounts(&pushed).await.unwrap());
        assert!(
            !client.import_accounts(&pushed).await.unwrap(),
            "an identical set is not rewritten"
        );
        let stored = root.join("accounts").join(SYNCED_ACCOUNTS_FILE);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&stored).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "credentials are owner-only");
        }
        let leftovers = std::fs::read_dir(root.join("accounts"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);

        client.reload_accounts().await.unwrap();
        assert_eq!(client.session_count(), 2);
        assert_eq!(game.logins(), 0, "loading does not log in");
        client.get_game_api(PROFILE, None).await.unwrap();
        assert_eq!(game.logins(), 1, "only the account that served logged in");

        for bad in [
            vec![],
            vec![json!("not an object")],
            vec![json!({"userId": "12", "deviceId": "d"})],
        ] {
            assert!(matches!(
                client.import_accounts(&bad).await,
                Err(AppError::ParseError(_))
            ));
        }
        assert_eq!(
            std::fs::read(&stored).unwrap(),
            serde_json::to_vec(&pushed).unwrap(),
            "a rejected push leaves the stored set alone"
        );
        game.task.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn master_exports_account_files_and_refuses_pushes() {
        let (client, root) = make_client(ServerRegion::En, "http://127.0.0.1:1").await;
        assert!(client.export_accounts().unwrap().is_empty());
        write_accounts(&root, &["10"]);
        std::fs::write(
            root.join("accounts/batch.json"),
            r#"[{"userId":"11","deviceId":"d","credential":"c"},{"userId":"12","deviceId":"d","credential":"c"}, 7, {"userId":"13","deviceId":"d"}]"#,
        )
        .unwrap();
        std::fs::write(root.join("accounts/broken.json"), "{").unwrap();
        std::fs::write(root.join("accounts/notes.txt"), "ignored").unwrap();
        let exported = client.export_accounts().unwrap();
        let ids: Vec<&str> = exported
            .iter()
            .map(|account| account["userId"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["10", "11", "12"]);
        assert_eq!(exported, client.export_accounts().unwrap(), "stable order");

        assert!(matches!(
            client.import_accounts(&exported).await,
            Err(AppError::Forbidden(_))
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn operator_switch_holds_route_and_relogs_sessions() {
        let primary = spawn_route(200, 200).await;
        let standby = spawn_route(200, 200).await;
        let (client, root) =
            make_egress_client(&[("primary", &primary.url), ("standby", &standby.url)]).await;
        write_accounts(&root, &["10"]);
        client.reload_accounts().await.unwrap();
        assert_eq!(client.egress.active_name(), "primary");

        assert!(matches!(
            client.switch_egress("missing", None),
            Err(AppError::NotFound(_))
        ));
        client.switch_egress("standby", Some(true)).unwrap();
        assert!(client.get_session().unwrap().needs_login());
        client.get_game_api(PROFILE, None).await.unwrap();
        assert_eq!((primary.logins(), standby.logins()), (1, 1));

        client.egress.expire_timers_for_test();
        client.egress_tick().await;
        let status = client.egress.status();
        assert_eq!(status.active, "standby", "held routes do not fail back");
        assert!(status.hold);
        primary.task.abort();
        standby.task.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn egress_config_builds_named_routes_and_rejects_duplicates() {
        let root = temp_dir("egress_config");
        let mut config = config("http://game.test", &root);
        config.egress = serde_yaml::from_str(
            "[{name: here, url: ''}, {url: 'http://127.0.0.1:9'}, {name: inherited}]",
        )
        .unwrap();
        let http = SekaiClient::build_http_client(None).unwrap();
        let client = SekaiClient::new(
            ServerRegion::Cn,
            config.clone(),
            None,
            None,
            http.clone(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            client.egress.status().routes,
            ["here", "egress1", "inherited"]
        );

        config.egress[2].name = "here".to_string();
        let duplicate = SekaiClient::new(ServerRegion::Cn, config, None, None, http, None).await;
        assert!(matches!(duplicate, Err(AppError::ParseError(_))));
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
    async fn resolves_split_nuverse_metadata_without_changing_legacy_logins() {
        async fn metadata_handler(
            headers: axum::http::HeaderMap,
            body: axum::body::Bytes,
        ) -> AxumResponse<Body> {
            assert_eq!(headers["x-session-token"], "auth-token");
            assert!(!headers.contains_key("x-data-version"));
            assert!(!headers.contains_key("x-asset-version"));
            let payload: serde_json::Value = SekaiCryptor::from_hex(KEY, IV)
                .unwrap()
                .unpack(&body)
                .unwrap();
            assert_eq!(payload, json!({}));
            AxumResponse::builder()
                .header("content-type", "application/octet-stream")
                .header("x-session-token", "metadata-token")
                .body(Body::from(encrypted(json!({
                    "dataVersion": "6.4.0.2", "assetVersion": "6.4.0", "cdnVersion": 274
                }))))
                .unwrap()
        }
        let app = Router::new().route(
            "/api/user/{id}/login",
            axum::routing::post(metadata_handler),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        for region in [ServerRegion::Cn, ServerRegion::Tw, ServerRegion::Kr] {
            let (client, root) = make_client(region, &url).await;
            let account = session(region, "123");
            account.set_session_token(Some("auth-token".into()));
            let login = serde_json::from_value(json!({"sessionToken": "auth-token"})).unwrap();
            let result = client
                .resolve_login_version_metadata(&account, login)
                .await
                .unwrap();
            assert_eq!(result.cdn_version, 274);
            assert_eq!(result.data_version, "6.4.0.2");
            assert_eq!(
                account.get_session_token().as_deref(),
                Some("metadata-token")
            );
            assert_eq!(client.headers.lock()["X-Data-Version"], "6.4.0.2");
            std::fs::remove_dir_all(root).unwrap();
        }
        server.abort();
        for region in [ServerRegion::Tw, ServerRegion::Kr, ServerRegion::Jp] {
            let (client, root) = make_client(region, "http://127.0.0.1:1").await;
            let account = session(region, "123");
            let login = serde_json::from_value(json!({
                "dataVersion": "old-data", "assetVersion": "old-asset",
                "cdnVersion": if region.is_cp_server() { 0 } else { 200 }
            }))
            .unwrap();
            let result = client
                .resolve_login_version_metadata(&account, login)
                .await
                .unwrap();
            assert_eq!(result.data_version, "old-data");
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn server_reported_app_versions_are_never_adopted() {
        // A login (and the Nuverse metadata probe) answering with an
        // appVersion/appHash must leave the version file, the in-memory state
        // and the X-App-Version / X-App-Hash headers alone.
        let reply = json!({
            "sessionToken": "login-token",
            "appVersion": "9.9.9",
            "appHash": "server-hash",
            "dataVersion": "d9",
            "assetVersion": "a9",
            "cdnVersion": 300,
        });
        let (url, server) = spawn_server(StaticResponse {
            status: 200,
            content_type: "application/octet-stream",
            body: encrypted(reply.clone()),
            session_token: None,
        })
        .await;
        for region in [ServerRegion::Jp, ServerRegion::Tw, ServerRegion::Cn] {
            let (client, root) = make_client(region, &url).await;
            client.refresh_version().await.unwrap();
            let file_before = std::fs::read(&client.config.version_path).unwrap();
            let account = session(region, "12");
            let login = client.login(&account).await.unwrap();
            assert_eq!(login.data_version, "d9");
            if !region.is_cp_server() {
                client
                    .fetch_nuverse_version_metadata(&account)
                    .await
                    .unwrap();
            }
            let parsed: LoginResponse = serde_json::from_value(reply.clone()).unwrap();
            client.update_version_headers_from_login(&parsed);
            let headers = client.headers.lock().clone();
            assert_eq!(headers["X-App-Version"], "6.0.2", "{region:?}");
            assert_eq!(headers["X-App-Hash"], "hash", "{region:?}");
            assert_eq!(headers["X-Data-Version"], "d9", "{region:?}");
            assert_eq!(client.version_helper.get().app_version, "6.0.2");
            assert_eq!(
                std::fs::read(&client.config.version_path).unwrap(),
                file_before
            );
            std::fs::remove_dir_all(root).unwrap();
        }
        server.abort();
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

        let (cn, cn_root) = make_client(ServerRegion::Cn, &login_url).await;
        let cn_session = session(ServerRegion::Cn, "12");
        cn.login(&cn_session).await.unwrap();
        let metadata = cn
            .fetch_nuverse_version_metadata(&cn_session)
            .await
            .unwrap();
        assert_eq!(metadata.data_version, "d2");
        assert_eq!(cn.headers.lock().get("X-Data-Version").unwrap(), "d2");
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

        for root in [cp_root, nuverse_root, cn_root, game_root] {
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
        assert!(client
            .recover_session(&account, &client.sent(&account))
            .await
            .unwrap());
        assert_eq!(account.get_session_token().as_deref(), Some("fresh"));
        // Another request already replaced the token the failed call used:
        // no login, just retry.
        let stale = Sent {
            token: Some("old".to_string()),
            data_version: client.data_version_header(),
        };
        assert!(!client.recover_session(&account, &stale).await.unwrap());
        assert!(!client.recover_upgrade(&account, &stale).await.unwrap());

        account.set_session_token(Some("old-2".to_string()));
        assert!(client
            .recover_upgrade(&account, &client.sent(&account))
            .await
            .unwrap());
        assert_eq!(
            client.headers.lock().get("X-Data-Version").unwrap(),
            "new-data"
        );
        // The headers moved on since a call was sent: its 426 needs no login.
        let behind = Sent {
            token: account.get_session_token(),
            data_version: Some("older-data".to_string()),
        };
        assert!(!client.recover_upgrade(&account, &behind).await.unwrap());
        assert!(!client
            .recover_raw_session(&account, true, 426, &behind)
            .await
            .unwrap());

        account.set_session_token(Some("old-3".to_string()));
        assert!(client
            .recover_raw_session(&account, false, 403, &client.sent(&account))
            .await
            .unwrap());
        account.set_session_token(Some("old-4".to_string()));
        assert!(client
            .recover_raw_session(&account, true, 426, &client.sent(&account))
            .await
            .unwrap());
        let sent = client.sent(&account);
        assert!(client
            .recover_raw_response(
                &account,
                AppError::ParseError("bad".to_string()),
                400,
                &sent
            )
            .await
            .is_err());
        assert!(client
            .recover_raw_response(&account, AppError::CookieExpired, 403, &sent)
            .await
            .is_err());
        server.abort();

        let (broken, broken_root) = make_client(ServerRegion::En, "http://127.0.0.1:1").await;
        let broken_session = session(ServerRegion::En, "1");
        let sent = broken.sent(&broken_session);
        assert!(matches!(
            broken.recover_session(&broken_session, &sent).await,
            Err(AppError::SessionError)
        ));
        assert!(matches!(
            broken
                .recover_raw_session(&broken_session, false, 403, &sent)
                .await,
            Err(AppError::SessionError)
        ));
        // Logins that never reach the game must not close the gate: on a
        // single route that would keep the region down after the network
        // is back.
        for _ in 0..3 {
            assert!(matches!(
                broken.login(&broken_session).await,
                Err(AppError::NetworkError(_))
            ));
        }
        assert!(!broken.egress.is_gated());
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
