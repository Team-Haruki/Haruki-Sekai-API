//! Registry master file content store (`registry.blob_store`).
//!
//! - **fs** (default): the region's master directory (the git worktree) is
//!   the store. `blob/{sha256}` serves only files of the current manifest
//!   whose bytes on disk still hash to that digest; nothing is copied.
//! - **pg**: every file a publish lists is stored content-addressed in the
//!   `registry_blobs` table of the registry state database, zstd-compressed
//!   (one frame, content checksum), before the manifest that lists it is
//!   committed (the publish transaction re-checks that every digest is
//!   present). Identical files across versions and regions are one row.
//!   Responses are decompressed on the fly, so bodies and headers match the
//!   fs store. Any retained blob is addressable, including files of retained
//!   snapshots; garbage collection removes blobs that no region's current or
//!   retained snapshot manifest lists and that no publish touched within the
//!   grace period. When a blob is missing (import still running) or the
//!   database is unreachable, reads of current files fall back to the master
//!   directory with the fs store's digest check. A read gives the database
//!   at most [`QUERY_TIMEOUT`] (slot waits included); a failed or timed-out
//!   query opens a short circuit breaker ([`BREAKER_OPEN`]) during which
//!   reads skip the database and go straight to the fallback.
//!
//! Memory stays bounded: files are hashed and compressed one at a time from a
//! streaming reader, and at most [`READ_CONCURRENCY`] compressed blobs are
//! held for responses at once (the 55 MB costume3ds file is ~1.5 MB stored;
//! the decoder window is capped at 1 MiB). A bundle loads the compressed
//! blobs of its manifest before answering (~8 MB for a full region), at most
//! [`BUNDLE_CONCURRENCY`] at once, so it never fails after its 200.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use axum::body::{Body, Bytes};
use chrono::Utc;
use futures::future::BoxFuture;
use sea_orm::sea_query::{Expr, LockType, OnConflict};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseConnection,
    EntityTrait, QueryFilter, QuerySelect,
};
use sha2::Digest as _;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;
use tracing::{debug, info, warn};

use super::state::{is_hex_digest, RegistryState};
use crate::api::internal::{file_sha256, MasterManifest, MasterManifestFile};
use crate::config::{BlobStoreKind, RegistryConfig};
use crate::db::entity::registry_blob;
use crate::error::AppError;
use crate::updater::master::is_safe_path_component;

/// `registry_blobs.encoding` of every stored blob.
pub const ENCODING_ZSTD: &str = "zstd";
/// Level 9 stores the real master set at ~1/35 of its size (level 3: ~1/29,
/// level 19 costs 50x the CPU for another 20%).
const ZSTD_LEVEL: i32 = 9;
/// 1 MiB window: bounds the decoder's memory per response for ~2% ratio.
const ZSTD_WINDOW_LOG: u32 = 20;
/// Body chunk size of decoded responses and of the bundle stream.
const CHUNK: usize = 64 * 1024;
/// Digests per `IN (...)` list.
const DIGEST_BATCH: usize = 500;
/// Blobs deleted per garbage collection run.
const GC_BATCH: usize = 500;
/// Blob responses in flight at once (each holds its compressed bytes, at
/// most ~1.5 MB, plus a 1 MiB decoder window until the body is sent): high
/// enough that slow clients cannot starve the rest, ~40 MB worst case.
pub const READ_CONCURRENCY: usize = 16;
/// Longest a read spends on the database: read slot, query slot and the
/// query together. Past it the read fails like a database error, so a
/// current file is served from disk quickly when the database hangs.
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(2);
/// After a failed or timed-out blob query, reads skip the database for this
/// long (current files come from disk, other reads answer 503 at once).
pub const BREAKER_OPEN: Duration = Duration::from_secs(5);
/// Blob row queries at once: below the state pool size (8 with the pg store)
/// so publishes, touches and GC always find a connection.
const QUERY_CONCURRENCY: usize = 4;
/// Bundles loading their blobs at once (each holds the compressed blobs of
/// a whole manifest, ~8 MB for a full region, until it is sent).
pub const BUNDLE_CONCURRENCY: usize = 4;
/// Longest a bundle spends loading its blobs before it falls back to the
/// directory tar.
const BUNDLE_LOAD_TIMEOUT: Duration = Duration::from_secs(20);
/// Digests per bundle load query (bounds one result set to ~100 blobs).
const BUNDLE_BATCH: usize = 100;
/// Smallest `registry.blob_gc_grace_secs` honoured: a publish touches its
/// blobs before committing, and the grace keeps a concurrent GC pass off them.
pub const MIN_GC_GRACE: Duration = Duration::from_secs(300);

