use figment::providers::{Env, Format, Yaml};
use figment::Figment;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::path::Path;
use tracing::warn;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub password_file: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub url_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default = "default_log_format")]
    pub log_format: String,
    #[serde(default)]
    pub main_log_file: String,
    #[serde(default)]
    pub access_log: String,
    #[serde(default)]
    pub access_log_path: String,
    #[serde(default)]
    pub sekai_user_jwt_signing_key: String,
    #[serde(default)]
    pub sekai_user_jwt_signing_key_file: String,
    #[serde(default)]
    pub enable_trust_proxy: bool,
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
    #[serde(default)]
    pub proxy_header: String,
    #[serde(default = "default_run_updaters_inproc")]
    pub run_updaters_inproc: bool,
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
fn default_log_format() -> String {
    "text".to_string()
}
fn default_run_updaters_inproc() -> bool {
    true
}
fn default_storage_poll_interval_secs() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DatabaseConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub driver: String,
    #[serde(default)]
    pub dsn: String,
    #[serde(default)]
    pub dsn_file: String,
    #[serde(default)]
    pub max_connections: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GitSigningFormat {
    #[default]
    #[serde(alias = "openpgp")]
    Gpg,
    Ssh,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub password_file: String,
    #[serde(default)]
    pub sign_commits: bool,
    #[serde(default)]
    pub signing_format: GitSigningFormat,
    #[serde(default)]
    pub signing_key: String,
    #[serde(default)]
    pub signing_key_file: String,
    #[serde(default)]
    pub signing_program: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default)]
    pub scheme: String,
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub bucket: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub access_key_id: String,
    #[serde(default)]
    pub secret_access_key: String,
    #[serde(default)]
    pub secret_access_key_file: String,
    #[serde(default)]
    pub access_key_secret: String,
    #[serde(default)]
    pub access_key_secret_file: String,
    #[serde(default = "default_storage_poll_interval_secs")]
    pub poll_interval_secs: u64,
    #[serde(default)]
    pub options: HashMap<String, String>,
}

impl StorageConfig {
    pub fn is_configured(&self) -> bool {
        !self.scheme.is_empty()
            || !self.root.is_empty()
            || !self.path.is_empty()
            || !self.endpoint.is_empty()
            || !self.bucket.is_empty()
            || !self.region.is_empty()
            || !self.access_key_id.is_empty()
            || !self.secret_access_key.is_empty()
            || !self.secret_access_key_file.is_empty()
            || !self.access_key_secret.is_empty()
            || !self.access_key_secret_file.is_empty()
            || !self.options.is_empty()
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            scheme: String::new(),
            root: String::new(),
            path: String::new(),
            endpoint: String::new(),
            bucket: String::new(),
            region: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            secret_access_key_file: String::new(),
            access_key_secret: String::new(),
            access_key_secret_file: String::new(),
            poll_interval_secs: default_storage_poll_interval_secs(),
            options: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub master_dir: String,
    #[serde(default)]
    pub master_storage: StorageConfig,
    #[serde(default)]
    pub version_path: String,
    #[serde(default)]
    pub version_storage: StorageConfig,
    #[serde(default)]
    pub account_dir: String,
    #[serde(default)]
    pub account_storage: StorageConfig,
    #[serde(default)]
    pub api_url: String,
    #[serde(default)]
    pub nuverse_master_data_url: String,
    #[serde(default)]
    pub nuverse_structure_file_path: String,
    #[serde(default)]
    pub nuverse_structure_storage: StorageConfig,
    #[serde(default)]
    pub require_cookies: bool,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub aes_key_hex: String,
    #[serde(default)]
    pub aes_key_hex_file: String,
    #[serde(default)]
    pub aes_iv_hex: String,
    #[serde(default)]
    pub aes_iv_hex_file: String,
    #[serde(default)]
    pub enable_master_updater: bool,
    #[serde(default)]
    pub master_updater_cron: String,
    #[serde(default)]
    pub enable_app_hash_updater: bool,
    #[serde(default)]
    pub app_hash_updater_cron: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppHashSource {
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetUpdaterInfo {
    pub url: String,
    #[serde(default)]
    pub authorization: String,
    #[serde(default)]
    pub authorization_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub proxy: String,
    #[serde(default)]
    pub jp_sekai_cookie_url: String,
    #[serde(default)]
    pub git: GitConfig,
    #[serde(default)]
    pub redis: RedisConfig,
    #[serde(default)]
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
            password_file: "".to_string(),
            url: "".to_string(),
            url_file: "".to_string(),
        }
    }
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            ssl: false,
            ssl_cert: "".to_string(),
            ssl_key: "".to_string(),
            log_level: default_log_level(),
            log_format: default_log_format(),
            main_log_file: "".to_string(),
            access_log: "".to_string(),
            access_log_path: "".to_string(),
            sekai_user_jwt_signing_key: "".to_string(),
            sekai_user_jwt_signing_key_file: "".to_string(),
            enable_trust_proxy: false,
            trusted_proxies: Vec::new(),
            proxy_header: "".to_string(),
            run_updaters_inproc: default_run_updaters_inproc(),
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
            password_file: "".to_string(),
            sign_commits: false,
            signing_format: GitSigningFormat::default(),
            signing_key: "".to_string(),
            signing_key_file: "".to_string(),
            signing_program: "".to_string(),
        }
    }
}

