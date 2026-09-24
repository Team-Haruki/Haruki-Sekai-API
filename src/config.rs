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
    /// Master-data ingest only: JSON files ingested at once. Each file keeps a
    /// few row batches in memory, so this is the ingest memory knob on a
    /// shared node.
    #[serde(default = "default_ingest_concurrency")]
    pub ingest_concurrency: usize,
}

fn default_max_connections() -> u32 {
    10
}

fn default_ingest_concurrency() -> usize {
    crate::ingest_engine::DEFAULT_INGEST_CONCURRENCY
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dsn: String::new(),
            max_connections: default_max_connections(),
            ingest_concurrency: default_ingest_concurrency(),
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
    /// Proxy override for git's networked commands. Absent inherits the
    /// top-level `proxy`; an empty string forces a direct connection even when
    /// the top-level proxy is set. Lets a node route git through a proxy
    /// without dragging every other outbound consumer along with it.
    #[serde(default)]
    pub proxy: Option<String>,
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

/// Remote account source for master production: this node runs the region's
/// master updater itself (download, unpack, ingest, git push all local) but
/// borrows a peer node's game accounts for the two steps that need them — the
/// login-derived version probe and (CP servers) the authenticated master-split
/// fetch, which the peer relays as an untouched encrypted byte stream so it
/// never pays the decode memory cost.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct MasterRemoteSourceConfig {
    /// Base URL of the account node, e.g. `http://100.76.159.97:9999`. Empty
    /// disables remote-source mode.
    #[serde(default)]
    pub url: String,
    /// Bearer token matching the account node's `backend.internal_token`.
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

/// Freshness/staleness windows (seconds, fractions allowed) for the cached
/// global read endpoints (ranking / system / information). Per-region so a hot
/// region can be tuned without affecting the others. Fractional values matter
/// for a periodic poller: a freshness window at or above the poll interval
/// makes alternating polls hit a still-fresh entry and read duplicate data,
/// halving the effective sampling rate — set the window to about half the poll
/// interval (e.g. `0.5` against a 1 s poller) so every poll serves the previous
/// refresh and triggers the next one.
#[derive(Debug, Clone, Deserialize)]
pub struct CacheTtlConfig {
    /// Freshness window for the event ranking top100 endpoint.
    #[serde(default = "default_ranking_top100_ttl")]
    pub ranking_top100: f64,
    /// Freshness window for the event ranking-border endpoint.
    #[serde(default = "default_ranking_border_ttl")]
    pub ranking_border: f64,
    /// Freshness window for /system and /information.
    #[serde(rename = "static", default = "default_static_ttl")]
    pub static_endpoints: f64,
    /// How long past its freshness window a cached response may still be served
    /// while a background revalidation refreshes it (stale-while-revalidate).
    /// Also bounds how long an unreachable upstream is masked by stale data.
    /// 0 disables stale serving.
    #[serde(default = "default_max_stale")]
    pub max_stale: f64,
}

fn default_ranking_top100_ttl() -> f64 {
    1.0
}

fn default_ranking_border_ttl() -> f64 {
    30.0
}

fn default_static_ttl() -> f64 {
    300.0
}

fn default_max_stale() -> f64 {
    30.0
}

impl Default for CacheTtlConfig {
    fn default() -> Self {
        Self {
            ranking_top100: default_ranking_top100_ttl(),
            ranking_border: default_ranking_border_ttl(),
            static_endpoints: default_static_ttl(),
            max_stale: default_max_stale(),
        }
    }
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
    /// Optional master-payload key/IV pair; empty values use the API cipher.
    #[serde(default)]
    pub master_aes_key_hex: String,
    #[serde(default)]
    pub master_aes_iv_hex: String,
    #[serde(default)]
    pub enable_master_updater: bool,
    #[serde(default)]
    pub master_updater_cron: String,
    /// Deprecated and ignored: the polling AppHash updater was removed. App
    /// identity is pushed to `POST /internal/app-identity` instead (by the
    /// registry or an operator's refresh script).
    #[serde(default)]
    pub enable_app_hash_updater: bool,
    /// Deprecated and ignored (see `enable_app_hash_updater`).
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
    #[serde(default)]
    pub master_remote_source: MasterRemoteSourceConfig,
    /// Cache windows for this region's cached read endpoints.
    #[serde(default)]
    pub cache_ttls: CacheTtlConfig,
}

/// Settings for the `master_registry` binary (the master data manager): a
/// headless node that pulls each region's master from its owner node
/// (`servers.<region>.master_sync`), owns git push and DB ingest, publishes a
/// manifest per region and fans out update notices.
#[derive(Debug, Clone, Deserialize)]
pub struct RegistryConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_registry_port")]
    pub port: u16,
    /// Bearer token for mutating endpoints and owner webhooks. Empty disables
    /// them (reads stay open, they are served on the internal network).
    #[serde(default)]
    pub token: String,
    /// Directory for manifests, publish history and app-identity overrides.
    #[serde(default = "default_registry_state_dir")]
    pub state_dir: String,
    /// Database for the registry state (manifests, snapshots, publish history,
    /// app-identity overrides, music_metas pointers), e.g. a PostgreSQL DSN.
    /// Empty keeps the JSON files under `state_dir`. On first start with an
    /// empty database the files are imported (and left in place). music_metas
    /// blobs stay under `state_dir` either way.
    #[serde(default)]
    pub state_dsn: String,
    /// Peers to notify (`POST <url>/internal/master-updated`) after a region
    /// is published.
    #[serde(default)]
    pub subscribers: Vec<MasterSyncPeer>,
    /// The music_metas feed the registry maintains for its consumers.
    #[serde(default)]
    pub music_metas: MusicMetasConfig,
    /// Account nodes that must learn a new app identity: `PUT /v1/app/{region}`
    /// on the registry pushes it to each one's `POST /internal/app-identity`.
    #[serde(default)]
    pub account_nodes: Vec<MasterSyncPeer>,
}