/// How [`BlobStore::store_manifest`] treats a file it cannot store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreMode {
    /// Publish: every file must be read from the master directory with the
    /// listed digest, else the publish fails.
    Strict,
    /// Import: files that are gone or changed on disk are looked up in the
    /// manifest's git commit, else skipped with a warning.
    Lenient,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StoreStats {
    /// Blobs written by this call.
    pub stored: usize,
    /// Blobs already present (their `last_seen_at` was refreshed).
    pub reused: usize,
    /// Lenient mode: blobs that could not be found anywhere.
    pub unavailable: usize,
    /// Uncompressed and stored bytes of the blobs written.
    pub bytes_in: u64,
    pub bytes_stored: u64,
}

impl std::ops::AddAssign for StoreStats {
    fn add_assign(&mut self, other: Self) {
        self.stored += other.stored;
        self.reused += other.reused;
        self.unavailable += other.unavailable;
        self.bytes_in += other.bytes_in;
        self.bytes_stored += other.bytes_stored;
    }
}

/// One master file ready to be served.
pub struct Blob {
    /// Uncompressed size (the response's Content-Length).
    pub size: u64,
    /// File mtime (fs) or when the blob was first stored (pg).
    pub modified: Option<SystemTime>,
    body: BlobBody,
}

enum BlobBody {
    File(std::fs::File),
    Zstd {
        data: Bytes,
        permit: Option<OwnedSemaphorePermit>,
    },
}

type ZstdReader = zstd::stream::read::Decoder<'static, std::io::Cursor<Bytes>>;

/// A reader that keeps the read permit of its blob until it is dropped.
struct PermitReader<R> {
    inner: R,
    _permit: Option<OwnedSemaphorePermit>,
}

impl<R: Read> Read for PermitReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Blob {
    /// The uncompressed bytes as a streaming response body.
    pub fn into_body(self) -> Body {
        match self.body {
            BlobBody::File(file) => Body::from_stream(tokio_util::io::ReaderStream::new(
                tokio::fs::File::from_std(file),
            )),
            BlobBody::Zstd { data, permit } => match zstd_reader(data) {
                Ok(decoder) => Body::from_stream(decode_stream(decoder, permit)),
                Err(e) => Body::from_stream(futures::stream::once(async move {
                    Err::<Bytes, std::io::Error>(e)
                })),
            },
        }
    }

    /// The uncompressed bytes as a blocking reader (bundle builder).
    pub fn into_reader(self) -> Result<Box<dyn Read + Send>, AppError> {
        Ok(match self.body {
            BlobBody::File(file) => Box::new(file),
            BlobBody::Zstd { data, permit } => Box::new(PermitReader {
                inner: zstd_reader(data)?,
                _permit: permit,
            }),
        })
    }
}

fn zstd_reader(data: Bytes) -> std::io::Result<ZstdReader> {
    let mut decoder = zstd::stream::read::Decoder::with_buffer(std::io::Cursor::new(data))?;
    decoder.window_log_max(ZSTD_WINDOW_LOG)?;
    Ok(decoder)
}

fn decode_stream(
    decoder: ZstdReader,
    permit: Option<OwnedSemaphorePermit>,
) -> impl futures::Stream<Item = std::io::Result<Bytes>> + Send {
    futures::stream::unfold(Some((decoder, permit)), |state| async move {
        let (mut decoder, permit) = state?;
        let mut buf = vec![0u8; CHUNK];
        match read_full(&mut decoder, &mut buf) {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok(Bytes::from(buf)), Some((decoder, permit))))
            }
            Err(e) => Some((Err(e), None)),
        }
    })
}

