//! Registry state: the published manifests (current + immutable snapshots by
//! content hash), the publish history, app-identity overrides and the
//! music_metas pointers.
//!
//! Two backends share one API:
//!
//! - **Files** (default, `registry.state_dsn` empty): plain JSON under
//!   `registry.state_dir`, every write through temp+rename so readers never
//!   see a torn file:
//!
//!   ```text
//!   <state_dir>/manifests/<region>/current.json          latest published manifest
//!   <state_dir>/manifests/<region>/history.jsonl         one summary line per publish
//!   <state_dir>/manifests/<region>/by-hash/<hash>.json   immutable manifest snapshots
//!   <state_dir>/app/<region>.json                        app-identity override
//!   <state_dir>/metas/<region>/current.json              music_metas pointer
//!   ```
//!
//! - **Database** (`registry.state_dsn` set, PostgreSQL in production): the
//!   same documents in `registry_state` keyed by `(region, kind, name)` and
//!   the history in `registry_publish_history`. A publish (current, snapshot,
//!   snapshot pruning, history line) is one transaction. On first start with
//!   empty tables the files above are imported once and left in place.
//!
//! music_metas blobs are content, not state: they stay under
//! `<state_dir>/metas/<region>/` with either backend. Master file content
//! lives in the master directories, or in `registry_blobs` next to these
//! tables with `registry.blob_store: pg` (`registry::blobs`); then a publish
//! commits only when every file it lists is already stored.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;