impl Config {
    /// Load configuration with the priority chain:
    /// `defaults` < `YAML file` < `HARUKI_*` env vars < `*_file` secret files.
    ///
    /// `CONFIG_PATH` env var selects the YAML file (default: `haruki-sekai-configs.yaml`
    /// in the current directory). When `CONFIG_PATH` is **explicitly set**, a missing
    /// file is a fatal error. When unset and the default file is missing, we log a
    /// warning and continue with defaults + env — this enables YAML-less deployments
    /// (e.g. K8s ConfigMap/Secret only) without breaking local usage.
    pub fn load() -> anyhow::Result<Self> {
        let (config_path, explicit) = match env::var("CONFIG_PATH") {
            Ok(p) => (p, true),
            Err(_) => ("haruki-sekai-configs.yaml".to_string(), false),
        };

        let mut figment = Figment::new();
        let path = Path::new(&config_path);
        if path.exists() {
            figment = figment.merge(Yaml::file(path));
        } else if explicit {
            return Err(anyhow::anyhow!(
                "Config file '{}' (from CONFIG_PATH) does not exist",
                config_path
            ));
        } else {
            warn!(
                "Config file '{}' not found; loading from defaults and HARUKI_* env vars only",
                config_path
            );
        }

        figment = figment.merge(Env::prefixed("HARUKI_").split("__"));

        let mut config: Config = figment
            .extract()
            .map_err(|e| anyhow::anyhow!("Failed to load config: {}", e))?;

        config.resolve_secret_files()?;
        Ok(config)
    }

    /// For every `*_file` field that is non-empty, read the referenced file and
    /// place its trimmed contents into the matching plaintext field. Files that
    /// don't exist or are empty after trimming are treated as fatal — silent
    /// fallthrough would risk shipping with an unset secret.
    fn resolve_secret_files(&mut self) -> anyhow::Result<()> {
        load_secret(
            &self.backend.sekai_user_jwt_signing_key_file,
            &mut self.backend.sekai_user_jwt_signing_key,
            "backend.sekai_user_jwt_signing_key_file",
        )?;
        load_secret(
            &self.database.dsn_file,
            &mut self.database.dsn,
            "database.dsn_file",
        )?;
        load_secret(
            &self.master_database.dsn_file,
            &mut self.master_database.dsn,
            "master_database.dsn_file",
        )?;
        load_secret(
            &self.redis.password_file,
            &mut self.redis.password,
            "redis.password_file",
        )?;
        load_secret(&self.redis.url_file, &mut self.redis.url, "redis.url_file")?;
        load_secret(
            &self.git.password_file,
            &mut self.git.password,
            "git.password_file",
        )?;
        load_secret(
            &self.git.signing_key_file,
            &mut self.git.signing_key,
            "git.signing_key_file",
        )?;

        for (region, server) in self.servers.iter_mut() {
            server
                .account_storage
                .resolve_secret_files(&format!("servers.{}.account_storage", region.as_str()))?;
            server
                .master_storage
                .resolve_secret_files(&format!("servers.{}.master_storage", region.as_str()))?;
            server
                .version_storage
                .resolve_secret_files(&format!("servers.{}.version_storage", region.as_str()))?;
            server
                .nuverse_structure_storage
                .resolve_secret_files(&format!(
                    "servers.{}.nuverse_structure_storage",
                    region.as_str()
                ))?;
            load_secret(
                &server.aes_key_hex_file,
                &mut server.aes_key_hex,
                &format!("servers.{}.aes_key_hex_file", region.as_str()),
            )?;
            load_secret(
                &server.aes_iv_hex_file,
                &mut server.aes_iv_hex,
                &format!("servers.{}.aes_iv_hex_file", region.as_str()),
            )?;
        }

        for (idx, asset) in self.asset_updater_servers.iter_mut().enumerate() {
            load_secret(
                &asset.authorization_file,
                &mut asset.authorization,
                &format!("asset_updater_servers[{}].authorization_file", idx),
            )?;
        }

        for (idx, source) in self.apphash_sources.iter_mut().enumerate() {
            source
                .storage
                .resolve_secret_files(&format!("apphash_sources[{}].storage", idx))?;
        }

        Ok(())
    }
}