/// Read until `buf` is full or the reader is exhausted.
fn read_full(reader: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// The compressed blobs of one manifest, loaded before a bundle is answered
/// so its stream never waits on the database.
pub struct BlobSet {
    blobs: HashMap<String, (u64, Bytes)>,
    _permit: OwnedSemaphorePermit,
}

impl BlobSet {
    /// Uncompressed size and a reader of the blob with this digest.
    pub fn reader(&self, sha256: &str) -> Result<(u64, Box<dyn Read + Send>), AppError> {
        let (size, data) = self
            .blobs
            .get(sha256)
            .ok_or_else(|| AppError::NotFound(format!("no blob {sha256}")))?;
        Ok((*size, Box::new(zstd_reader(data.clone())?)))
    }
}

/// Where the registry reads master file content from.
pub trait BlobStore: Send + Sync {
    fn kind(&self) -> BlobStoreKind;

    /// Make every file `manifest` lists (read from `master_dir`) readable
    /// from the store. Called before the manifest is published.
    fn store_manifest<'a>(
        &'a self,
        master_dir: &'a Path,
        manifest: &'a MasterManifest,
        mode: StoreMode,
    ) -> BoxFuture<'a, Result<StoreStats, AppError>>;

    /// The blob with this digest, if the store can serve it. `local` is the
    /// master directory path of a current file with this digest: the fs
    /// store serves only such files, the pg store falls back to it.
    fn get<'a>(
        &'a self,
        sha256: &'a str,
        local: Option<&'a Path>,
    ) -> BoxFuture<'a, Result<Option<Blob>, AppError>>;

    /// The digests of `digests` the store does not hold (fs: none known).
    fn missing<'a>(
        &'a self,
        digests: &'a [String],
    ) -> BoxFuture<'a, Result<HashSet<String>, AppError>>;

    /// Every blob of `digests` held in memory (compressed), or `None` when
    /// any is missing or the store cannot load them (fs: always `None`).
    fn load_all<'a>(
        &'a self,
        digests: &'a [String],
    ) -> BoxFuture<'a, Result<Option<BlobSet>, AppError>>;

    /// Delete a bounded batch of unreferenced blobs past the grace period;
    /// returns how many were deleted.
    fn collect_garbage<'a>(
        &'a self,
        state: &'a RegistryState,
    ) -> BoxFuture<'a, Result<usize, AppError>>;
}

/// Open the store `registry.blob_store` selects. `pg` needs the database
/// state (`registry.state_dsn`): its table lives next to the state tables so
/// a publish can check the blobs inside its own transaction.
pub async fn open_blob_store(
    config: &RegistryConfig,
    state: &mut RegistryState,
) -> Result<Arc<dyn BlobStore>, AppError> {
    match config.blob_store {
        BlobStoreKind::Fs => Ok(Arc::new(FsBlobStore)),
        BlobStoreKind::Pg => {
            let Some(db) = state.database().cloned() else {
                return Err(AppError::Internal(
                    "registry.blob_store: pg requires registry.state_dsn".to_string(),
                ));
            };
            crate::db::init_registry_blob_table(&db).await?;
            state.require_blobs();
            let mut grace = Duration::from_secs(config.blob_gc_grace_secs);
            if grace < MIN_GC_GRACE {
                warn!(
                    "registry.blob_gc_grace_secs {} is below the minimum; using {}",
                    config.blob_gc_grace_secs,
                    MIN_GC_GRACE.as_secs()
                );
                grace = MIN_GC_GRACE;
            }
            Ok(Arc::new(DbBlobStore::new(db, grace)))
        }
    }
}

/// The master directory as the store (today's behaviour).
#[derive(Debug, Default, Clone, Copy)]
pub struct FsBlobStore;

impl FsBlobStore {
    async fn get_file(&self, sha256: &str, path: &Path) -> Result<Option<Blob>, AppError> {
        let meta = match tokio::fs::metadata(path).await {
            Ok(meta) => meta,
            Err(_) => return Ok(None),
        };
        // The file may have been rewritten since the manifest was published;
        // never serve different bytes under a digest URL.
        let actual = {
            let path = path.to_path_buf();
            let meta = meta.clone();
            tokio::task::spawn_blocking(move || file_sha256(&path, &meta))
                .await
                .map_err(|e| AppError::Internal(format!("digest task: {e}")))??
        };
        if actual != sha256 {
            return Ok(None);
        }
        match tokio::fs::File::open(path).await {
            Ok(file) => Ok(Some(Blob {
                size: meta.len(),
                modified: meta.modified().ok(),
                body: BlobBody::File(file.into_std().await),
            })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

impl BlobStore for FsBlobStore {
    fn kind(&self) -> BlobStoreKind {
        BlobStoreKind::Fs
    }

    fn store_manifest<'a>(
        &'a self,
        _master_dir: &'a Path,
        _manifest: &'a MasterManifest,
        _mode: StoreMode,
    ) -> BoxFuture<'a, Result<StoreStats, AppError>> {
        Box::pin(async { Ok(StoreStats::default()) })
    }

    fn get<'a>(
        &'a self,
        sha256: &'a str,
        local: Option<&'a Path>,
    ) -> BoxFuture<'a, Result<Option<Blob>, AppError>> {
        Box::pin(async move {
            match local {
                Some(path) => self.get_file(sha256, path).await,
                None => Ok(None),
            }
        })
    }

