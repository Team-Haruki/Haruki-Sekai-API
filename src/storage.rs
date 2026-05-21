use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use opendal::{Entry, ErrorKind, Metadata, Operator};

use crate::config::StorageConfig;
use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageEntry {
    pub path: String,
    pub name: String,
    pub fingerprint: String,
}

#[derive(Clone)]
pub struct StorageLocation {
    op: Option<Operator>,
    base_path: String,
    scheme: String,
    display: String,
    local_path: Option<PathBuf>,
    poll_interval: Duration,
}

impl StorageLocation {
    pub fn file(config: &StorageConfig, legacy_path: &str, name: &str) -> Result<Self, AppError> {
        if config.is_configured() {
            return Self::from_config(config, legacy_path, false, false, name);
        }
        Self::from_legacy_file(legacy_path, name)
    }

    pub fn dir(
        config: &StorageConfig,
        legacy_path: &str,
        create_local_root: bool,
        name: &str,
    ) -> Result<Self, AppError> {
        if config.is_configured() {
            return Self::from_config(config, legacy_path, true, create_local_root, name);
        }
        Self::from_legacy_dir(legacy_path, create_local_root, name)
    }

    pub fn is_available(&self) -> bool {
        self.op.is_some()
    }

    pub fn is_local_fs(&self) -> bool {
        self.scheme == "fs" && self.local_path.is_some()
    }

    pub fn display(&self) -> &str {
        &self.display
    }

    pub fn local_path(&self) -> Option<&Path> {
        self.local_path.as_deref()
    }

    pub fn poll_interval(&self) -> Duration {
        self.poll_interval
    }

    pub async fn exists_base(&self) -> Result<bool, AppError> {
        self.exists(&self.base_path).await
    }

    pub async fn read_base(&self) -> Result<Vec<u8>, AppError> {
        self.read_path(&self.base_path).await
    }

    pub async fn read_child(&self, child: &str) -> Result<Vec<u8>, AppError> {
        let path = join_path(&self.dir_base(), child);
        self.read_path(&path).await
    }

    pub async fn read_path(&self, path: &str) -> Result<Vec<u8>, AppError> {
        let op = self.operator()?;
        let buf = op.read(path).await?;
        Ok(buf.to_vec())
    }

    pub async fn write_base(&self, data: impl Into<Vec<u8>>) -> Result<(), AppError> {
        self.write_path(&self.base_path, data).await
    }

    pub async fn write_child(&self, child: &str, data: impl Into<Vec<u8>>) -> Result<(), AppError> {
        let path = join_path(&self.dir_base(), child);
        self.write_path(&path, data).await
    }

    pub async fn write_sibling(
        &self,
        file_name: &str,
        data: impl Into<Vec<u8>>,
    ) -> Result<(), AppError> {
        let path = match self.base_path.rsplit_once('/') {
            Some((parent, _)) if !parent.is_empty() => join_path(parent, file_name),
            _ => normalize_file_path(file_name),
        };
        self.write_path(&path, data).await
    }

    pub async fn create_dir(&self) -> Result<(), AppError> {
        let op = self.operator()?;
        let path = self.dir_base();
        if !path.is_empty() {
            op.create_dir(&path).await?;
        }
        Ok(())
    }