impl StorageConfig {
    fn resolve_secret_files(&mut self, ctx: &str) -> anyhow::Result<()> {
        load_secret(
            &self.secret_access_key_file,
            &mut self.secret_access_key,
            &format!("{}.secret_access_key_file", ctx),
        )?;
        load_secret(
            &self.access_key_secret_file,
            &mut self.access_key_secret,
            &format!("{}.access_key_secret_file", ctx),
        )?;
        Ok(())
    }
}

fn load_secret(file_field: &str, target: &mut String, ctx: &str) -> anyhow::Result<()> {
    if file_field.is_empty() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(file_field).map_err(|e| {
        anyhow::anyhow!(
            "Failed to read secret file for {} ('{}'): {}",
            ctx,
            file_field,
            e
        )
    })?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!(
            "Secret file for {} ('{}') is empty after trimming",
            ctx,
            file_field
        ));
    }
    *target = trimmed.to_string();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;

    // Env-mutating tests must run serially.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        keys: Vec<String>,
        prev_cwd: std::path::PathBuf,
    }
    impl EnvGuard {
        fn new() -> Self {
            Self {
                keys: Vec::new(),
                prev_cwd: env::current_dir().unwrap(),
            }
        }
        fn set(&mut self, k: &str, v: &str) {
            self.keys.push(k.to_string());
            unsafe { env::set_var(k, v) };
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for k in &self.keys {
                unsafe { env::remove_var(k) };
            }
            unsafe { env::remove_var("CONFIG_PATH") };
            let _ = env::set_current_dir(&self.prev_cwd);
        }
    }

    fn write_yaml(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        let p = dir.join("haruki-sekai-configs.yaml");
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        p
    }

    const MINIMAL_YAML: &str = r#"
backend:
  host: "0.0.0.0"
  port: 9999
redis:
  enabled: false
  host: "127.0.0.1"
  port: 6379
