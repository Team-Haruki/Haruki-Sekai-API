use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::Value as JsonValue;
use tracing::{error, info, warn};

use crate::config::{ServerConfig, ServerRegion};
use crate::error::AppError;

use super::account::{AccountType, SekaiAccount, SekaiAccountCP, SekaiAccountNuverse};
use super::helper::{CookieHelper, VersionHelper};
use super::sekai_client::SekaiClient;

pub struct SekaiClientManager {
    pub region: ServerRegion,
    pub config: ServerConfig,
    pub version_helper: Arc<VersionHelper>,
    pub cookie_helper: Option<Arc<CookieHelper>>,
    pub clients: Vec<Arc<SekaiClient>>,
    pub proxy: Option<String>,
    client_index: AtomicUsize,
}

impl SekaiClientManager {
    pub async fn new(
        region: ServerRegion,
        config: ServerConfig,
        proxy: Option<String>,
        jp_cookie_url: Option<String>,
    ) -> Result<Self, AppError> {
        let version_helper = Arc::new(VersionHelper::new(&config.version_path));
        let cookie_helper = if region == ServerRegion::Jp && config.require_cookies {
            if let Some(ref url) = jp_cookie_url {
                if !url.is_empty() {
                    Some(Arc::new(CookieHelper::new(url)))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let manager = Self {
            region,
            config,
            version_helper,
            cookie_helper,
            clients: Vec::new(),
            proxy,
            client_index: AtomicUsize::new(0),
        };
        Ok(manager)
    }

    pub async fn init(&mut self) -> Result<(), AppError> {
        info!(
            "{} Initializing client manager...",
            self.region.as_str().to_uppercase()
        );
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
            let client = SekaiClient::new(
                self.region,
                self.config.clone(),
                account,
                self.cookie_helper.clone(),
                self.version_helper.clone(),
                self.proxy.clone(),
            )?;
            if let Err(e) = client.init().await {
                error!("Failed to init client: {}", e);
                continue;
            }
            match client.login().await {
                Ok(_) => {
                    self.clients.push(Arc::new(client));
                }
                Err(e) => {
                    error!("Failed to login: {}", e);
                }
            }
        }
        info!(
            "{} Client manager initialized with {} clients",
            self.region.as_str().to_uppercase(),
            self.clients.len()
        );
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
        if self.region.is_cp_server() {
            let json_str = serde_json::to_string(&value).ok()?;
            match sonic_rs::from_str::<SekaiAccountCP>(&json_str) {
                Ok(acc) => Some(AccountType::CP(acc)),
                Err(e) => {
                    if let Some(i) = idx {
                        warn!("[{}][{}] CP unmarshal error: {}", path.display(), i, e);
                    } else {
                        warn!("[{}] CP unmarshal error: {}", path.display(), e);
                    }
                    None
                }
            }
        } else {
            let json_str = serde_json::to_string(&value).ok()?;
            match sonic_rs::from_str::<SekaiAccountNuverse>(&json_str) {
                Ok(acc) => Some(AccountType::Nuverse(acc)),
                Err(e) => {
                    if let Some(i) = idx {
                        warn!("[{}][{}] Nuverse unmarshal error: {}", path.display(), i, e);
                    } else {
                        warn!("[{}] Nuverse unmarshal error: {}", path.display(), e);
                    }
                    None
                }
            }
        }
    }

    #[must_use]
    pub fn get_client(&self) -> Option<Arc<SekaiClient>> {
        if self.clients.is_empty() {
            return None;
        }
        let idx = self.client_index.fetch_add(1, Ordering::SeqCst) % self.clients.len();
        Some(self.clients[idx].clone())
    }

    pub async fn refresh_version(&self) -> Result<(), AppError> {
        let version = self.version_helper.load().await?;
        for client in &self.clients {
            let mut headers = client.headers.lock();
            headers.insert("X-App-Version".to_string(), version.app_version.clone());
            headers.insert("X-Data-Version".to_string(), version.data_version.clone());
            headers.insert("X-Asset-Version".to_string(), version.asset_version.clone());
            headers.insert("X-App-Hash".to_string(), version.app_hash.clone());
        }
        Ok(())
    }

    pub async fn refresh_cookies(&self) -> Result<(), AppError> {
        if let Some(ref helper) = self.cookie_helper {
            let cookie = helper.get_cookies(self.proxy.as_deref()).await?;
            for client in &self.clients {
                client
                    .headers
                    .lock()
                    .insert("Cookie".to_string(), cookie.clone());
            }
        }
        Ok(())
    }

    #[tracing::instrument(skip(self, params), fields(region = ?self.region))]
    pub async fn get_game_api(
        &self,
        path: &str,
        params: Option<&HashMap<String, String>>,
    ) -> Result<(JsonValue, u16), AppError> {
        let client = self.get_client().ok_or(AppError::NoClientAvailable)?;
        let max_retries = 4;
        let mut retry_count = 0;
        while retry_count < max_retries {
            let resp = client.get(path, params).await?;
            match client.handle_response_ordered(resp).await {
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
                    if let Err(e) = client.login().await {
                        error!(
                            "{} Re-login failed: {}",
                            self.region.as_str().to_uppercase(),
                            e
                        );
                        return Err(AppError::SessionError);
                    }
                    retry_count += 1;
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                Err(AppError::CookieExpired) => {
                    if self.config.require_cookies {
                        warn!(
                            "{} Cookies expired, refreshing...",
                            self.region.as_str().to_uppercase()
                        );
                        self.refresh_cookies().await?;
                        retry_count += 1;
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
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
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
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
        let client = self.get_client().ok_or(AppError::NoClientAvailable)?;
        client.get_cp_mysekai_image(path).await
    }

    pub async fn get_nuverse_mysekai_image(
        &self,
        user_id: &str,
        index: &str,
    ) -> Result<Vec<u8>, AppError> {
        let client = self.get_client().ok_or(AppError::NoClientAvailable)?;
        client.get_nuverse_mysekai_image(user_id, index).await
    }
}
