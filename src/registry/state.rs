//! On-disk registry state. Deliberately plain JSON under `registry.state_dir`
//! so the skeleton needs no database; the layout is
//!
//! ```text
//! <state_dir>/manifests/<region>/current.json   latest published manifest
//! <state_dir>/manifests/<region>/history.jsonl  one summary line per publish
//! <state_dir>/app/<region>.json                 app-identity override
//! ```
//!
//! Every write goes through temp+rename so readers never see a torn file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::api::internal::MasterManifest;
use crate::client::helper::write_file_atomic;
use crate::config::ServerRegion;
use crate::error::AppError;
use crate::updater::apphash::AppInfo;

/// One publish, as recorded in the history log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishRecord {
    pub data_version: String,
    pub asset_version: String,
    pub app_version: String,
    pub cdn_version: i32,
    pub published_at: String,
    pub file_count: usize,
    pub total_size: u64,
    /// SHA-256 over the sorted `name:sha256` lines of the manifest; two
    /// manifests with the same content hash carry identical files.
    pub content_hash: String,
}

impl PublishRecord {
    pub fn from_manifest(manifest: &MasterManifest) -> Self {
        Self {
            data_version: manifest.data_version.clone(),
            asset_version: manifest.asset_version.clone(),
            app_version: manifest.app_version.clone(),
            cdn_version: manifest.cdn_version,
            published_at: manifest.generated_at.clone(),
            file_count: manifest.files.len(),
            total_size: manifest.files.iter().map(|f| f.size).sum(),
            content_hash: content_hash(manifest),
        }
    }
}

/// Digest of the file set (names and digests only), independent of the
/// manifest's timestamp.
pub fn content_hash(manifest: &MasterManifest) -> String {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    for file in &manifest.files {
        hasher.update(file.name.as_bytes());
        hasher.update(b":");
        hasher.update(file.sha256.as_bytes());
        hasher.update(b"\n");
    }
    hex::encode(hasher.finalize())
}

#[derive(Debug, Clone)]
pub struct RegistryState {
    dir: PathBuf,
}

impl RegistryState {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn manifest_dir(&self, region: ServerRegion) -> PathBuf {
        self.dir.join("manifests").join(region.as_str())
    }

    fn current_path(&self, region: ServerRegion) -> PathBuf {
        self.manifest_dir(region).join("current.json")
    }

    fn history_path(&self, region: ServerRegion) -> PathBuf {
        self.manifest_dir(region).join("history.jsonl")
    }

    fn app_path(&self, region: ServerRegion) -> PathBuf {
        self.dir
            .join("app")
            .join(format!("{}.json", region.as_str()))
    }

    /// The latest published manifest, if any.
    pub async fn current(&self, region: ServerRegion) -> Result<Option<MasterManifest>, AppError> {
        read_json(&self.current_path(region)).await
    }

    /// Publish history, newest first, at most `limit` entries.
    pub async fn history(
        &self,
        region: ServerRegion,
        limit: usize,
    ) -> Result<Vec<PublishRecord>, AppError> {
        let path = self.history_path(region);
        let text = match tokio::fs::read_to_string(&path).await {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut records: Vec<PublishRecord> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        records.reverse();
        records.truncate(limit);
        Ok(records)
    }

    /// Record `manifest` as the region's current state. Returns `true` when
    /// it differs from the previous publish (version or file set), in which
    /// case a history line is appended; an unchanged republish only refreshes
    /// `current.json`.
    pub async fn publish(
        &self,
        region: ServerRegion,
        manifest: &MasterManifest,
    ) -> Result<bool, AppError> {
        let record = PublishRecord::from_manifest(manifest);
        let previous = self.current(region).await?;
        let changed = match previous {
            Some(prev) => {
                prev.data_version != manifest.data_version
                    || prev.cdn_version != manifest.cdn_version
                    || content_hash(&prev) != record.content_hash
            }
            None => true,
        };
        tokio::fs::create_dir_all(self.manifest_dir(region)).await?;
        let json = serde_json::to_vec_pretty(manifest)
            .map_err(|e| AppError::ParseError(format!("manifest: {e}")))?;
        write_file_atomic(&self.current_path(region), &json).await?;
        if changed {
            use tokio::io::AsyncWriteExt;
            let mut line = serde_json::to_string(&record)
                .map_err(|e| AppError::ParseError(format!("publish record: {e}")))?;
            line.push('\n');
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.history_path(region))
                .await?;
            file.write_all(line.as_bytes()).await?;
            file.flush().await?;
        }
        Ok(changed)
    }