"#;

    #[test]
    fn loads_example_yaml_unchanged() {
        let _g = ENV_LOCK.lock().unwrap();
        let mut guard = EnvGuard::new();
        let example = std::fs::read_to_string("haruki-sekai-configs.example.yaml").unwrap();
        let tmp = tempdir();
        let path = write_yaml(&tmp, &example);
        guard.set("CONFIG_PATH", path.to_str().unwrap());

        let cfg = Config::load().expect("example yaml must load");
        assert_eq!(cfg.backend.host, "0.0.0.0");
        assert_eq!(cfg.backend.port, 9999);
        assert!(cfg.backend.enable_trust_proxy);
        assert_eq!(cfg.redis.host, "127.0.0.1");
        assert_eq!(cfg.redis.port, 6379);
        assert!(cfg.database.enabled);
        assert_eq!(cfg.database.driver, "postgres");
        assert_eq!(cfg.servers.len(), 5);
    }

    #[test]
    fn env_overrides_yaml() {
        let _g = ENV_LOCK.lock().unwrap();
        let mut guard = EnvGuard::new();
        let tmp = tempdir();
        let path = write_yaml(&tmp, MINIMAL_YAML);
        guard.set("CONFIG_PATH", path.to_str().unwrap());
        guard.set("HARUKI_BACKEND__PORT", "12345");
        guard.set("HARUKI_REDIS__HOST", "redis.svc");

        let cfg = Config::load().unwrap();
        assert_eq!(cfg.backend.port, 12345);
        assert_eq!(cfg.backend.host, "0.0.0.0"); // unchanged
        assert_eq!(cfg.redis.host, "redis.svc");
    }

    #[test]
    fn secret_file_overrides_field() {
        let _g = ENV_LOCK.lock().unwrap();
        let mut guard = EnvGuard::new();
        let tmp = tempdir();

        let secret_path = tmp.join("jwt.key");
        std::fs::write(&secret_path, "from-secret-file\n").unwrap();

        let yaml = format!(
            r#"
backend:
  sekai_user_jwt_signing_key: "from-yaml"
  sekai_user_jwt_signing_key_file: "{}"
"#,
            secret_path.to_str().unwrap()
        );
        let path = write_yaml(&tmp, &yaml);
        guard.set("CONFIG_PATH", path.to_str().unwrap());

        let cfg = Config::load().unwrap();
        assert_eq!(cfg.backend.sekai_user_jwt_signing_key, "from-secret-file");
    }

    #[test]
    fn secret_file_beats_env_for_same_field() {
        let _g = ENV_LOCK.lock().unwrap();
        let mut guard = EnvGuard::new();
        let tmp = tempdir();

        let secret_path = tmp.join("dsn");
        std::fs::write(&secret_path, "postgres://from-file/db").unwrap();

        let path = write_yaml(&tmp, MINIMAL_YAML);
        guard.set("CONFIG_PATH", path.to_str().unwrap());
        guard.set("HARUKI_DATABASE__DSN", "postgres://from-env/db");
        guard.set("HARUKI_DATABASE__DSN_FILE", secret_path.to_str().unwrap());

        let cfg = Config::load().unwrap();
        assert_eq!(cfg.database.dsn, "postgres://from-file/db");
    }

    #[test]
    fn storage_secret_file_overrides_field() {
        let _g = ENV_LOCK.lock().unwrap();
        let mut guard = EnvGuard::new();
        let tmp = tempdir();

        let secret_path = tmp.join("s3-secret");
        std::fs::write(&secret_path, "from-storage-file\n").unwrap();

        let yaml = format!(
            r#"
servers:
  jp:
    version_storage:
      scheme: "s3"
      bucket: "haruki"
      path: "jp/version.json"
      access_key_id: "key"
      secret_access_key: "from-yaml"
      secret_access_key_file: "{}"
"#,
            secret_path.to_str().unwrap()
        );
        let path = write_yaml(&tmp, &yaml);
        guard.set("CONFIG_PATH", path.to_str().unwrap());

        let cfg = Config::load().unwrap();
        let jp = cfg.servers.get(&ServerRegion::Jp).unwrap();
        assert_eq!(jp.version_storage.secret_access_key, "from-storage-file");
    }

    #[test]
    fn missing_default_yaml_warns_but_loads() {
        let _g = ENV_LOCK.lock().unwrap();
        let mut guard = EnvGuard::new();
        let tmp = tempdir();
        env::set_current_dir(&tmp).unwrap();
        guard.set("HARUKI_BACKEND__PORT", "8888");

        let cfg = Config::load().expect("should load with no yaml file");
        assert_eq!(cfg.backend.port, 8888);
        assert_eq!(cfg.backend.host, "0.0.0.0"); // default
    }

    #[test]
    fn explicit_missing_config_path_is_error() {
        let _g = ENV_LOCK.lock().unwrap();
        let mut guard = EnvGuard::new();
        guard.set("CONFIG_PATH", "/tmp/definitely-does-not-exist-haruki.yaml");
        assert!(Config::load().is_err());
    }

    fn tempdir() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "haruki-cfg-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
