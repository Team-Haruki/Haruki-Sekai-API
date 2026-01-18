use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerRegion {
    Jp,
    En,
    Tw,
    Kr,
    Cn,
}

impl ServerRegion {
    pub fn as_str(&self) -> &'static str {
        match self {
            ServerRegion::Jp => "jp",
            ServerRegion::En => "en",
            ServerRegion::Tw => "tw",
            ServerRegion::Kr => "kr",
            ServerRegion::Cn => "cn",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "jp" => Some(ServerRegion::Jp),
            "en" => Some(ServerRegion::En),
            "tw" => Some(ServerRegion::Tw),
            "kr" => Some(ServerRegion::Kr),
            "cn" => Some(ServerRegion::Cn),
            _ => None,
        }
    }

    pub fn is_cp_server(&self) -> bool {
        matches!(self, ServerRegion::Jp | ServerRegion::En)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RedisConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BackendConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub ssl: bool,
    #[serde(default)]
    pub ssl_cert: String,
    #[serde(default)]
    pub ssl_key: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default)]
    pub main_log_file: String,
    #[serde(default)]
    pub access_log: String,
    #[serde(default)]
    pub access_log_path: String,
    #[serde(default)]
    pub sekai_user_jwt_signing_key: String,
    #[serde(default)]
    pub enable_trust_proxy: bool,
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
    #[serde(default)]
    pub proxy_header: String,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}
fn default_port() -> u16 {
    9999
}
fn default_log_level() -> String {
    "info".to_string()
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DatabaseConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub driver: String,
    #[serde(default)]
    pub dsn: String,
    #[serde(default)]
    pub max_connections: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub master_dir: String,
    #[serde(default)]
    pub version_path: String,
    #[serde(default)]
    pub account_dir: String,
    #[serde(default)]
    pub api_url: String,
    #[serde(default)]
    pub nuverse_master_data_url: String,
    #[serde(default)]
    pub nuverse_structure_file_path: String,
    #[serde(default)]
    pub require_cookies: bool,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub aes_key_hex: String,
    #[serde(default)]
    pub aes_iv_hex: String,
    #[serde(default)]
    pub enable_master_updater: bool,
    #[serde(default)]
    pub master_updater_cron: String,
    #[serde(default)]
    pub enable_app_hash_updater: bool,
    #[serde(default)]
    pub app_hash_updater_cron: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppHashSource {
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetUpdaterInfo {
    pub url: String,
    #[serde(default)]
    pub authorization: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub proxy: String,
    #[serde(default)]
    pub jp_sekai_cookie_url: String,
    #[serde(default)]
    pub git: GitConfig,
    #[serde(default)]
    pub redis: RedisConfig,
    pub backend: BackendConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub apphash_sources: Vec<AppHashSource>,
    #[serde(default)]
    pub asset_updater_servers: Vec<AssetUpdaterInfo>,
    #[serde(default)]
    pub servers: HashMap<ServerRegion, ServerConfig>,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "localhost".to_string(),
            port: 6379,
            password: "".to_string(),
        }
    }
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            username: "".to_string(),
            email: "".to_string(),
            password: "".to_string(),
        }
    }
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let config_path =
            env::var("CONFIG_PATH").unwrap_or_else(|_| "haruki-sekai-configs.yaml".to_string());
        let path = Path::new(&config_path);
        let file = File::open(path)
            .map_err(|e| anyhow::anyhow!("Failed to open config file '{}': {}", config_path, e))?;
        let reader = BufReader::new(file);
        let config: Config = serde_yaml::from_reader(reader)
            .map_err(|e| anyhow::anyhow!("Failed to parse config: {}", e))?;
        Ok(config)
    }
}