/// Periodic pull of the regional `music_metas*.json` files (see
/// `registry::metas`). Enabled by default with the community upstream.
#[derive(Debug, Clone, Deserialize)]
pub struct MusicMetasConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 6-field cron for the upstream check; every 30 minutes by default.
    #[serde(default = "default_music_metas_cron")]
    pub cron: String,
    /// Inject the synthetic omakase rows (music_id 10000) consumers expect.
    #[serde(default = "default_true")]
    pub inject_omakase: bool,
    /// Per-region upstream URL overrides; an empty string disables a region.
    #[serde(default)]
    pub sources: HashMap<ServerRegion, String>,
    /// Proxy override for the music_metas fetch. Absent inherits the top-level
    /// `proxy`; an empty string forces a direct connection. A node that needs a
    /// proxy only for git (CN08 reaches github.com unreliably but sekai-data
    /// directly) sets this to `''` so a dead proxy node cannot take the metas
    /// feed down with it.
    #[serde(default)]
    pub proxy: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_music_metas_cron() -> String {
    "0 */30 * * * *".to_string()
}

impl Default for MusicMetasConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cron: default_music_metas_cron(),
            inject_omakase: true,
            sources: HashMap::new(),
            proxy: None,
        }
    }
}

fn default_registry_port() -> u16 {
    9998
}

fn default_registry_state_dir() -> String {
    "./Data/registry".to_string()
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_registry_port(),
            token: String::new(),
            state_dir: default_registry_state_dir(),
            state_dsn: String::new(),
            subscribers: Vec::new(),
            music_metas: MusicMetasConfig::default(),
            account_nodes: Vec::new(),
        }
    }
}

/// Deprecated and ignored: kept only so existing config files still parse.
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
    #[serde(default)]
    pub registry: RegistryConfig,
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
            proxy: None,
        }
    }
}

