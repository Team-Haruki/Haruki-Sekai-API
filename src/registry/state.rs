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
/// manifest's timestamp. Uses the manifest's recorded hash when present.
pub fn content_hash(manifest: &MasterManifest) -> String {
    if !manifest.content_hash.is_empty() {
        return manifest.content_hash.clone();
    }
    crate::api::internal::manifest_content_hash(&manifest.files)
}

/// Immutable manifest snapshots kept per region (newest publishes).
const MANIFEST_SNAPSHOTS_KEPT: usize = 20;

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

    fn snapshot_dir(&self, region: ServerRegion) -> PathBuf {
        self.manifest_dir(region).join("by-hash")
    }

    /// An immutable manifest snapshot by content hash, if still kept.
    pub async fn manifest_by_hash(
        &self,
        region: ServerRegion,
        content_hash: &str,
    ) -> Result<Option<MasterManifest>, AppError> {
        if !is_hex_digest(content_hash) {
            return Ok(None);
        }
        read_json(
            &self
                .snapshot_dir(region)
                .join(format!("{content_hash}.json")),
        )
        .await
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
        tokio::fs::create_dir_all(self.snapshot_dir(region)).await?;
        let json = serde_json::to_vec_pretty(manifest)
            .map_err(|e| AppError::ParseError(format!("manifest: {e}")))?;
        write_file_atomic(&self.current_path(region), &json).await?;
        if is_hex_digest(&record.content_hash) {
            write_file_atomic(
                &self
                    .snapshot_dir(region)
                    .join(format!("{}.json", record.content_hash)),
                &json,
            )
            .await?;
            self.prune_snapshots(region).await;
        }
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

    /// Keep only the newest `MANIFEST_SNAPSHOTS_KEPT` snapshots (by mtime).
    async fn prune_snapshots(&self, region: ServerRegion) {
        let Ok(mut rd) = tokio::fs::read_dir(self.snapshot_dir(region)).await else {
            return;
        };
        let mut entries = Vec::new();
        while let Ok(Some(entry)) = rd.next_entry().await {
            if let Ok(meta) = entry.metadata().await {
                let modified = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                entries.push((modified, entry.path()));
            }
        }
        if entries.len() <= MANIFEST_SNAPSHOTS_KEPT {
            return;
        }
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        for (_, path) in entries.into_iter().skip(MANIFEST_SNAPSHOTS_KEPT) {
            let _ = tokio::fs::remove_file(path).await;
        }
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

/// A lowercase hex SHA-256, the only shape accepted in digest-keyed paths.
pub fn is_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
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
            content_hash: String::new(),
            git_commit: None,
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
        // Every publish left an immutable snapshot addressable by content
        // hash (two publishes with the same file set share one snapshot).
        for record in &history {
            let snap = state
                .manifest_by_hash(ServerRegion::Jp, &record.content_hash)
                .await
                .unwrap()
                .expect("snapshot kept");
            assert_eq!(content_hash(&snap), record.content_hash);
        }
        assert!(state
            .manifest_by_hash(ServerRegion::Jp, "../current")
            .await
            .unwrap()
            .is_none());
        assert!(is_hex_digest(&history[0].content_hash));
        assert!(!is_hex_digest("ABC"));
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