    fn missing<'a>(
        &'a self,
        _digests: &'a [String],
    ) -> BoxFuture<'a, Result<HashSet<String>, AppError>> {
        Box::pin(async { Ok(HashSet::new()) })
    }

    fn load_all<'a>(
        &'a self,
        _digests: &'a [String],
    ) -> BoxFuture<'a, Result<Option<BlobSet>, AppError>> {
        Box::pin(async { Ok(None) })
    }

    fn collect_garbage<'a>(
        &'a self,
        _state: &'a RegistryState,
    ) -> BoxFuture<'a, Result<usize, AppError>> {
        Box::pin(async { Ok(0) })
    }
}

/// `registry_blobs` in the registry state database.
pub struct DbBlobStore {
    db: DatabaseConnection,
    reads: Arc<Semaphore>,
    queries: Semaphore,
    bundles: Arc<Semaphore>,
    grace: Duration,
    gc_lock: tokio::sync::Mutex<()>,
    /// Database budget of one read ([`QUERY_TIMEOUT`]).
    timeout: Duration,
    /// How long the breaker stays open ([`BREAKER_OPEN`]).
    breaker_open: Duration,
    /// Reads skip the database until this instant (circuit breaker).
    breaker_until: Mutex<Option<Instant>>,
}

impl DbBlobStore {
    /// `grace` is used as given; [`open_blob_store`] enforces [`MIN_GC_GRACE`].
    pub fn new(db: DatabaseConnection, grace: Duration) -> Self {
        Self {
            db,
            reads: Arc::new(Semaphore::new(READ_CONCURRENCY)),
            queries: Semaphore::new(QUERY_CONCURRENCY),
            bundles: Arc::new(Semaphore::new(BUNDLE_CONCURRENCY)),
            grace,
            gc_lock: tokio::sync::Mutex::new(()),
            timeout: QUERY_TIMEOUT,
            breaker_open: BREAKER_OPEN,
            breaker_until: Mutex::new(None),
        }
    }

    /// Override the read budget and the breaker window (tests).
    pub fn with_timeouts(mut self, timeout: Duration, breaker_open: Duration) -> Self {
        self.timeout = timeout;
        self.breaker_open = breaker_open;
        self
    }

    /// Fail fast while the breaker is open.
    fn check_breaker(&self) -> Result<(), AppError> {
        let mut until = self.breaker_until.lock().unwrap_or_else(|e| e.into_inner());
        match *until {
            Some(t) if Instant::now() < t => Err(AppError::DatabaseError(
                "blob store unavailable (recent database error)".to_string(),
            )),
            Some(_) => {
                *until = None;
                Ok(())
            }
            None => Ok(()),
        }
    }

    fn trip(&self, error: &AppError) {
        let mut until = self.breaker_until.lock().unwrap_or_else(|e| e.into_inner());
        if until.is_none() {
            warn!(
                "Blob store read failed ({error}); skipping the database for {}s",
                self.breaker_open.as_secs_f64()
            );
        }
        *until = Some(Instant::now() + self.breaker_open);
    }

    /// Run a blob query under a query slot, bounded by `deadline`; a failure
    /// or timeout opens the breaker.
    async fn guarded<T, F>(&self, deadline: Instant, query: F) -> Result<T, AppError>
    where
        F: std::future::Future<Output = Result<T, sea_orm::DbErr>>,
    {
        let result = tokio::time::timeout_at(deadline, async {
            let _slot = self
                .queries
                .acquire()
                .await
                .map_err(|e| AppError::Internal(format!("blob query slot: {e}")))?;
            query.await.map_err(AppError::from)
        })
        .await
        .unwrap_or_else(|_| Err(AppError::DatabaseError("blob query: timed out".to_string())));
        if let Err(e) = &result {
            self.trip(e);
        }
        result
    }