    pub async fn list_json_files(&self) -> Result<Vec<StorageEntry>, AppError> {
        let op = match &self.op {
            Some(op) => op,
            None => return Ok(Vec::new()),
        };
        let prefix = self.dir_base();
        let entries = match op.list(&prefix).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut files = Vec::new();
        for entry in entries {
            if !is_file_entry(&entry) || !entry.name().ends_with(".json") {
                continue;
            }
            let path = entry.path().to_string();
            let name = entry.name().to_string();
            let meta = match op.stat(&path).await {
                Ok(meta) => meta,
                Err(e) if e.kind() == ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            files.push(StorageEntry {
                path,
                name,
                fingerprint: metadata_fingerprint(&meta),
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    pub async fn json_fingerprint_map(&self) -> Result<BTreeMap<String, String>, AppError> {
        let mut out = BTreeMap::new();
        for entry in self.list_json_files().await? {
            out.insert(entry.path, entry.fingerprint);
        }
        Ok(out)
    }

    async fn write_path(&self, path: &str, data: impl Into<Vec<u8>>) -> Result<(), AppError> {
        let op = self.operator()?;
        op.write(path, data.into()).await?;
        Ok(())
    }

    async fn exists(&self, path: &str) -> Result<bool, AppError> {
        let op = match &self.op {
            Some(op) => op,
            None => return Ok(false),
        };
        match op.stat(path).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn operator(&self) -> Result<&Operator, AppError> {
        self.op.as_ref().ok_or_else(|| {
            AppError::NotFound(format!("Storage location unavailable: {}", self.display))
        })
    }

    fn dir_base(&self) -> String {
        normalize_dir_path(&self.base_path)
    }

    fn from_legacy_file(legacy_path: &str, name: &str) -> Result<Self, AppError> {
        if legacy_path.is_empty() {
            return Ok(Self::unavailable(
                name,
                legacy_path,
                Duration::from_secs(30),
            ));
        }
        let path = PathBuf::from(legacy_path);
        let absolute = absolute_path(&path)?;
        let root = absolute
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let file_name = absolute
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or_else(|| AppError::ParseError(format!("Invalid file path: {}", legacy_path)))?
            .to_string();
        let op = build_operator("fs", map_with_root(&root))?;
        Ok(Self {
            op: Some(op),
            base_path: normalize_file_path(&file_name),
            scheme: "fs".to_string(),
            display: legacy_path.to_string(),
            local_path: Some(absolute),
            poll_interval: Duration::from_secs(30),
        })
    }

    fn from_legacy_dir(
        legacy_path: &str,
        create_local_root: bool,
        name: &str,
    ) -> Result<Self, AppError> {
        if legacy_path.is_empty() {
            return Ok(Self::unavailable(
                name,
                legacy_path,
                Duration::from_secs(30),
            ));
        }
        let path = absolute_path(&PathBuf::from(legacy_path))?;
        if !create_local_root && !path.exists() {
            return Ok(Self::unavailable(
                name,
                legacy_path,
                Duration::from_secs(30),
            ));
        }
        let op = build_operator("fs", map_with_root(&path))?;
        Ok(Self {
            op: Some(op),
            base_path: String::new(),
            scheme: "fs".to_string(),
            display: legacy_path.to_string(),
            local_path: Some(path),
            poll_interval: Duration::from_secs(30),
        })
    }

    fn from_config(
        config: &StorageConfig,
        legacy_path: &str,
        is_dir: bool,
        create_local_root: bool,
        name: &str,
    ) -> Result<Self, AppError> {
        let scheme = if config.scheme.is_empty() {
            "fs"
        } else {
            config.scheme.as_str()
        };
        let mut map = config.options.clone();
        insert_if_present(&mut map, "endpoint", &config.endpoint);
        insert_if_present(&mut map, "bucket", &config.bucket);
        insert_if_present(&mut map, "container", &config.bucket);
        insert_if_present(&mut map, "region", &config.region);
        insert_if_present(&mut map, "access_key_id", &config.access_key_id);
        insert_if_present(&mut map, "secret_access_key", &config.secret_access_key);
        insert_if_present(&mut map, "access_key_secret", &config.access_key_secret);
        if config.secret_access_key.is_empty() && !config.access_key_secret.is_empty() {
            insert_if_present(&mut map, "secret_access_key", &config.access_key_secret);
        }
        if config.access_key_secret.is_empty() && !config.secret_access_key.is_empty() {
            insert_if_present(&mut map, "access_key_secret", &config.secret_access_key);
        }

        let mut local_path = None;
        let base_path = if scheme == "fs" {
            let Some((root, rel_path, display_path)) =
                resolve_fs_config_paths(config, legacy_path, is_dir, create_local_root, name)?
            else {
                let unavailable_path = if !config.path.is_empty() {
                    config.path.as_str()
                } else if !config.root.is_empty() {
                    config.root.as_str()
                } else {
                    legacy_path
                };
                return Ok(Self::unavailable(
                    name,
                    unavailable_path,
                    Duration::from_secs(config.poll_interval_secs.max(1)),
                ));
            };
            map.insert("root".to_string(), root.to_string_lossy().into_owned());
            local_path = Some(display_path);
            rel_path
        } else {
            insert_if_present(&mut map, "root", &config.root);
            if is_dir {
                normalize_dir_path(&config.path)
            } else {
                resolve_config_file_path(config, legacy_path, name)?
            }
        };

        let op = build_operator(scheme, map)?;
        Ok(Self {
            op: Some(op),
            base_path,
            scheme: scheme.to_string(),
            display: if config.path.is_empty() {
                format!("{} storage for {}", scheme, name)
            } else {
                format!("{}://{}", scheme, config.path)
            },
            local_path,
            poll_interval: Duration::from_secs(config.poll_interval_secs.max(1)),
        })
    }

    fn unavailable(name: &str, path: &str, poll_interval: Duration) -> Self {
        Self {
            op: None,
            base_path: String::new(),
            scheme: "fs".to_string(),
            display: if path.is_empty() {
                format!("{} not configured", name)
            } else {
                path.to_string()
            },
            local_path: None,
            poll_interval,
        }
    }
}

fn build_operator(
    scheme: &str,
    map: impl IntoIterator<Item = (String, String)>,
) -> Result<Operator, AppError> {
    opendal::init_default_registry();
    Operator::via_iter(scheme, map).map_err(AppError::from)
}

fn map_with_root(root: &Path) -> HashMap<String, String> {
    HashMap::from([("root".to_string(), root.to_string_lossy().into_owned())])
}

fn resolve_fs_config_paths(
    config: &StorageConfig,
    legacy_path: &str,
    is_dir: bool,
    create_local_root: bool,
    name: &str,
) -> Result<Option<(PathBuf, String, PathBuf)>, AppError> {
    if !config.root.is_empty() {
        let root = absolute_path(&PathBuf::from(&config.root))?;
        let rel = if is_dir {
            normalize_dir_path(&config.path)
        } else {
            resolve_config_file_path(config, legacy_path, name)?
        };
        let display = if rel.is_empty() {
            root.clone()
        } else {
            root.join(&rel)
        };
        if is_dir && !create_local_root && !display.exists() {
            return Ok(None);
        }
        return Ok(Some((root, rel, display)));
    }

    if legacy_path.is_empty() {
        return Err(AppError::ParseError(format!(
            "{} fs storage requires root/path or legacy path",
            name
        )));
    }

    let absolute = absolute_path(&PathBuf::from(legacy_path))?;
    if is_dir {
        if !create_local_root && !absolute.exists() {
            return Ok(None);
        }
        Ok(Some((absolute.clone(), String::new(), absolute)))
    } else {
        let root = absolute
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let file_name = absolute
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or_else(|| AppError::ParseError(format!("Invalid file path: {}", legacy_path)))?
            .to_string();
        Ok(Some((root, normalize_file_path(&file_name), absolute)))
    }
}

fn resolve_config_file_path(
    config: &StorageConfig,
    legacy_path: &str,
    name: &str,
) -> Result<String, AppError> {
    if !config.path.is_empty() {
        return Ok(normalize_file_path(&config.path));
    }
    if !legacy_path.is_empty() {
        return legacy_file_name(legacy_path);
    }
    Err(AppError::ParseError(format!(
        "{} storage requires path or legacy file path",
        name
    )))
}

fn legacy_file_name(legacy_path: &str) -> Result<String, AppError> {
    Path::new(legacy_path)
        .file_name()
        .and_then(|v| v.to_str())
        .map(normalize_file_path)
        .ok_or_else(|| AppError::ParseError(format!("Invalid file path: {}", legacy_path)))
}

fn absolute_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn insert_if_present(map: &mut HashMap<String, String>, key: &str, value: &str) {
    if !value.is_empty() {
        map.insert(key.to_string(), value.to_string());
    }
}

fn normalize_file_path(path: &str) -> String {
    path.trim_start_matches('/')
        .trim_end_matches('/')
        .to_string()
}

fn normalize_dir_path(path: &str) -> String {
    let trimmed = path.trim_start_matches('/').trim_end_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{}/", trimmed)
    }
}

fn join_path(base: &str, child: &str) -> String {
    let child = normalize_file_path(child);
    if base.is_empty() {
        child
    } else {
        format!("{}{}", normalize_dir_path(base), child)
    }
}

fn is_file_entry(entry: &Entry) -> bool {
    entry.metadata().is_file()
}

fn metadata_fingerprint(meta: &Metadata) -> String {
    let last_modified = meta
        .last_modified()
        .map(|v| v.to_string())
        .unwrap_or_default();
    let etag = meta.etag().unwrap_or_default();
    format!("{}:{}:{}", meta.content_length(), last_modified, etag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn legacy_file_read_write_and_sibling() -> anyhow::Result<()> {
        let tmp = tempdir();
        let version_path = tmp.join("version.json");
        let store = StorageLocation::file(
            &StorageConfig::default(),
            version_path.to_str().unwrap(),
            "version_path",
        )?;

        store
            .write_base(br#"{"appVersion":"1.0.0"}"#.to_vec())
            .await?;
        assert_eq!(
            store.read_base().await?,
            br#"{"appVersion":"1.0.0"}"#.to_vec()
        );

        store
            .write_sibling("1.0.0.json", b"snapshot".to_vec())
            .await?;
        assert_eq!(std::fs::read(tmp.join("1.0.0.json"))?, b"snapshot");
        Ok(())
    }

    #[tokio::test]
    async fn legacy_dir_lists_json_files() -> anyhow::Result<()> {
        let tmp = tempdir();
        std::fs::write(tmp.join("a.json"), "{}")?;
        std::fs::write(tmp.join("b.txt"), "ignored")?;

        let store = StorageLocation::dir(
            &StorageConfig::default(),
            tmp.to_str().unwrap(),
            false,
            "account_dir",
        )?;
        let entries = store.list_json_files().await?;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "a.json");
        Ok(())
    }

    #[tokio::test]
    async fn configured_fs_storage_uses_root_and_path() -> anyhow::Result<()> {
        let tmp = tempdir();
        std::fs::create_dir_all(tmp.join("nested"))?;
        let cfg = StorageConfig {
            scheme: "fs".to_string(),
            root: tmp.to_string_lossy().into_owned(),
            path: "nested/version.json".to_string(),
            ..StorageConfig::default()
        };
        let store = StorageLocation::file(&cfg, "", "version_path")?;
        store.write_base(b"ok".to_vec()).await?;
        assert_eq!(std::fs::read(tmp.join("nested/version.json"))?, b"ok");
        Ok(())
    }

    #[tokio::test]
    async fn configured_file_storage_falls_back_to_legacy_file_name() -> anyhow::Result<()> {
        let tmp = tempdir();
        let cfg = StorageConfig {
            scheme: "fs".to_string(),
            root: tmp.to_string_lossy().into_owned(),
            ..StorageConfig::default()
        };
        let store = StorageLocation::file(&cfg, "Data/version/jp.json", "version_path")?;

        store.write_base(b"ok".to_vec()).await?;
        assert_eq!(std::fs::read(tmp.join("jp.json"))?, b"ok");
        Ok(())
    }

    #[tokio::test]
    async fn missing_read_only_fs_dir_is_unavailable() -> anyhow::Result<()> {
        let tmp = tempdir();
        let missing = tmp.join("accounts");
        let cfg = StorageConfig {
            scheme: "fs".to_string(),
            root: tmp.to_string_lossy().into_owned(),
            path: "accounts".to_string(),
            ..StorageConfig::default()
        };
        let store = StorageLocation::dir(&cfg, "", false, "account_dir")?;

        assert!(!store.is_available());
        assert!(!missing.exists());
        assert!(store.list_json_files().await?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn writable_legacy_dir_creates_root_on_write() -> anyhow::Result<()> {
        let tmp = tempdir();
        let master_dir = tmp.join("new-master");
        let store = StorageLocation::dir(
            &StorageConfig::default(),
            master_dir.to_str().unwrap(),
            true,
            "master_dir",
        )?;

        store.write_child("music.json", b"[]".to_vec()).await?;
        assert_eq!(std::fs::read(master_dir.join("music.json"))?, b"[]");
        Ok(())
    }

    fn tempdir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "haruki-storage-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}