    /// The operator-set app identity for a region, if one was stored.
    pub async fn app_identity(&self, region: ServerRegion) -> Result<Option<AppInfo>, AppError> {
        read_json(&self.app_path(region)).await
    }

    pub async fn set_app_identity(
        &self,
        region: ServerRegion,
        info: &AppInfo,
    ) -> Result<(), AppError> {
        let path = self.app_path(region);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let json = serde_json::to_vec_pretty(info)
            .map_err(|e| AppError::ParseError(format!("app identity: {e}")))?;
        write_file_atomic(&path, &json).await?;
        Ok(())
    }

    pub async fn clear_app_identity(&self, region: ServerRegion) -> Result<bool, AppError> {
        match tokio::fs::remove_file(self.app_path(region)).await {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}

async fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, AppError> {
    match tokio::fs::read(path).await {
        Ok(data) => serde_json::from_slice(&data)
            .map(Some)
            .map_err(|e| AppError::ParseError(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::internal::MasterManifestFile;

    fn manifest(data_version: &str, files: &[(&str, &str)]) -> MasterManifest {
        MasterManifest {
            server: "jp".to_string(),
            app_version: "5.6.0".to_string(),
            app_hash: "h".to_string(),
            data_version: data_version.to_string(),
            asset_version: "5.6.1.10".to_string(),
            asset_hash: "a".to_string(),
            cdn_version: 0,
            generated_at: "2026-09-11T00:00:00Z".to_string(),
            files: files
                .iter()
                .map(|(name, sha)| MasterManifestFile {
                    name: name.to_string(),
                    size: 3,
                    sha256: sha.to_string(),
                })
                .collect(),
        }
    }

    #[tokio::test]
    async fn publishes_tracks_history_and_stores_app_identity() {
        let root = std::env::temp_dir().join(format!("haruki_registry_{}", uuid::Uuid::new_v4()));
        let state = RegistryState::new(&root);
        assert!(state.current(ServerRegion::Jp).await.unwrap().is_none());
        assert!(state
            .history(ServerRegion::Jp, 10)
            .await
            .unwrap()
            .is_empty());

        let first = manifest("1", &[("cards.json", "aa")]);
        assert!(state.publish(ServerRegion::Jp, &first).await.unwrap());
        // Same version and files: current refreshed, no new history line.
        let mut same = first.clone();
        same.generated_at = "2026-09-11T00:00:01Z".to_string();
        assert!(!state.publish(ServerRegion::Jp, &same).await.unwrap());
        assert_eq!(
            state
                .current(ServerRegion::Jp)
                .await
                .unwrap()
                .unwrap()
                .generated_at,
            same.generated_at
        );
        // A changed file with the same version still counts as a publish.
        let changed = manifest("1", &[("cards.json", "bb")]);
        assert!(state.publish(ServerRegion::Jp, &changed).await.unwrap());
        let newer = manifest("2", &[("cards.json", "bb")]);
        assert!(state.publish(ServerRegion::Jp, &newer).await.unwrap());
        let history = state.history(ServerRegion::Jp, 10).await.unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].data_version, "2");
        assert_eq!(history[2].content_hash, content_hash(&first));
        assert_eq!(state.history(ServerRegion::Jp, 1).await.unwrap().len(), 1);
        assert_ne!(content_hash(&first), content_hash(&changed));

        assert!(state
            .app_identity(ServerRegion::En)
            .await
            .unwrap()
            .is_none());
        state
            .set_app_identity(
                ServerRegion::En,
                &AppInfo {
                    app_version: "5.7.0".to_string(),
                    app_hash: "new".to_string(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            state
                .app_identity(ServerRegion::En)
                .await
                .unwrap()
                .unwrap()
                .app_version,
            "5.7.0"
        );
        assert!(state.clear_app_identity(ServerRegion::En).await.unwrap());
        assert!(!state.clear_app_identity(ServerRegion::En).await.unwrap());

        // Corrupt state surfaces as a parse error rather than a silent None.
        std::fs::write(root.join("manifests/jp/current.json"), "{").unwrap();
        assert!(matches!(
            state.current(ServerRegion::Jp).await,
            Err(AppError::ParseError(_))
        ));
        std::fs::remove_dir_all(root).unwrap();
    }
}