    async fn fetch(&self, sha256: &str) -> Result<Option<Blob>, AppError> {
        self.check_breaker()?;
        // One budget for the read slot, the query slot and the query: slow
        // clients (holding read slots) or a hung database cannot stall reads.
        let deadline = Instant::now() + self.timeout;
        let permit = tokio::time::timeout_at(deadline, self.reads.clone().acquire_owned())
            .await
            .map_err(|_| AppError::Internal("blob read slot: timed out".to_string()))?
            .map_err(|e| AppError::Internal(format!("blob read slot: {e}")))?;
        let row = self
            .guarded(
                deadline,
                registry_blob::Entity::find_by_id(sha256.to_string()).one(&self.db),
            )
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        if row.encoding != ENCODING_ZSTD {
            return Err(AppError::Internal(format!(
                "blob {sha256}: unknown encoding {:?}",
                row.encoding
            )));
        }
        Ok(Some(Blob {
            size: row.size.max(0) as u64,
            modified: Some(row.created_at.into()),
            body: BlobBody::Zstd {
                data: Bytes::from(row.content),
                permit: Some(permit),
            },
        }))
    }

    async fn load(&self, digests: &[String]) -> Result<Option<BlobSet>, AppError> {
        self.check_breaker()?;
        let permit = tokio::time::timeout(self.timeout, self.bundles.clone().acquire_owned())
            .await
            .map_err(|_| AppError::Internal("bundle slot: timed out".to_string()))?
            .map_err(|e| AppError::Internal(format!("bundle slot: {e}")))?;
        let wanted: Vec<String> = digests
            .iter()
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let deadline = Instant::now() + BUNDLE_LOAD_TIMEOUT.max(self.timeout);
        let mut blobs = HashMap::with_capacity(wanted.len());
        for chunk in wanted.chunks(BUNDLE_BATCH) {
            let rows = self
                .guarded(
                    deadline,
                    registry_blob::Entity::find()
                        .filter(registry_blob::Column::Sha256.is_in(chunk.to_vec()))
                        .all(&self.db),
                )
                .await?;
            for row in rows {
                if row.encoding != ENCODING_ZSTD {
                    return Err(AppError::Internal(format!(
                        "blob {}: unknown encoding {:?}",
                        row.sha256, row.encoding
                    )));
                }
                blobs.insert(
                    row.sha256,
                    (row.size.max(0) as u64, Bytes::from(row.content)),
                );
            }
        }
        if blobs.len() < wanted.len() {
            return Ok(None);
        }
        Ok(Some(BlobSet {
            blobs,
            _permit: permit,
        }))
    }

    /// Refresh `last_seen_at` of blobs a publish references again.
    async fn touch(&self, digests: &[String]) -> Result<(), AppError> {
        let now = Utc::now();
        for chunk in digests.chunks(DIGEST_BATCH) {
            registry_blob::Entity::update_many()
                .col_expr(registry_blob::Column::LastSeenAt, Expr::value(now))
                .filter(registry_blob::Column::Sha256.is_in(chunk.to_vec()))
                .exec(&self.db)
                .await?;
        }
        Ok(())
    }