use chrono::{DateTime, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::api::internal::MasterManifest;
use crate::client::helper::write_file_atomic;
use crate::client::helper::AppInfo;
use crate::config::ServerRegion;
use crate::db::entity::{registry_publish_history, registry_state};
use crate::error::AppError;

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

const ALL_REGIONS: [ServerRegion; 5] = [
    ServerRegion::Jp,
    ServerRegion::En,
    ServerRegion::Tw,
    ServerRegion::Kr,
    ServerRegion::Cn,
];

/// `registry_state.kind` values.
const KIND_MANIFEST: &str = "manifest";
const KIND_SNAPSHOT: &str = "manifest_snapshot";
const KIND_APP: &str = "app_identity";
const KIND_METAS: &str = "metas_record";
/// `registry_state.name` of the singleton documents.
const NAME_CURRENT: &str = "current";

#[derive(Debug, Clone)]
pub struct RegistryState {
    dir: PathBuf,
    /// Set when the state lives in a database instead of `dir`.
    db: Option<DatabaseConnection>,
    /// Database backend only: the singleton documents (current manifest, app
    /// identity, metas pointer) as last read or written, so the hot read
    /// paths (`current`, every `blob/` lookup) do not query the database per
    /// request and keep answering through a transient database outage. This
    /// instance is the only writer (one registry per state database), so the
    /// cache is refreshed by its own publishes and writes.
    cache: SingletonCache,
    /// Database backend with `registry.blob_store: pg`: a publish refuses to
    /// commit a manifest listing a digest missing from `registry_blobs`.
    require_blobs: bool,
}

type SingletonCache = Arc<RwLock<HashMap<(ServerRegion, &'static str), Option<serde_json::Value>>>>;

impl RegistryState {
    /// File-backed state under `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            db: None,
            cache: Arc::default(),
            require_blobs: false,
        }
    }

    /// Database-backed state: connect to `dsn`, create the tables when
    /// missing and, when they are empty, import the file state under `dir`
    /// once. `dir` still holds the music_metas blobs.
    pub async fn connect(dir: impl Into<PathBuf>, dsn: &str) -> Result<Self, AppError> {
        let db = crate::db::init_registry_state_db(dsn).await?;
        let state = Self {
            dir: dir.into(),
            db: Some(db),
            cache: Arc::default(),
            require_blobs: false,
        };
        state.import_files_if_empty().await?;
        state.load_cache().await?;
        Ok(state)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Whether the state lives in a database (else in files under `dir`).
    pub fn is_database(&self) -> bool {
        self.db.is_some()
    }

    /// The state database connection, when the state lives in one.
    pub fn database(&self) -> Option<&DatabaseConnection> {
        self.db.as_ref()
    }

    /// Make every database publish check, inside its transaction, that the
    /// manifest's files are all in `registry_blobs` (`blob_store: pg`).
    pub fn require_blobs(&mut self) {
        self.require_blobs = true;
    }

    /// The retained manifest snapshots of a region, newest first.
    pub async fn snapshots(&self, region: ServerRegion) -> Result<Vec<MasterManifest>, AppError> {
        if let Some(db) = &self.db {
            let rows = registry_state::Entity::find()
                .filter(registry_state::Column::Region.eq(region.as_str()))
                .filter(registry_state::Column::Kind.eq(KIND_SNAPSHOT))
                .order_by_desc(registry_state::Column::UpdatedAt)
                .order_by_desc(registry_state::Column::Name)
                .all(db)
                .await?;
            return Ok(rows
                .into_iter()
                .filter_map(|row| serde_json::from_value(row.value).ok())
                .collect());
        }
        let mut entries = Vec::new();
        if let Ok(mut rd) = tokio::fs::read_dir(self.snapshot_dir(region)).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                let modified = entry
                    .metadata()
                    .await
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                entries.push((modified, entry.path()));
            }
        }
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        let mut manifests = Vec::new();
        for (_, path) in entries {
            if let Ok(Some(manifest)) = read_json::<MasterManifest>(&path).await {
                manifests.push(manifest);
            }
        }
        Ok(manifests)
    }

    /// Every file digest a region's current manifest or a retained snapshot
    /// lists, over all regions: the blobs garbage collection must keep.
    pub async fn referenced_digests(&self) -> Result<HashSet<String>, AppError> {
        let mut digests = HashSet::new();
        for region in ALL_REGIONS {
            let current = self.current(region).await?;
            for manifest in current.into_iter().chain(self.snapshots(region).await?) {
                digests.extend(manifest.files.into_iter().map(|f| f.sha256));
            }
        }
        Ok(digests)
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

    fn app_path(&self, region: ServerRegion) -> PathBuf {
        self.dir
            .join("app")
            .join(format!("{}.json", region.as_str()))
    }

    fn metas_record_path(&self, region: ServerRegion) -> PathBuf {
        self.dir
            .join("metas")
            .join(region.as_str())
            .join("current.json")
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
        if let Some(db) = &self.db {
            return db_get(db, region, KIND_SNAPSHOT, content_hash).await;
        }
        read_json(
            &self
                .snapshot_dir(region)
                .join(format!("{content_hash}.json")),
        )
        .await
    }

    /// The latest published manifest, if any.
    pub async fn current(&self, region: ServerRegion) -> Result<Option<MasterManifest>, AppError> {
        if self.db.is_some() {
            return self.cached(region, KIND_MANIFEST).await;
        }
        read_json(&self.current_path(region)).await
    }

    /// Publish history, newest first, at most `limit` entries.
    pub async fn history(
        &self,
        region: ServerRegion,
        limit: usize,
    ) -> Result<Vec<PublishRecord>, AppError> {
        if let Some(db) = &self.db {
            let rows = registry_publish_history::Entity::find()
                .filter(registry_publish_history::Column::Region.eq(region.as_str()))
                .order_by_desc(registry_publish_history::Column::Id)
                .limit(limit as u64)
                .all(db)
                .await?;
            return Ok(rows
                .into_iter()
                .filter_map(|row| serde_json::from_value(row.record).ok())
                .collect());
        }
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
    /// `current`.
    pub async fn publish(
        &self,
        region: ServerRegion,
        manifest: &MasterManifest,
    ) -> Result<bool, AppError> {
        let record = PublishRecord::from_manifest(manifest);
        if let Some(db) = &self.db {
            return self.publish_db(db, region, manifest, &record).await;
        }
        let previous = self.current(region).await?;
        let changed = is_changed(previous.as_ref(), manifest, &record);
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

    /// The database publish: current, snapshot, snapshot pruning and the
    /// history line commit together or not at all.
    async fn publish_db(
        &self,
        db: &DatabaseConnection,
        region: ServerRegion,
        manifest: &MasterManifest,
        record: &PublishRecord,
    ) -> Result<bool, AppError> {
        let txn = db.begin().await?;
        if self.require_blobs {
            let digests: Vec<String> = manifest.files.iter().map(|f| f.sha256.clone()).collect();
            let missing = super::blobs::missing_digests(&txn, &digests).await?;
            if !missing.is_empty() {
                return Err(AppError::Internal(format!(
                    "{} publish refused: {} listed file(s) missing from registry_blobs",
                    region.as_str(),
                    missing.len()
                )));
            }
        }
        let previous: Option<MasterManifest> =
            db_get(&txn, region, KIND_MANIFEST, NAME_CURRENT).await?;
        let changed = is_changed(previous.as_ref(), manifest, record);
        let now = Utc::now();
        let value = to_json(manifest, "manifest")?;
        let value_for_cache = value.clone();
        db_put(
            &txn,
            region,
            KIND_MANIFEST,
            NAME_CURRENT,
            value.clone(),
            now,
        )
        .await?;
        if is_hex_digest(&record.content_hash) {
            db_put(
                &txn,
                region,
                KIND_SNAPSHOT,
                &record.content_hash,
                value,
                now,
            )
            .await?;
            let stale: Vec<String> = registry_state::Entity::find()
                .select_only()
                .column(registry_state::Column::Name)
                .filter(registry_state::Column::Region.eq(region.as_str()))
                .filter(registry_state::Column::Kind.eq(KIND_SNAPSHOT))
                .order_by_desc(registry_state::Column::UpdatedAt)
                .order_by_desc(registry_state::Column::Name)
                .into_tuple::<String>()
                .all(&txn)
                .await?
                .into_iter()
                .skip(MANIFEST_SNAPSHOTS_KEPT)
                .collect();
            if !stale.is_empty() {
                registry_state::Entity::delete_many()
                    .filter(registry_state::Column::Region.eq(region.as_str()))
                    .filter(registry_state::Column::Kind.eq(KIND_SNAPSHOT))
                    .filter(registry_state::Column::Name.is_in(stale))
                    .exec(&txn)
                    .await?;
            }
        }
        if changed {
            registry_publish_history::Entity::insert(registry_publish_history::ActiveModel {
                region: Set(region.as_str().to_string()),
                record: Set(to_json(record, "publish record")?),
                recorded_at: Set(now),
                ..Default::default()
            })
            .exec(&txn)
            .await?;
        }
        txn.commit().await?;
        self.cache
            .write()
            .insert((region, KIND_MANIFEST), Some(value_for_cache));
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
        if self.db.is_some() {
            return self.cached(region, KIND_APP).await;
        }
        read_json(&self.app_path(region)).await
    }

    pub async fn set_app_identity(
        &self,
        region: ServerRegion,
        info: &AppInfo,
    ) -> Result<(), AppError> {
        if let Some(db) = &self.db {
            let value = to_json(info, "app identity")?;
            db_put(
                db,
                region,
                KIND_APP,
                NAME_CURRENT,
                value.clone(),
                Utc::now(),
            )
            .await?;
            self.cache.write().insert((region, KIND_APP), Some(value));
            return Ok(());
        }
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
        if let Some(db) = &self.db {
            let result = registry_state::Entity::delete_many()
                .filter(registry_state::Column::Region.eq(region.as_str()))
                .filter(registry_state::Column::Kind.eq(KIND_APP))
                .filter(registry_state::Column::Name.eq(NAME_CURRENT))
                .exec(db)
                .await?;
            self.cache.write().insert((region, KIND_APP), None);
            return Ok(result.rows_affected > 0);
        }
        match tokio::fs::remove_file(self.app_path(region)).await {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// The music_metas pointer of a region, if one was stored.
    pub async fn metas_record<T: serde::de::DeserializeOwned>(
        &self,
        region: ServerRegion,
    ) -> Result<Option<T>, AppError> {
        if self.db.is_some() {
            return self.cached(region, KIND_METAS).await;
        }
        read_json(&self.metas_record_path(region)).await
    }

    pub async fn set_metas_record<T: Serialize>(
        &self,
        region: ServerRegion,
        record: &T,
    ) -> Result<(), AppError> {
        if let Some(db) = &self.db {
            let value = to_json(record, "metas record")?;
            db_put(
                db,
                region,
                KIND_METAS,
                NAME_CURRENT,
                value.clone(),
                Utc::now(),
            )
            .await?;
            self.cache.write().insert((region, KIND_METAS), Some(value));
            return Ok(());
        }
        let path = self.metas_record_path(region);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let json = serde_json::to_vec_pretty(record)
            .map_err(|e| AppError::ParseError(format!("metas record: {e}")))?;
        write_file_atomic(&path, &json).await?;
        Ok(())
    }

    /// Fill the singleton cache from the database (after the import).
    async fn load_cache(&self) -> Result<(), AppError> {
        let Some(db) = &self.db else {
            return Ok(());
        };
        let mut loaded = HashMap::new();
        for region in ALL_REGIONS {
            for kind in [KIND_MANIFEST, KIND_APP, KIND_METAS] {
                let value: Option<serde_json::Value> =
                    db_get(db, region, kind, NAME_CURRENT).await?;
                loaded.insert((region, kind), value);
            }
        }
        *self.cache.write() = loaded;
        Ok(())
    }

    /// A cached singleton document, read from the database on a miss.
    async fn cached<T: serde::de::DeserializeOwned>(
        &self,
        region: ServerRegion,
        kind: &'static str,
    ) -> Result<Option<T>, AppError> {
        let hit = self.cache.read().get(&(region, kind)).cloned();
        let value = match (hit, &self.db) {
            (Some(value), _) => value,
            (None, Some(db)) => {
                let value: Option<serde_json::Value> =
                    db_get(db, region, kind, NAME_CURRENT).await?;
                self.cache.write().insert((region, kind), value.clone());
                value
            }
            (None, None) => None,
        };
        value
            .map(|v| {
                serde_json::from_value(v).map_err(|e| {
                    AppError::ParseError(format!("registry_state {}/{kind}: {e}", region.as_str()))
                })
            })
            .transpose()
    }

    /// One-time import of the file state into empty database tables, in one
    /// transaction. Unreadable files are skipped with a warning; the files
    /// themselves are never modified or removed.
    async fn import_files_if_empty(&self) -> Result<usize, AppError> {
        let Some(db) = &self.db else {
            return Ok(0);
        };
        let documents = registry_state::Entity::find().count(db).await?;
        let history = registry_publish_history::Entity::find().count(db).await?;
        if documents > 0 || history > 0 {
            return Ok(0);
        }
        let files = RegistryState::new(self.dir.clone());
        let txn = db.begin().await?;
        let mut imported = 0usize;
        for region in ALL_REGIONS {
            let singles = [
                (KIND_MANIFEST, files.current_path(region)),
                (KIND_APP, files.app_path(region)),
                (KIND_METAS, files.metas_record_path(region)),
            ];
            for (kind, path) in singles {
                if let Some((value, at)) = import_document(&path).await {
                    db_put(&txn, region, kind, NAME_CURRENT, value, at).await?;
                    imported += 1;
                }
            }
            if let Ok(mut rd) = tokio::fs::read_dir(files.snapshot_dir(region)).await {
                while let Ok(Some(entry)) = rd.next_entry().await {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let Some(hash) = name.strip_suffix(".json").filter(|h| is_hex_digest(h)) else {
                        continue;
                    };
                    if let Some((value, at)) = import_document(&entry.path()).await {
                        db_put(&txn, region, KIND_SNAPSHOT, hash, value, at).await?;
                        imported += 1;
                    }
                }
            }
            // Oldest first, so ids ascend in publish order like the log.
            let mut records = files.history(region, usize::MAX).await?;
            records.reverse();
            for record in records {
                let at = DateTime::parse_from_rfc3339(&record.published_at)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                registry_publish_history::Entity::insert(registry_publish_history::ActiveModel {
                    region: Set(region.as_str().to_string()),
                    record: Set(to_json(&record, "publish record")?),
                    recorded_at: Set(at),
                    ..Default::default()
                })
                .exec(&txn)
                .await?;
                imported += 1;
            }
        }
        txn.commit().await?;
        if imported > 0 {
            info!(
                "Imported {} registry state entries from {} into the database (files kept)",
                imported,
                self.dir.display()
            );
        }
        Ok(imported)
    }
}

fn is_changed(
    previous: Option<&MasterManifest>,
    manifest: &MasterManifest,
    record: &PublishRecord,
) -> bool {
    match previous {
        Some(prev) => {
            prev.data_version != manifest.data_version
                || prev.cdn_version != manifest.cdn_version
                || content_hash(prev) != record.content_hash
        }
        None => true,
    }
}

fn to_json<T: Serialize>(value: &T, what: &str) -> Result<serde_json::Value, AppError> {
    serde_json::to_value(value).map_err(|e| AppError::ParseError(format!("{what}: {e}")))
}

async fn db_get<C: ConnectionTrait, T: serde::de::DeserializeOwned>(
    conn: &C,
    region: ServerRegion,
    kind: &str,
    name: &str,
) -> Result<Option<T>, AppError> {
    let row = registry_state::Entity::find_by_id((
        region.as_str().to_string(),
        kind.to_string(),
        name.to_string(),
    ))
    .one(conn)
    .await?;
    row.map(|row| {
        serde_json::from_value(row.value).map_err(|e| {
            AppError::ParseError(format!(
                "registry_state {}/{kind}/{name}: {e}",
                region.as_str()
            ))
        })
    })
    .transpose()
}

async fn db_put<C: ConnectionTrait>(
    conn: &C,
    region: ServerRegion,
    kind: &str,
    name: &str,
    value: serde_json::Value,
    at: DateTime<Utc>,
) -> Result<(), AppError> {
    registry_state::Entity::insert(registry_state::ActiveModel {
        region: Set(region.as_str().to_string()),
        kind: Set(kind.to_string()),
        name: Set(name.to_string()),
        value: Set(value),
        updated_at: Set(at),
    })
    .on_conflict(
        OnConflict::columns([
            registry_state::Column::Region,
            registry_state::Column::Kind,
            registry_state::Column::Name,
        ])
        .update_columns([
            registry_state::Column::Value,
            registry_state::Column::UpdatedAt,
        ])
        .to_owned(),
    )
    .exec(conn)
    .await?;
    Ok(())
}

/// A state file's JSON and mtime, or `None` (with a warning when it exists
/// but cannot be read or parsed).
async fn import_document(path: &Path) -> Option<(serde_json::Value, DateTime<Utc>)> {
    let data = match tokio::fs::read(path).await {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            warn!(
                "Skipping unreadable registry state {}: {}",
                path.display(),
                e
            );
            return None;
        }
    };
    let value: serde_json::Value = match serde_json::from_slice(&data) {
        Ok(v) => v,
        Err(e) => {
            warn!("Skipping corrupt registry state {}: {}", path.display(), e);
            return None;
        }
    };
    let at = tokio::fs::metadata(path)
        .await
        .and_then(|m| m.modified())
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|_| Utc::now());
    Some((value, at))
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

    fn hex(n: usize) -> String {
        format!("{:064x}", n)
    }

    /// The database backend against `dsn`, starting from a file state that
    /// must be imported once and then left alone.
    async fn exercise_database_state(dsn: &str) {
        let root =
            std::env::temp_dir().join(format!("haruki_registry_db_{}", uuid::Uuid::new_v4()));
        let files = RegistryState::new(&root);
        let first = manifest("1", &[("cards.json", "aa")]);
        let second = manifest("2", &[("cards.json", "bb")]);
        assert!(files.publish(ServerRegion::Jp, &first).await.unwrap());
        assert!(files.publish(ServerRegion::Jp, &second).await.unwrap());
        let app = AppInfo {
            app_version: "5.7.0".to_string(),
            app_hash: "override".to_string(),
        };
        files
            .set_app_identity(ServerRegion::Kr, &app)
            .await
            .unwrap();
        let metas = serde_json::json!({"region": "jp", "sha256": hex(7), "rows": 3});
        files
            .set_metas_record(ServerRegion::Jp, &metas)
            .await
            .unwrap();

        let state = RegistryState::connect(&root, dsn).await.unwrap();
        assert!(state.is_database());
        // Imported: every document and the history in publish order.
        assert_eq!(
            state
                .current(ServerRegion::Jp)
                .await
                .unwrap()
                .unwrap()
                .data_version,
            "2"
        );
        let history = state.history(ServerRegion::Jp, 10).await.unwrap();
        assert_eq!(history, files.history(ServerRegion::Jp, 10).await.unwrap());
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].data_version, "2");
        for record in &history {
            assert!(state
                .manifest_by_hash(ServerRegion::Jp, &record.content_hash)
                .await
                .unwrap()
                .is_some());
        }
        assert_eq!(
            state.app_identity(ServerRegion::Kr).await.unwrap(),
            Some(app.clone())
        );
        assert_eq!(
            state
                .metas_record::<serde_json::Value>(ServerRegion::Jp)
                .await
                .unwrap(),
            Some(metas.clone())
        );
        assert!(state.current(ServerRegion::En).await.unwrap().is_none());

        // Same publish semantics as the files: an unchanged republish only
        // refreshes current, a changed file set appends history.
        let mut same = second.clone();
        same.generated_at = "2026-09-11T00:00:09Z".to_string();
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
        assert_eq!(state.history(ServerRegion::Jp, 10).await.unwrap().len(), 2);
        assert!(state
            .publish(ServerRegion::Jp, &manifest("2", &[("cards.json", "cc")]))
            .await
            .unwrap());
        assert_eq!(state.history(ServerRegion::Jp, 10).await.unwrap().len(), 3);
        assert_eq!(state.history(ServerRegion::Jp, 1).await.unwrap().len(), 1);
        assert!(state
            .history(ServerRegion::En, 10)
            .await
            .unwrap()
            .is_empty());
        // The files are left untouched by database writes.
        assert_eq!(files.history(ServerRegion::Jp, 10).await.unwrap().len(), 2);

        // Snapshots are capped at the newest MANIFEST_SNAPSHOTS_KEPT.
        let mut hashes = Vec::new();
        for n in 0..(MANIFEST_SNAPSHOTS_KEPT + 3) {
            let mut m = manifest("3", &[("cards.json", "dd")]);
            m.content_hash = hex(1000 + n);
            hashes.push(m.content_hash.clone());
            assert!(state.publish(ServerRegion::Tw, &m).await.unwrap());
        }
        let mut kept = 0;
        for hash in &hashes {
            if state
                .manifest_by_hash(ServerRegion::Tw, hash)
                .await
                .unwrap()
                .is_some()
            {
                kept += 1;
            }
        }
        assert_eq!(kept, MANIFEST_SNAPSHOTS_KEPT);
        assert!(state
            .manifest_by_hash(ServerRegion::Tw, hashes.last().unwrap())
            .await
            .unwrap()
            .is_some());
        assert!(state
            .manifest_by_hash(ServerRegion::Tw, &hashes[0])
            .await
            .unwrap()
            .is_none());
        assert!(state
            .manifest_by_hash(ServerRegion::Jp, "../current")
            .await
            .unwrap()
            .is_none());

        assert!(state.clear_app_identity(ServerRegion::Kr).await.unwrap());
        assert!(!state.clear_app_identity(ServerRegion::Kr).await.unwrap());
        assert!(state
            .app_identity(ServerRegion::Kr)
            .await
            .unwrap()
            .is_none());
        state
            .set_metas_record(ServerRegion::Jp, &serde_json::json!({"sha256": hex(8)}))
            .await
            .unwrap();

        // Reads of the singletons are served from memory, so a database
        // outage does not fail `current` (or the blob lookups built on it).
        let offline = state.clone();
        if let Some(db) = &offline.db {
            db.clone().close().await.unwrap();
        }
        assert!(offline.current(ServerRegion::Jp).await.unwrap().is_some());
        assert!(offline.current(ServerRegion::En).await.unwrap().is_none());
        assert!(offline
            .metas_record::<serde_json::Value>(ServerRegion::Jp)
            .await
            .unwrap()
            .is_some());

        // A restart does not import again: the database stays authoritative.
        let again = RegistryState::connect(&root, dsn).await.unwrap();
        assert_eq!(again.history(ServerRegion::Jp, 10).await.unwrap().len(), 3);
        assert!(again
            .app_identity(ServerRegion::Kr)
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            again
                .metas_record::<serde_json::Value>(ServerRegion::Jp)
                .await
                .unwrap(),
            Some(serde_json::json!({"sha256": hex(8)}))
        );
        assert!(root.join("app/kr.json").exists(), "files are kept");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn database_state_on_sqlite() {
        let db = std::env::temp_dir().join(format!("haruki_registry_{}.db", uuid::Uuid::new_v4()));
        exercise_database_state(&format!("sqlite://{}?mode=rwc", db.display())).await;
        let _ = std::fs::remove_file(db);
    }

    /// Set `HARUKI_TEST_REGISTRY_DSN` to a scratch PostgreSQL database (its
    /// registry tables are dropped first), then `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore] // Requires a running PostgreSQL
    async fn database_state_on_postgres() {
        let dsn = std::env::var("HARUKI_TEST_REGISTRY_DSN")
            .unwrap_or_else(|_| "postgres://haruki:sekai@localhost:5432/registry_test".to_string());
        let db = sea_orm::Database::connect(&dsn).await.unwrap();
        db.execute_unprepared(
            "DROP TABLE IF EXISTS registry_state; DROP TABLE IF EXISTS registry_publish_history;",
        )
        .await
        .unwrap();
        exercise_database_state(&dsn).await;
    }
}