impl Config {
    /// Settings from the removed polling AppHash updater that are still
    /// present in the file; each is ignored and worth a startup warning.
    pub fn deprecated_app_hash_settings(&self) -> Vec<String> {
        let mut found = Vec::new();
        if !self.apphash_sources.is_empty() {
            found.push("apphash_sources".to_string());
        }
        let mut regions: Vec<_> = self.servers.iter().collect();
        regions.sort_by_key(|(r, _)| **r);
        for (region, server) in regions {
            let region = region.as_str();
            if server.enable_app_hash_updater {
                found.push(format!("servers.{region}.enable_app_hash_updater"));
            }
            if !server.app_hash_updater_cron.trim().is_empty() {
                found.push(format!("servers.{region}.app_hash_updater_cron"));
            }
        }
        found
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_regions_parse_and_report_protocol_family() {
        let cases = [
            ("JP", ServerRegion::Jp, true),
            ("en", ServerRegion::En, true),
            ("Tw", ServerRegion::Tw, false),
            ("kr", ServerRegion::Kr, false),
            ("CN", ServerRegion::Cn, false),
        ];

        for (input, expected, is_cp) in cases {
            let parsed: ServerRegion = input.parse().unwrap();
            assert_eq!(parsed, expected);
            assert_eq!(parsed.as_str(), input.to_ascii_lowercase());
            assert_eq!(parsed.is_cp_server(), is_cp);
        }
        assert_eq!(
            "unknown".parse::<ServerRegion>().unwrap_err(),
            "Unknown server region: unknown"
        );
    }

    #[test]
    fn partial_yaml_sections_receive_documented_defaults() {
        let config: Config = serde_yaml::from_str(
            r#"
backend: {}
redis: {}
database: {}
git: {}
servers:
  jp:
    upstreams:
      - url: https://upstream.example
  cn: {}
"#,
        )
        .unwrap();

        assert_eq!(config.backend.host, "0.0.0.0");
        assert_eq!(config.backend.port, 9999);
        assert_eq!(config.backend.log_level, "info");
        assert_eq!(config.redis.host, "localhost");
        assert_eq!(config.redis.port, 6379);
        assert_eq!(config.database.max_connections, 10);
        assert!(!config.git.enabled);
        assert_eq!(config.git.signing_format, GitSigningFormat::Gpg);

        let jp = &config.servers[&ServerRegion::Jp];
        assert_eq!(jp.upstreams[0].priority, 10);
        assert_eq!(
            jp.nuverse_schema_bundle_path,
            "Data/structures/nuverse_schema_bundle.json"
        );
        assert_eq!(config.servers[&ServerRegion::Cn].local_priority, 0);
    }

    #[test]
    fn absent_optional_sections_use_struct_defaults() {
        let config: Config = serde_yaml::from_str("backend: {}\n").unwrap();

        assert!(!config.redis.enabled);
        assert_eq!(config.redis.password, "");
        assert!(!config.database.enabled);
        assert_eq!(config.database.dsn, "");
        assert_eq!(config.master_database.max_connections, 10);
        assert_eq!(config.master_database.ingest_concurrency, 2);
        assert_eq!(config.registry.port, 9998);
        assert_eq!(config.registry.state_dir, "./Data/registry");
        assert!(config.registry.token.is_empty());
        assert!(config.registry.music_metas.enabled);
        assert!(config.registry.account_nodes.is_empty());
        assert!(config.deprecated_app_hash_settings().is_empty());
        assert!(config.registry.music_metas.inject_omakase);
        assert_eq!(config.registry.music_metas.cron, "0 */30 * * * *");
        assert_eq!(config.git.username, "");
        assert!(!config.git.sign_commits);
        assert!(config.servers.is_empty());
    }

    #[test]
    fn git_signing_format_accepts_supported_names() {
        #[derive(Deserialize)]
        struct Wrapper {
            format: GitSigningFormat,
        }

        for (yaml, expected) in [
            ("format: gpg", GitSigningFormat::Gpg),
            ("format: openpgp", GitSigningFormat::Gpg),
            ("format: ssh", GitSigningFormat::Ssh),
        ] {
            let parsed: Wrapper = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(parsed.format, expected);
        }
    }
}