    /// Insert if absent; an existing row only has `last_seen_at` refreshed.
    async fn insert(&self, file: &MasterManifestFile, content: Vec<u8>) -> Result<(), AppError> {
        let now = Utc::now();
        registry_blob::Entity::insert(registry_blob::ActiveModel {
            sha256: Set(file.sha256.clone()),
            size: Set(file.size as i64),
            encoding: Set(ENCODING_ZSTD.to_string()),
            stored_size: Set(content.len() as i64),
            content: Set(content),
            created_at: Set(now),
            last_seen_at: Set(now),
        })
        .on_conflict(
            OnConflict::column(registry_blob::Column::Sha256)
                .update_column(registry_blob::Column::LastSeenAt)
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    async fn store_one(
        &self,
        master_dir: &Path,
        manifest: &MasterManifest,
        file: &MasterManifestFile,
        mode: StoreMode,
    ) -> Result<Option<u64>, AppError> {
        if !is_safe_path_component(&file.name) || !is_hex_digest(&file.sha256) {
            return Err(AppError::Internal(format!(
                "manifest lists an invalid file entry {:?}",
                file.name
            )));
        }
        let path = master_dir.join(&file.name);
        let expected = (file.sha256.clone(), file.size);
        let from_disk = tokio::task::spawn_blocking(move || -> Result<_, AppError> {
            match std::fs::File::open(&path) {
                Ok(f) => encode_verified(f, &expected.0, expected.1),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            }
        })
        .await
        .map_err(|e| AppError::Internal(format!("blob encode task: {e}")))??;
        let content = match (from_disk, mode) {
            (Some(content), _) => Some(content),
            (None, StoreMode::Strict) => {
                return Err(AppError::Internal(format!(
                    "{} changed or vanished since the manifest was built",
                    file.name
                )))
            }
            (None, StoreMode::Lenient) => match manifest.git_commit.clone() {
                Some(commit) => {
                    let dir = master_dir.to_path_buf();
                    let file = file.clone();
                    tokio::task::spawn_blocking(move || encode_from_git(&dir, &commit, &file))
                        .await
                        .map_err(|e| AppError::Internal(format!("blob git task: {e}")))?
                }
                None => None,
            },
        };
        let Some(content) = content else {
            return Ok(None);
        };
        let stored = content.len() as u64;
        self.insert(file, content).await?;
        Ok(Some(stored))
    }
}

impl BlobStore for DbBlobStore {
    fn kind(&self) -> BlobStoreKind {
        BlobStoreKind::Pg
    }

    fn store_manifest<'a>(
        &'a self,
        master_dir: &'a Path,
        manifest: &'a MasterManifest,
        mode: StoreMode,
    ) -> BoxFuture<'a, Result<StoreStats, AppError>> {
        Box::pin(async move {
            let mut seen = HashSet::new();
            let unique: Vec<&MasterManifestFile> = manifest
                .files
                .iter()
                .filter(|f| seen.insert(f.sha256.as_str()))
                .collect();
            let digests: Vec<String> = unique.iter().map(|f| f.sha256.clone()).collect();
            let missing = missing_digests(&self.db, &digests, false).await?;
            let present: Vec<String> = digests
                .iter()
                .filter(|d| !missing.contains(*d))
                .cloned()
                .collect();
            self.touch(&present).await?;
            let mut stats = StoreStats {
                reused: present.len(),
                ..Default::default()
            };
            for file in unique.into_iter().filter(|f| missing.contains(&f.sha256)) {
                match self.store_one(master_dir, manifest, file, mode).await? {
                    Some(stored) => {
                        stats.stored += 1;
                        stats.bytes_in += file.size;
                        stats.bytes_stored += stored;
                    }
                    None => {
                        stats.unavailable += 1;
                        warn!(
                            "{} Blob {} ({}) not found on disk or in git; not imported",
                            manifest.server.to_uppercase(),
                            file.sha256,
                            file.name
                        );
                    }
                }
            }
            Ok(stats)
        })
    }

    fn get<'a>(
        &'a self,
        sha256: &'a str,
        local: Option<&'a Path>,
    ) -> BoxFuture<'a, Result<Option<Blob>, AppError>> {
        Box::pin(async move {
            if !is_hex_digest(sha256) {
                return Ok(None);
            }
            match self.fetch(sha256).await {
                Ok(Some(blob)) => Ok(Some(blob)),
                Ok(None) => match local {
                    Some(path) => {
                        debug!("Blob {sha256} not stored yet, serving it from disk");
                        FsBlobStore.get_file(sha256, path).await
                    }
                    None => Ok(None),
                },
                Err(e) => match local {
                    Some(path) => {
                        debug!("Blob store read failed ({e}); serving {sha256} from disk");
                        FsBlobStore.get_file(sha256, path).await
                    }
                    None => Err(e),
                },
            }
        })
    }

    fn missing<'a>(
        &'a self,
        digests: &'a [String],
    ) -> BoxFuture<'a, Result<HashSet<String>, AppError>> {
        Box::pin(async move { missing_digests(&self.db, digests, false).await })
    }

    fn load_all<'a>(
        &'a self,
        digests: &'a [String],
    ) -> BoxFuture<'a, Result<Option<BlobSet>, AppError>> {
        Box::pin(async move { self.load(digests).await })
    }

    fn collect_garbage<'a>(
        &'a self,
        state: &'a RegistryState,
    ) -> BoxFuture<'a, Result<usize, AppError>> {
        Box::pin(async move {
            let _guard = self.gc_lock.lock().await;
            // Referenced set first: a publish racing this run touches its
            // blobs, which the cutoff below then excludes. Any unreadable
            // manifest or snapshot aborts the pass (nothing is deleted).
            let referenced = state
                .referenced_digests()
                .await
                .map_err(|e| AppError::Internal(format!("GC aborted, nothing deleted: {e}")))?;
            let grace = chrono::Duration::from_std(self.grace)
                .unwrap_or_else(|_| chrono::Duration::days(3650));
            let cutoff = Utc::now() - grace;
            let stale: Vec<String> = registry_blob::Entity::find()
                .select_only()
                .column(registry_blob::Column::Sha256)
                .filter(registry_blob::Column::LastSeenAt.lt(cutoff))
                .into_tuple::<String>()
                .all(&self.db)
                .await?;
            let doomed: Vec<String> = stale
                .into_iter()
                .filter(|d| !referenced.contains(d))
                .take(GC_BATCH)
                .collect();
            if doomed.is_empty() {
                return Ok(0);
            }
            let result = registry_blob::Entity::delete_many()
                .filter(registry_blob::Column::Sha256.is_in(doomed))
                .filter(registry_blob::Column::LastSeenAt.lt(cutoff))
                .exec(&self.db)
                .await?;
            let deleted = result.rows_affected as usize;
            if deleted > 0 {
                info!("Registry blob GC deleted {} unreferenced blob(s)", deleted);
            }
            Ok(deleted)
        })
    }
}

