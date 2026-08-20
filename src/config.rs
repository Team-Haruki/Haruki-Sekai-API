use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

    pub fn is_cp_server(&self) -> bool {
        matches!(self, ServerRegion::Jp | ServerRegion::En)
    }
}

impl std::str::FromStr for ServerRegion {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "jp" => Ok(ServerRegion::Jp),
            "en" => Ok(ServerRegion::En),
            "tw" => Ok(ServerRegion::Tw),
            "kr" => Ok(ServerRegion::Kr),
            "cn" => Ok(ServerRegion::Cn),
            _ => Err(format!("Unknown server region: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RedisConfig {
    #[serde(default)]
    pub enabled: bool,
    // Field-level defaults must match `impl Default`: a *partial* `redis:`
    // section takes these, while a fully absent section takes the Default impl.
    #[serde(default = "default_redis_host")]
    pub host: String,
    #[serde(default = "default_redis_port")]
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
    /// Default log level for this crate's targets; the RUST_LOG env var, when
    /// set, takes precedence.
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default)]
    pub sekai_user_jwt_signing_key: String,
    /// Shared secret required (as `Authorization: Bearer <token>`) on this
    /// node's `/internal/*` endpoints. Empty disables those endpoints entirely,
    /// so a node never exposes internal forwarding unless explicitly configured.
    #[serde(default)]
    pub internal_token: String,
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
fn default_redis_host() -> String {
    "localhost".to_string()
}
fn default_redis_port() -> u16 {
    6379
}

fn default_nuverse_schema_bundle_path() -> String {
    "Data/structures/nuverse_schema_bundle.json".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub dsn: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
}

fn default_max_connections() -> u32 {
    10
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dsn: String::new(),
            max_connections: default_max_connections(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GitSigningFormat {
    #[default]
    #[serde(alias = "openpgp")]
    Gpg,
    Ssh,
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
    #[serde(default)]
    pub sign_commits: bool,
    #[serde(default)]
    pub signing_format: GitSigningFormat,
    #[serde(default)]
    pub signing_key: String,
    #[serde(default)]
    pub signing_program: String,
}

/// A remote Haruki Sekai API node that can serve this region's game API calls
/// (reached over the internal network, e.g. a Tailscale IP). Targets are tried
/// in ascending `priority` order; the local client participates with the
/// region's `local_priority` (default 0), so with all defaults local is
/// preferred and remotes (default 10) are fallbacks. Set an upstream's
/// priority below `local_priority` to prefer it (geo/QPS routing).
#[derive(Debug, Clone, Deserialize)]
pub struct UpstreamConfig {
    /// Base URL of the remote node, e.g. `http://100.64.0.2:9999`.
    pub url: String,
    /// Bearer token matching the remote node's `backend.internal_token`.
    #[serde(default)]
    pub token: String,
    #[serde(default = "default_upstream_priority")]
    pub priority: i32,
    /// Optional display name for logs; defaults to the URL.
    #[serde(default)]
    pub name: String,
}

fn default_upstream_priority() -> i32 {
    10
}

/// A peer node to notify (webhook) after this node updates a region's master
/// data locally.
#[derive(Debug, Clone, Deserialize)]
pub struct MasterSyncPeer {
    /// Base URL of the peer node, e.g. `http://100.64.0.1:9999`.
    pub url: String,
    /// Bearer token matching the peer's `backend.internal_token`.
    #[serde(default)]
    pub token: String,
}

/// Master-data synchronization between nodes. Each region has one "owner" node
/// (the one running the master updater with its own accounts); peer nodes pull
/// that region's master data from the owner over the internal network instead
/// of downloading it from the game/CDN themselves.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct MasterSyncConfig {
    /// Owner node to pull this region's master data from (base URL). Empty
    /// disables pulling on this node.
    #[serde(default)]
    pub source_url: String,
    /// Bearer token matching the owner's `backend.internal_token`.
    #[serde(default)]
    pub source_token: String,
    /// Cron for fallback version polling against the owner (6-field). Empty
    /// disables polling; the owner's webhook then is the only trigger.
    #[serde(default)]
    pub poll_cron: String,
    /// Peers this node notifies after it updates the region's master data
    /// itself (i.e. when this node is the owner).
    #[serde(default)]
    pub notify: Vec<MasterSyncPeer>,
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
    #[serde(default = "default_nuverse_schema_bundle_path")]
    pub nuverse_schema_bundle_path: String,
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
    /// Remote nodes that can serve this region's game API calls. A region with
    /// upstreams but `enabled: false` (no local accounts) is served remote-only.
    #[serde(default)]
    pub upstreams: Vec<UpstreamConfig>,
    /// Priority of the local client among this region's targets (lower = tried
    /// first). Only meaningful when `upstreams` is non-empty.
    #[serde(default)]
    pub local_priority: i32,
    #[serde(default)]
    pub master_sync: MasterSyncConfig,
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
    pub master_database: DatabaseConfig,
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
            sign_commits: false,
            signing_format: GitSigningFormat::default(),
            signing_key: "".to_string(),
            signing_program: "".to_string(),
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