/// The digests of `digests` that `registry_blobs` does not hold. With
/// `lock` (inside the publish transaction) the found rows are share-locked
/// until commit, so a concurrent GC `DELETE` of one of them either finished
/// before (the row is missing here) or waits for the commit (and then sees
/// the refreshed `last_seen_at`). SQLite serializes writers and needs none.
pub(crate) async fn missing_digests<C: ConnectionTrait>(
    conn: &C,
    digests: &[String],
    lock: bool,
) -> Result<HashSet<String>, AppError> {
    let mut missing: HashSet<String> = digests.iter().cloned().collect();
    let wanted: Vec<String> = missing.iter().cloned().collect();
    let lock_type = match (lock, conn.get_database_backend()) {
        (false, _) | (true, DatabaseBackend::Sqlite) => None,
        (true, DatabaseBackend::Postgres) => Some(LockType::KeyShare),
        (true, _) => Some(LockType::Share),
    };
    for chunk in wanted.chunks(DIGEST_BATCH) {
        let mut query = registry_blob::Entity::find()
            .select_only()
            .column(registry_blob::Column::Sha256)
            .filter(registry_blob::Column::Sha256.is_in(chunk.to_vec()));
        if let Some(lock_type) = lock_type {
            query = query.lock(lock_type);
        }
        let found: Vec<String> = query.into_tuple::<String>().all(conn).await?;
        for digest in found {
            missing.remove(&digest);
        }
    }
    Ok(missing)
}

/// Stream `reader` through SHA-256 and a zstd encoder. `None` when the bytes
/// are not exactly `size` long with digest `sha256`.
pub fn encode_verified(
    mut reader: impl Read,
    sha256: &str,
    size: u64,
) -> Result<Option<Vec<u8>>, AppError> {
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), ZSTD_LEVEL)?;
    encoder.include_checksum(true)?;
    encoder.window_log(ZSTD_WINDOW_LOG)?;
    encoder.set_pledged_src_size(Some(size))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > size {
            return Ok(None);
        }
        hasher.update(&buf[..n]);
        encoder.write_all(&buf[..n])?;
    }
    if total != size || hex::encode(hasher.finalize()) != sha256 {
        return Ok(None);
    }
    Ok(Some(encoder.finish()?))
}

/// The file as committed at `commit` (`git cat-file blob <commit>:./<name>`
/// in the master directory), compressed, when its digest matches.
fn encode_from_git(master_dir: &Path, commit: &str, file: &MasterManifestFile) -> Option<Vec<u8>> {
    let valid_commit = commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit());
    if !valid_commit || !is_safe_path_component(&file.name) {
        return None;
    }
    let mut child = std::process::Command::new("git")
        .arg("-C")
        .arg(master_dir)
        .args(["cat-file", "blob"])
        .arg(format!("{commit}:./{}", file.name))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let encoded = encode_verified(stdout, &file.sha256, file.size);
    let _ = child.kill();
    let _ = child.wait();
    encoded.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(data: &[u8]) -> String {
        hex::encode(sha2::Sha256::digest(data))
    }

    #[test]
    fn encodes_only_matching_bytes_and_round_trips() {
        let data = br#"[{"id":1,"name":"a"},{"id":2,"name":"b"}]"#.repeat(1000);
        let digest = sha(&data);
        let encoded = encode_verified(data.as_slice(), &digest, data.len() as u64)
            .unwrap()
            .unwrap();
        assert!(encoded.len() < data.len() / 10);
        let mut decoded = Vec::new();
        zstd_reader(Bytes::from(encoded))
            .unwrap()
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, data);
        // Wrong digest, short and long input are all rejected.
        assert!(
            encode_verified(data.as_slice(), &sha(b"x"), data.len() as u64)
                .unwrap()
                .is_none()
        );
        assert!(encode_verified(&data[..10], &digest, data.len() as u64)
            .unwrap()
            .is_none());
        assert!(encode_verified(data.as_slice(), &digest, 10)
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn fs_store_serves_only_current_files_with_matching_digest() {
        let dir = std::env::temp_dir().join(format!("haruki_blobs_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cards.json");
        std::fs::write(&path, b"[1]").unwrap();
        let digest = sha(b"[1]");
        let store = FsBlobStore;
        assert!(store.get(&digest, None).await.unwrap().is_none());
        let blob = store.get(&digest, Some(&path)).await.unwrap().unwrap();
        assert_eq!(blob.size, 3);
        let mut body = Vec::new();
        blob.into_reader().unwrap().read_to_end(&mut body).unwrap();
        assert_eq!(body, b"[1]");
        assert!(store
            .get(&sha(b"[2]"), Some(&path))
            .await
            .unwrap()
            .is_none());
        assert!(store
            .get(&digest, Some(&dir.join("gone.json")))
            .await
            .unwrap()
            .is_none());
        assert!(store.missing(&[digest]).await.unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A hung database costs a read at most the query budget; then the
    /// breaker skips the database until it closes again.
    #[tokio::test]
    async fn slow_database_falls_back_fast_and_opens_the_breaker() {
        let dir = std::env::temp_dir().join(format!("haruki_blobs_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dsn = format!("sqlite://{}?mode=rwc", dir.join("blobs.db").display());
        let db = sea_orm::Database::connect(&dsn).await.unwrap();
        crate::db::init_registry_blob_table(&db).await.unwrap();
        let data = b"[{\"id\":1}]";
        let digest = sha(data);
        let path = dir.join("cards.json");
        std::fs::write(&path, data).unwrap();
        let budget = Duration::from_millis(200);
        let window = Duration::from_millis(600);
        let store = DbBlobStore::new(db, Duration::from_secs(86_400)).with_timeouts(budget, window);
        let file = MasterManifestFile {
            name: "cards.json".to_string(),
            sha256: digest.clone(),
            size: data.len() as u64,
        };
        let encoded = encode_verified(&data[..], &digest, file.size)
            .unwrap()
            .unwrap();
        store.insert(&file, encoded).await.unwrap();
        assert!(store.get(&digest, None).await.unwrap().is_some());

        // Every query slot taken, as if the database hung.
        let held = store
            .queries
            .acquire_many(QUERY_CONCURRENCY as u32)
            .await
            .unwrap();
        let started = std::time::Instant::now();
        let blob = store.get(&digest, Some(&path)).await.unwrap().unwrap();
        let elapsed = started.elapsed();
        assert!(elapsed >= budget && elapsed < budget * 5, "{elapsed:?}");
        assert!(matches!(blob.body, BlobBody::File(_)), "served from disk");
        drop(held);

        // Breaker open: no database wait even though it is healthy again.
        let started = std::time::Instant::now();
        assert!(store.get(&digest, None).await.is_err());
        let blob = store.get(&digest, Some(&path)).await.unwrap().unwrap();
        assert!(matches!(blob.body, BlobBody::File(_)));
        assert!(started.elapsed() < budget);
        assert!(store.load_all(std::slice::from_ref(&digest)).await.is_err());

        tokio::time::sleep(window).await;
        let blob = store.get(&digest, None).await.unwrap().unwrap();
        assert!(matches!(blob.body, BlobBody::Zstd { .. }));
        let set = store
            .load_all(std::slice::from_ref(&digest))
            .await
            .unwrap()
            .unwrap();
        let (size, mut reader) = set.reader(&digest).unwrap();
        let mut body = Vec::new();
        reader.read_to_end(&mut body).unwrap();
        assert_eq!((size, body.as_slice()), (data.len() as u64, &data[..]));
        assert!(store
            .load_all(&[digest.clone(), sha(b"other")])
            .await
            .unwrap()
            .is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
