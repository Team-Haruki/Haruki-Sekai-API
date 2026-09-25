//! Registry HTTP surface.
//!
//! Reads (open on the internal network):
//! - `GET /health`
//! - `GET /v1/master/{region}/current`            latest manifest (ETag = file set + version)
//! - `GET /v1/master/{region}/history?limit=N`    publish history, newest first
//! - `GET /v1/master/{region}/files/{name}`       one master file (ETag = its SHA-256,
//!   `If-None-Match` -> 304, `Last-Modified`, `Content-Length`)
//! - `GET /v1/master/{region}/bundle`             tar of the master directory
//! - `GET /v1/master/{region}/manifests/{hash}`  immutable manifest snapshot by contentHash
//! - `GET /v1/master/{region}/blob/{sha256}`      immutable master file by digest (fs
//!   store: current files only; pg store: any retained blob, see `registry::blobs`)
//! - `GET /v1/metas/{region}/current`             music_metas pointer (ETag = digest)
//! - `GET /v1/metas/{region}/music_metas.json`    current music_metas bytes (mutable)
//! - `GET /v1/metas/{region}/blob/{sha256}`       immutable music_metas bytes
//! - `GET /v1/app/{region}`                       `{appVersion, appHash}` (override,
//!   else the identity the owner's synced version file carries)
//!
//! CDN contract: pointers (`current`, `files/{name}`, `music_metas.json`,
//! `app`) answer `Cache-Control: no-cache` with a strong ETag and must be
//! revalidated (a CDN that ignores `no-cache` needs a bypass rule for those
//! paths, or clients add a unique query parameter); digest-addressed
//! resources (`manifests/{hash}`, `blob/{sha256}`) are immutable for a year.
//! Their 404s (a digest not stored yet, e.g. while the pg import runs) and
//! 503s (pg store unreachable) are `no-store`, so a CDN never pins a miss.
//!
//! Mutations (require `registry.token`; disabled when it is empty):
//! - `PUT /v1/app/{region}` / `DELETE /v1/app/{region}`  app-identity override; PUT
//!   also pushes it to every `registry.account_nodes` entry (`POST /internal/app-identity`)
//! - `POST /v1/master/{region}/refresh`            pull from owner now, then publish
//! - `POST /v1/master/{region}/publish`            re-scan the directory and publish
//! - `POST /v1/metas/{region}/refresh`             pull music_metas from upstream now
//! - `POST /internal/master-updated`               owner webhook (same shape a
//!   SekaiAPI peer accepts, so an owner's `master_sync.notify` can point here)

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use tracing::{error, info, warn};

use super::Registry;
use crate::api::internal::{build_master_tar, file_sha256, MasterUpdatedNotice};
use crate::client::helper::AppInfo;
use crate::config::{BlobStoreKind, ServerRegion};
use crate::error::AppError;
use crate::updater::master::is_safe_path_component;

type Shared = Arc<Registry>;

pub fn router(registry: Shared) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/master/{region}/current", get(current))
        .route("/v1/master/{region}/history", get(history))
        .route(
            "/v1/master/{region}/manifests/{hash}",
            get(manifest_by_hash),
        )
        .route("/v1/master/{region}/files/{name}", get(file))
        .route("/v1/master/{region}/blob/{sha256}", get(blob))
        .route("/v1/master/{region}/bundle", get(bundle))
        .route("/v1/metas/{region}/current", get(metas_current))
        .route("/v1/metas/{region}/music_metas.json", get(metas_file))
        .route("/v1/metas/{region}/blob/{sha256}", get(metas_blob))
        .route("/v1/metas/{region}/refresh", post(metas_refresh))
        .route("/v1/master/{region}/refresh", post(refresh))
        .route("/v1/master/{region}/publish", post(publish))
        .route(
            "/v1/app/{region}",
            get(app_identity)
                .put(set_app_identity)
                .delete(clear_app_identity),
        )
        .route("/internal/master-updated", post(master_updated))
        .with_state(registry)
}

fn parse_region(region: &str) -> Result<ServerRegion, AppError> {
    region
        .parse()
        .map_err(|_| AppError::InvalidServerRegion(region.to_string()))
}

/// Gate a mutating endpoint on `registry.token`: 404 when no token is
/// configured (the endpoint does not exist), 401 on a wrong one.
fn check_token(registry: &Registry, headers: &HeaderMap) -> Result<(), Box<Response>> {
    let expected = &registry.config.registry.token;
    if expected.is_empty() {
        return Err(Box::new(StatusCode::NOT_FOUND.into_response()));
    }
    let presented = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if presented != Some(expected.as_str()) {
        return Err(Box::new(StatusCode::UNAUTHORIZED.into_response()));
    }
    Ok(())
}

fn json<T: serde::Serialize>(value: &T) -> Response {
    match serde_json::to_string(value) {
        Ok(body) => (StatusCode::OK, [("content-type", "application/json")], body).into_response(),
        Err(e) => AppError::ParseError(e.to_string()).into_response(),
    }
}

/// Health, shaped for an external monitor: `status` is `degraded` (never a
/// non-200, so a monitor can still read the body) when a region's mirror push
/// failed. A failed push is otherwise silent — publishing continues and the
/// mirror simply stops moving.
async fn health(State(registry): State<Shared>) -> Response {
    let mut regions = serde_json::Map::new();
    let mut problems: Vec<String> = Vec::new();
    for region in registry.regions() {
        let current = registry.state.current(region).await.ok().flatten();
        let git = registry.syncers.get(&region).and_then(|s| s.git_state());
        if let Some(state) = git.as_ref() {
            if !state.ok {
                problems.push(format!(
                    "{} git push failed at {} ({})",
                    region.as_str(),
                    state.at,
                    state.reason.map(|r| r.as_str()).unwrap_or("unknown")
                ));
            }
        }
        regions.insert(
            region.as_str().to_string(),
            serde_json::json!({
                "dataVersion": current.as_ref().map(|m| m.data_version.clone()),
                "publishedAt": current.as_ref().map(|m| m.generated_at.clone()),
                "synced": registry.syncers.contains_key(&region),
                "gitPush": git,
            }),
        );
    }
    json(&serde_json::json!({
        "status": if problems.is_empty() { "ok" } else { "degraded" },
        "version": env!("CARGO_PKG_VERSION"),
        "blobStore": match registry.blobs.kind() {
            BlobStoreKind::Fs => "fs",
            BlobStoreKind::Pg => "pg",
        },
        "problems": problems,
        "regions": regions,
    }))
}

/// Strong ETag for a resource identified by a content digest.
fn etag(digest: &str) -> String {
    format!("\"{digest}\"")
}

/// Whether an `If-None-Match` header matches `etag` (exact or `*`).
fn if_none_match(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get("if-none-match")
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(',')
                .map(|t| t.trim().trim_start_matches("W/"))
                .any(|t| t == "*" || t == etag)
        })
        .unwrap_or(false)
}

fn http_date(time: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time)
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}

/// Cache policy for content that can never change under its URL (digest or
/// content-hash addressed): a CDN may hold it for a year.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// Mark a response as never cacheable (misses and failures of URLs that are
/// otherwise immutable).
fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        "cache-control",
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

/// 404 for a digest- or hash-addressed URL: the resource may appear later
/// (import, publish), so the miss must not be cached.
fn digest_not_found(what: String) -> Response {
    no_store(AppError::NotFound(what).into_response())
}

/// 503 for a read the blob store could not answer (database unreachable,
/// busy): transient, never cached.
fn store_unavailable(e: AppError) -> Response {
    warn!("Registry blob read failed: {e}");
    let mut response = e.into_response();
    *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
    let mut response = no_store(response);
    response
        .headers_mut()
        .insert("retry-after", axum::http::HeaderValue::from_static("5"));
    response
}

/// Stream a file as an immutable, digest-tagged response.
async fn immutable_file(path: &std::path::Path, digest: &str, content_type: &str) -> Response {
    let meta = match tokio::fs::metadata(path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return digest_not_found(format!("no blob {digest}"))
        }
        Err(e) => return AppError::IoError(e.to_string()).into_response(),
    };
    match tokio::fs::File::open(path).await {
        Ok(file) => {
            let mut response =
                axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file))
                    .into_response();
            let headers = response.headers_mut();
            let set = |headers: &mut HeaderMap, name: &'static str, value: String| {
                if let Ok(value) = axum::http::HeaderValue::from_str(&value) {
                    headers.insert(name, value);
                }
            };
            set(headers, "content-type", content_type.to_string());
            set(headers, "content-length", meta.len().to_string());
            set(headers, "etag", etag(digest));
            set(headers, "cache-control", IMMUTABLE.to_string());
            response
        }
        Err(e) => AppError::IoError(e.to_string()).into_response(),
    }
}

fn not_modified(etag: &str) -> Response {
    (
        StatusCode::NOT_MODIFIED,
        [
            ("etag", etag.to_string()),
            ("cache-control", "no-cache".to_string()),
        ],
    )
        .into_response()
}

async fn current(
    State(registry): State<Shared>,
    Path(region): Path<String>,
    headers: HeaderMap,
) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    match registry.state.current(region).await {
        Ok(Some(manifest)) => {
            // The manifest's identity is its file set plus version, not the
            // timestamp of the publish that last refreshed it.
            let tag = etag(&format!(
                "{}-{}",
                super::state::content_hash(&manifest),
                manifest.data_version
            ));
            if if_none_match(&headers, &tag) {
                return not_modified(&tag);
            }
            match serde_json::to_string(&manifest) {
                Ok(body) => (
                    StatusCode::OK,
                    [
                        ("content-type", "application/json".to_string()),
                        ("etag", tag),
                        ("cache-control", "no-cache".to_string()),
                        ("x-haruki-data-version", manifest.data_version.clone()),
                    ],
                    body,
                )
                    .into_response(),
                Err(e) => AppError::ParseError(e.to_string()).into_response(),
            }
        }
        Ok(None) => {
            AppError::NotFound(format!("region {} has not been published", region.as_str()))
                .into_response()
        }
        Err(e) => e.into_response(),
    }
}

/// Immutable manifest snapshot by content hash (the `contentHash` of a
/// `current` response).
async fn manifest_by_hash(
    State(registry): State<Shared>,
    Path((region, hash)): Path<(String, String)>,
) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    match registry.state.manifest_by_hash(region, &hash).await {
        Ok(Some(manifest)) => match serde_json::to_string(&manifest) {
            Ok(body) => (
                StatusCode::OK,
                [
                    ("content-type", "application/json".to_string()),
                    ("etag", etag(&hash)),
                    ("cache-control", IMMUTABLE.to_string()),
                ],
                body,
            )
                .into_response(),
            Err(e) => AppError::ParseError(e.to_string()).into_response(),
        },
        Ok(None) => digest_not_found(format!("no manifest {hash}")),
        Err(e) => e.into_response(),
    }
}

/// A master file by its SHA-256 (from the manifest): immutable, so a CDN
/// keeps serving it across versions for every table that did not change.
/// With the fs store only files of the current manifest are addressable (a
/// stale digest is a 404 and the consumer re-reads `current`); the pg store
/// also serves every blob a retained snapshot lists.
async fn blob(
    State(registry): State<Shared>,
    Path((region, sha256)): Path<(String, String)>,
) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    if !super::state::is_hex_digest(&sha256) {
        return digest_not_found(format!("no blob {sha256}"));
    }
    let manifest = match registry.state.current(region).await {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    let local = match manifest
        .as_ref()
        .and_then(|m| m.files.iter().find(|f| f.sha256 == sha256))
    {
        Some(entry) => match registry.region_paths(region) {
            Ok((master_dir, _)) => Some(std::path::Path::new(&master_dir).join(&entry.name)),
            Err(e) => return e.into_response(),
        },
        None => None,
    };
    match registry.blobs.get(&sha256, local.as_deref()).await {
        Ok(Some(blob)) => immutable_blob(blob, &sha256, "application/json"),
        Ok(None) => digest_not_found(format!("no blob {sha256}")),
        Err(e) => store_unavailable(e),
    }
}

/// An immutable, digest-tagged response streaming `blob`.
fn immutable_blob(blob: super::blobs::Blob, digest: &str, content_type: &str) -> Response {
    let size = blob.size;
    let mut response = blob.into_body().into_response();
    let headers = response.headers_mut();
    for (name, value) in [
        ("content-type", content_type.to_string()),
        ("content-length", size.to_string()),
        ("etag", etag(digest)),
        ("cache-control", IMMUTABLE.to_string()),
    ] {
        if let Ok(value) = axum::http::HeaderValue::from_str(&value) {
            headers.insert(name, value);
        }
    }
    response
}

fn metas_manager(registry: &Registry) -> Result<&super::metas::MusicMetasManager, AppError> {
    registry
        .metas
        .as_ref()
        .ok_or_else(|| AppError::NotFound("music_metas feed is disabled".to_string()))
}

/// The music_metas pointer: digest, size and upstream freshness.
async fn metas_current(
    State(registry): State<Shared>,
    Path(region): Path<String>,
    headers: HeaderMap,
) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    let manager = match metas_manager(&registry) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    match manager.current(region).await {
        Ok(Some(record)) => {
            let tag = etag(&record.sha256);
            if if_none_match(&headers, &tag) {
                return not_modified(&tag);
            }
            match serde_json::to_string(&record) {
                Ok(body) => (
                    StatusCode::OK,
                    [
                        ("content-type", "application/json".to_string()),
                        ("etag", tag),
                        ("cache-control", "no-cache".to_string()),
                    ],
                    body,
                )
                    .into_response(),
                Err(e) => AppError::ParseError(e.to_string()).into_response(),
            }
        }
        Ok(None) => AppError::NotFound(format!(
            "music_metas for {} not fetched yet",
            region.as_str()
        ))
        .into_response(),
        Err(e) => e.into_response(),
    }
}

/// The current music_metas bytes at a stable URL (mutable, ETag = digest).
async fn metas_file(
    State(registry): State<Shared>,
    Path(region): Path<String>,
    headers: HeaderMap,
) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    let manager = match metas_manager(&registry) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    let record = match manager.current(region).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return AppError::NotFound(format!(
                "music_metas for {} not fetched yet",
                region.as_str()
            ))
            .into_response()
        }
        Err(e) => return e.into_response(),
    };
    let tag = etag(&record.sha256);
    if if_none_match(&headers, &tag) {
        return not_modified(&tag);
    }
    let Some(path) = manager.blob_path(region, &record.sha256) else {
        return AppError::Internal("corrupt music_metas pointer".to_string()).into_response();
    };
    let mut response = immutable_file(&path, &record.sha256, "application/json").await;
    if let Ok(value) = axum::http::HeaderValue::from_str("no-cache") {
        response.headers_mut().insert("cache-control", value);
    }
    response
}

/// music_metas bytes by digest: immutable.
async fn metas_blob(
    State(registry): State<Shared>,
    Path((region, sha256)): Path<(String, String)>,
) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    let manager = match metas_manager(&registry) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    let Some(path) = manager.blob_path(region, &sha256) else {
        return digest_not_found(format!("no blob {sha256}"));
    };
    immutable_file(&path, &sha256, "application/json").await
}

/// Pull the region's music_metas from upstream now (synchronous; a few
/// seconds at most).
async fn metas_refresh(
    State(registry): State<Shared>,
    Path(region): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = check_token(&registry, &headers) {
        return *resp;
    }
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    let manager = match metas_manager(&registry) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    match manager.refresh(region).await {
        Ok(outcome) => json(&serde_json::json!({
            "changed": outcome.changed,
            "notModified": outcome.not_modified,
            "sha256": outcome.record.sha256,
            "size": outcome.record.size,
            "rows": outcome.record.rows,
        })),
        Err(e) => e.into_response(),
    }
}

#[derive(Deserialize)]
struct HistoryQuery {
    #[serde(default = "default_history_limit")]
    limit: usize,
}

fn default_history_limit() -> usize {
    20
}

async fn history(
    State(registry): State<Shared>,
    Path(region): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    match registry
        .state
        .history(region, query.limit.clamp(1, 1000))
        .await
    {
        Ok(records) => json(&records),
        Err(e) => e.into_response(),
    }
}

/// One master file with a strong ETag (its SHA-256, the same digest the
/// manifest lists) so consumers can revalidate with `If-None-Match` and
/// fetch only files whose digest changed.
async fn file(
    State(registry): State<Shared>,
    Path((region, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    if !is_safe_path_component(&name) || !name.ends_with(".json") || name.starts_with('.') {
        return AppError::NotFound(format!("no master file {:?}", name)).into_response();
    }
    if registry.blobs.kind() != BlobStoreKind::Fs {
        return file_from_store(&registry, region, &name, &headers).await;
    }
    let (master_dir, _) = match registry.region_paths(region) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let path = std::path::Path::new(&master_dir).join(&name);
    let meta = match tokio::fs::metadata(&path).await {
        Ok(m) if m.is_file() => m,
        Ok(_) => return AppError::NotFound(format!("no master file {:?}", name)).into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return AppError::NotFound(format!("no master file {:?}", name)).into_response()
        }
        Err(e) => return AppError::IoError(e.to_string()).into_response(),
    };
    let digest = {
        let path = path.clone();
        let meta = meta.clone();
        match tokio::task::spawn_blocking(move || file_sha256(&path, &meta)).await {
            Ok(Ok(d)) => d,
            Ok(Err(e)) => return e.into_response(),
            Err(e) => return AppError::Internal(format!("digest task: {e}")).into_response(),
        }
    };
    let tag = etag(&digest);
    if if_none_match(&headers, &tag) {
        return not_modified(&tag);
    }
    let mut response_headers = vec![
        ("content-type", "application/json".to_string()),
        ("content-length", meta.len().to_string()),
        ("etag", tag),
        ("cache-control", "no-cache".to_string()),
    ];
    if let Ok(modified) = meta.modified() {
        response_headers.push(("last-modified", http_date(modified)));
    }
    match tokio::fs::File::open(&path).await {
        Ok(file) => {
            let stream = tokio_util::io::ReaderStream::new(file);
            let mut response = axum::body::Body::from_stream(stream).into_response();
            for (name, value) in response_headers {
                if let Ok(value) = axum::http::HeaderValue::from_str(&value) {
                    response.headers_mut().insert(name, value);
                }
            }
            response
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            AppError::NotFound(format!("no master file {:?}", name)).into_response()
        }
        Err(e) => AppError::IoError(e.to_string()).into_response(),
    }
}

/// A master file of the current manifest from the blob store (pg): same
/// headers as the fs path, with `Last-Modified` = when the content was first
/// stored. Unlike fs (which serves whatever is on disk), the name resolves
/// through `current`: a file on disk but not in `current` is a 404, and so
/// is a file whose blob is not stored yet (import running) while its bytes
/// on disk no longer match the manifest digest.
async fn file_from_store(
    registry: &Registry,
    region: ServerRegion,
    name: &str,
    headers: &HeaderMap,
) -> Response {
    let not_found = || AppError::NotFound(format!("no master file {:?}", name)).into_response();
    let local = match registry.region_paths(region) {
        Ok((master_dir, _)) => std::path::Path::new(&master_dir).join(name),
        Err(e) => return e.into_response(),
    };
    let manifest = match registry.state.current(region).await {
        Ok(Some(m)) => m,
        Ok(None) => return not_found(),
        Err(e) => return e.into_response(),
    };
    let Some(entry) = manifest.files.iter().find(|f| f.name == name) else {
        return not_found();
    };
    let tag = etag(&entry.sha256);
    if if_none_match(headers, &tag) {
        return not_modified(&tag);
    }
    let blob = match registry.blobs.get(&entry.sha256, Some(&local)).await {
        Ok(Some(blob)) => blob,
        Ok(None) => return not_found(),
        Err(e) => return store_unavailable(e),
    };
    let mut response_headers = vec![
        ("content-type", "application/json".to_string()),
        ("content-length", blob.size.to_string()),
        ("etag", tag),
        ("cache-control", "no-cache".to_string()),
    ];
    if let Some(modified) = blob.modified {
        response_headers.push(("last-modified", http_date(modified)));
    }
    let mut response = blob.into_body().into_response();
    for (name, value) in response_headers {
        if let Ok(value) = axum::http::HeaderValue::from_str(&value) {
            response.headers_mut().insert(name, value);
        }
    }
    response
}

async fn bundle(State(registry): State<Shared>, Path(region): Path<String>) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    if registry.blobs.kind() != BlobStoreKind::Fs {
        match bundle_from_store(&registry, region).await {
            Ok(Some(response)) => return response,
            Ok(None) => {}
            // Store unreachable or busy: the directory tar still works.
            Err(e) => warn!(
                "{} Bundle from the master directory: blob store failed: {}",
                region.as_str().to_uppercase(),
                e
            ),
        }
    }
    let (master_dir, version_path) = match registry.region_paths(region) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let tmp_path = std::env::temp_dir().join(format!(
        "haruki-registry-bundle-{}-{}.tar",
        region.as_str(),
        uuid::Uuid::new_v4()
    ));
    let build_path = tmp_path.clone();
    let built = tokio::task::spawn_blocking(move || {
        build_master_tar(&master_dir, &version_path, &build_path)
    })
    .await;
    let built = match built {
        Ok(r) => r,
        Err(e) => Err(AppError::Internal(format!("bundle task: {e}"))),
    };
    if let Err(e) = built {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return e.into_response();
    }
    match tokio::fs::File::open(&tmp_path).await {
        Ok(file) => {
            // Unlink now; the open fd keeps the bytes until streaming ends.
            let _ = tokio::fs::remove_file(&tmp_path).await;
            (
                StatusCode::OK,
                [("content-type", "application/x-tar")],
                axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file)),
            )
                .into_response()
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            AppError::IoError(e.to_string()).into_response()
        }
    }
}

/// The current manifest's files as a tar streamed from the blob store (pg):
/// the same entries the fs bundle holds, in manifest order with fixed
/// metadata (mode 0644, mtime = publish time), plus a version entry
/// rebuilt from the manifest. Consistent with `current` by construction.
/// Every compressed blob is loaded before the 200 is sent, so the stream
/// itself never touches the database and cannot break off halfway.
/// `None` (fall back to the directory bundle) while a blob is missing; an
/// empty manifest is a 404, like an empty directory.
async fn bundle_from_store(
    registry: &Registry,
    region: ServerRegion,
) -> Result<Option<Response>, AppError> {
    let Some(manifest) = registry.state.current(region).await? else {
        return Ok(None);
    };
    if manifest.files.is_empty() {
        return Ok(Some(
            AppError::NotFound("master directory is empty".to_string()).into_response(),
        ));
    }
    let digests: Vec<String> = manifest.files.iter().map(|f| f.sha256.clone()).collect();
    let Some(blobs) = registry.blobs.load_all(&digests).await? else {
        info!(
            "{} Bundle from the master directory: blob(s) not stored yet",
            region.as_str().to_uppercase()
        );
        return Ok(None);
    };
    let (tx, rx) = tokio::sync::mpsc::channel::<std::io::Result<axum::body::Bytes>>(8);
    tokio::task::spawn_blocking(move || {
        let writer = ChannelWriter {
            tx: tx.clone(),
            buf: Vec::with_capacity(BUNDLE_CHUNK),
        };
        if let Err(e) = write_bundle(&blobs, &manifest, writer) {
            let _ = tx.blocking_send(Err(std::io::Error::other(e.to_string())));
        }
    });
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    });
    Ok(Some(
        (
            StatusCode::OK,
            [("content-type", "application/x-tar")],
            axum::body::Body::from_stream(stream),
        )
            .into_response(),
    ))
}

const BUNDLE_CHUNK: usize = 64 * 1024;

/// Blocking `Write` into the response body channel, in `BUNDLE_CHUNK` pieces.
struct ChannelWriter {
    tx: tokio::sync::mpsc::Sender<std::io::Result<axum::body::Bytes>>,
    buf: Vec<u8>,
}

impl ChannelWriter {
    fn send(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = std::mem::replace(&mut self.buf, Vec::with_capacity(BUNDLE_CHUNK));
        self.tx
            .blocking_send(Ok(axum::body::Bytes::from(chunk)))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "client went away"))
    }
}

impl std::io::Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= BUNDLE_CHUNK {
            self.send()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.send()
    }
}

fn write_bundle(
    blobs: &super::blobs::BlobSet,
    manifest: &crate::api::internal::MasterManifest,
    writer: ChannelWriter,
) -> Result<(), AppError> {
    use std::io::Write as _;
    let mtime = chrono::DateTime::parse_from_rfc3339(&manifest.generated_at)
        .map(|t| t.timestamp().max(0) as u64)
        .unwrap_or(0);
    let header = |size: u64| {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(size);
        // Master data is public; 0644 is what the synced worktree files carry.
        header.set_mode(0o644); // NOSONAR
        header.set_mtime(mtime);
        header
    };
    let mut builder = tar::Builder::new(writer);
    for file in &manifest.files {
        let (size, reader) = blobs.reader(&file.sha256)?;
        let mut entry = header(size);
        builder.append_data(&mut entry, &file.name, reader)?;
    }
    if !manifest.data_version.is_empty() {
        let version = crate::client::helper::VersionInfo {
            app_version: manifest.app_version.clone(),
            app_hash: manifest.app_hash.clone(),
            data_version: manifest.data_version.clone(),
            asset_version: manifest.asset_version.clone(),
            asset_hash: manifest.asset_hash.clone(),
            cdn_version: manifest.cdn_version,
        };
        let data = serde_json::to_vec(&version)
            .map_err(|e| AppError::ParseError(format!("version entry: {e}")))?;
        let mut entry = header(data.len() as u64);
        builder.append_data(
            &mut entry,
            crate::updater::sync::BUNDLE_VERSION_ENTRY,
            data.as_slice(),
        )?;
    }
    builder.into_inner()?.flush()?;
    Ok(())
}

async fn app_identity(State(registry): State<Shared>, Path(region): Path<String>) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    match registry.app_identity(region).await {
        Ok(info) => {
            let mut response = json(&info);
            if let Ok(value) = axum::http::HeaderValue::from_str("no-cache") {
                response.headers_mut().insert("cache-control", value);
            }
            response
        }
        Err(e) => e.into_response(),
    }
}

/// Parse a JSON body after the token check has passed, so an unauthorized
/// or disabled endpoint never reports body errors.
fn parse_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, AppError> {
    serde_json::from_slice(body).map_err(|e| AppError::ParseError(format!("request body: {e}")))
}

async fn set_app_identity(
    State(registry): State<Shared>,
    Path(region): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Err(resp) = check_token(&registry, &headers) {
        return *resp;
    }
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    let info: AppInfo = match parse_body(&body) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    if info.app_version.trim().is_empty() && info.app_hash.trim().is_empty() {
        return AppError::ParseError("appVersion or appHash is required".to_string())
            .into_response();
    }
    match registry.set_app_identity(region, &info).await {
        Ok((effective, pushed)) => json(&serde_json::json!({
            "appVersion": effective.app_version,
            "appHash": effective.app_hash,
            "pushed": pushed,
        })),
        Err(e) => e.into_response(),
    }
}

async fn clear_app_identity(
    State(registry): State<Shared>,
    Path(region): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = check_token(&registry, &headers) {
        return *resp;
    }
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    match registry.state.clear_app_identity(region).await {
        Ok(removed) => json(&serde_json::json!({ "removed": removed })),
        Err(e) => e.into_response(),
    }
}

async fn refresh(
    State(registry): State<Shared>,
    Path(region): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = check_token(&registry, &headers) {
        return *resp;
    }
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    if !registry.syncers.contains_key(&region) {
        return AppError::NotFound(format!(
            "no master sync configured for region {}",
            region.as_str()
        ))
        .into_response();
    }
    spawn_refresh(registry, region, "manual refresh");
    json(&serde_json::json!({ "triggered": true }))
}

async fn publish(
    State(registry): State<Shared>,
    Path(region): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = check_token(&registry, &headers) {
        return *resp;
    }
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    match registry.publish(region).await {
        Ok((manifest, changed)) => json(&serde_json::json!({
            "changed": changed,
            "dataVersion": manifest.data_version,
            "files": manifest.files.len(),
        })),
        Err(e) => e.into_response(),
    }
}

async fn master_updated(
    State(registry): State<Shared>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Err(resp) = check_token(&registry, &headers) {
        return *resp;
    }
    let notice: MasterUpdatedNotice = match parse_body(&body) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let region = match parse_region(&notice.server) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    if !registry.syncers.contains_key(&region) {
        return AppError::NotFound(format!(
            "no master sync configured for region {}",
            region.as_str()
        ))
        .into_response();
    }
    info!(
        "{} Owner webhook received (dataVersion {:?}), starting sync",
        region.as_str().to_uppercase(),
        notice.data_version
    );
    spawn_refresh(registry, region, "webhook");
    json(&serde_json::json!({ "triggered": true }))
}

fn spawn_refresh(registry: Shared, region: ServerRegion, trigger: &'static str) {
    tokio::spawn(async move {
        match registry.refresh(region).await {
            Ok(true) => {}
            Ok(false) => info!(
                "{} {}: already up to date",
                region.as_str().to_uppercase(),
                trigger
            ),
            Err(e) => error!(
                "{} {} sync failed (poll will retry): {}",
                region.as_str().to_uppercase(),
                trigger,
                e
            ),
        }
    });
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use axum::body::Body;
    use axum::http::Response as HttpResponse;
    use axum::routing::any;

    use super::*;
    use crate::config::{Config, MasterSyncPeer, ServerConfig};
    use crate::updater::sync::{build_syncers, BUNDLE_VERSION_ENTRY};

    fn temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!("haruki_reg_http_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (format!("http://{address}"), task)
    }

    fn base_config(root: &std::path::Path, token: &str) -> Config {
        let mut config: Config = serde_yaml::from_str("backend: {}").unwrap();
        config.registry.token = token.to_string();
        config.registry.state_dir = root.join("state").to_string_lossy().into_owned();
        let mut server: ServerConfig = serde_yaml::from_str("{}").unwrap();
        server.master_dir = root.join("master").to_string_lossy().into_owned();
        server.version_path = root.join("version.json").to_string_lossy().into_owned();
        std::fs::create_dir_all(&server.master_dir).unwrap();
        config.servers.insert(ServerRegion::Jp, server);
        config
    }

    fn write_version(root: &std::path::Path, data_version: &str) {
        std::fs::write(
            root.join("version.json"),
            format!(
                r#"{{"appVersion":"5.6.0","appHash":"owner-hash","dataVersion":"{data_version}","assetVersion":"5.6.1.10","cdnVersion":0}}"#
            ),
        )
        .unwrap();
    }

    async fn get_json(client: &reqwest::Client, url: &str) -> (u16, serde_json::Value) {
        let resp = client.get(url).send().await.unwrap();
        let status = resp.status().as_u16();
        let body = resp.bytes().await.unwrap();
        (
            status,
            serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn database_state_serves_the_same_responses_as_files() {
        let root = temp_dir();
        let mut config = base_config(&root, "secret");
        config.registry.music_metas.enabled = false;
        let config = Arc::new(config);
        std::fs::write(root.join("master/cards.json"), "[{\"id\":1}]").unwrap();
        write_version(&root, "5.6.1.11");
        let files = Arc::new(Registry::new(config.clone(), HashMap::new()));
        files.publish_missing().await;
        files
            .state
            .set_app_identity(
                ServerRegion::Jp,
                &AppInfo {
                    app_version: "5.7.0".to_string(),
                    app_hash: "override".to_string(),
                },
            )
            .await
            .unwrap();
        let dsn = format!("sqlite://{}?mode=rwc", root.join("state.db").display());
        let state = super::super::state::RegistryState::connect(&config.registry.state_dir, &dsn)
            .await
            .unwrap();
        let database = Arc::new(Registry::with_state(config, HashMap::new(), state));
        // Publishing again at startup is a no-op: the imported current exists.
        database.publish_missing().await;
        let (file_base, file_server) = serve(router(files)).await;
        let (db_base, db_server) = serve(router(database)).await;
        let client = reqwest::Client::new();

        let (_, current) = get_json(&client, &format!("{file_base}/v1/master/jp/current")).await;
        let hash = current["contentHash"].as_str().unwrap().to_string();
        for path in [
            "/v1/master/jp/current".to_string(),
            format!("/v1/master/jp/manifests/{hash}"),
            "/v1/master/jp/history".to_string(),
            "/v1/master/kr/current".to_string(),
            "/v1/app/jp".to_string(),
        ] {
            let a = client
                .get(format!("{file_base}{path}"))
                .send()
                .await
                .unwrap();
            let b = client.get(format!("{db_base}{path}")).send().await.unwrap();
            assert_eq!(a.status(), b.status(), "{path}");
            for header in ["etag", "cache-control", "content-type"] {
                assert_eq!(
                    a.headers().get(header),
                    b.headers().get(header),
                    "{path} {header}"
                );
            }
            assert_eq!(a.bytes().await.unwrap(), b.bytes().await.unwrap(), "{path}");
        }
        let blob = current["files"][0]["sha256"].as_str().unwrap();
        let resp = client
            .get(format!("{db_base}/v1/master/jp/blob/{blob}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let resp = client
            .get(format!("{db_base}/v1/master/jp/current"))
            .header("if-none-match", current_etag(&client, &file_base).await)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 304);
        file_server.abort();
        db_server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A registry on database state at `dsn` with `blob_store: pg`.
    async fn blob_registry(config: &Config, dsn: &str) -> Arc<Registry> {
        let mut config = config.clone();
        config.registry.state_dsn = dsn.to_string();
        config.registry.blob_store = BlobStoreKind::Pg;
        let mut state = super::super::state::RegistryState::connect_with_pool(
            &config.registry.state_dir,
            dsn,
            crate::db::registry_state_pool_size(BlobStoreKind::Pg),
        )
        .await
        .unwrap();
        let blobs = super::super::blobs::open_blob_store(&config.registry, &mut state)
            .await
            .unwrap();
        Arc::new(
            Registry::with_state(Arc::new(config), HashMap::new(), state).with_blob_store(blobs),
        )
    }

    fn untar(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        use std::io::Read;
        let mut archive = tar::Archive::new(bytes);
        let mut entries: Vec<(String, Vec<u8>)> = archive
            .entries()
            .unwrap()
            .map(|entry| {
                let mut entry = entry.unwrap();
                let name = entry.path().unwrap().to_string_lossy().into_owned();
                let mut data = Vec::new();
                entry.read_to_end(&mut data).unwrap();
                if name == BUNDLE_VERSION_ENTRY {
                    let version: crate::client::helper::VersionInfo =
                        serde_json::from_slice(&data).unwrap();
                    data = serde_json::to_vec(&version).unwrap();
                }
                (name, data)
            })
            .collect();
        entries.sort();
        entries
    }

    async fn blob_rows(registry: &Registry) -> u64 {
        use sea_orm::{EntityTrait, PaginatorTrait};
        crate::db::entity::RegistryBlob::find()
            .count(registry.state.database().unwrap())
            .await
            .unwrap()
    }

    /// fs and pg blob stores against one master directory: identical bodies
    /// and headers for every read, import of existing manifests, dedupe,
    /// historical blobs, the publish guard and garbage collection.
    async fn exercise_blob_store(dsn: &str) {
        let root = temp_dir();
        let mut config = base_config(&root, "secret");
        config.registry.music_metas.enabled = false;
        let master = root.join("master");
        std::fs::write(master.join("cards.json"), "[{\"id\":1}]").unwrap();
        // Same bytes under two names: one blob.
        std::fs::write(master.join("dup.json"), "[{\"id\":1}]").unwrap();
        let big: String = (0..20_000)
            .map(|i| format!("{{\"id\":{i},\"title\":\"music {i}\"}}"))
            .collect::<Vec<_>>()
            .join(",");
        let big = format!("[{big}]");
        std::fs::write(master.join("musics.json"), &big).unwrap();
        write_version(&root, "5.6.1.11");
        let files = Arc::new(Registry::new(Arc::new(config.clone()), HashMap::new()));
        files.publish_missing().await;
        // Switching an existing deployment: the file state is imported, the
        // blobs are not there yet and reads fall back to the directory.
        let pg = blob_registry(&config, dsn).await;
        pg.publish_missing().await;
        assert_eq!(blob_rows(&pg).await, 0);
        let (file_base, file_server) = serve(router(files.clone())).await;
        let (pg_base, pg_server) = serve(router(pg.clone())).await;
        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{pg_base}/v1/master/jp/files/musics.json"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.text().await.unwrap(), big);
        let resp = client
            .get(format!("{pg_base}/v1/master/jp/bundle"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        let (_, health) = get_json(&client, &format!("{pg_base}/health")).await;
        assert_eq!(health["blobStore"], "pg");
        let (_, health) = get_json(&client, &format!("{file_base}/health")).await;
        assert_eq!(health["blobStore"], "fs");

        let stats = pg.import_blobs().await;
        assert_eq!((stats.stored, stats.unavailable), (2, 0));
        assert_eq!(blob_rows(&pg).await, 2);
        // Idempotent.
        assert_eq!(pg.import_blobs().await.stored, 0);

        let (_, current) = get_json(&client, &format!("{file_base}/v1/master/jp/current")).await;
        let hash = current["contentHash"].as_str().unwrap().to_string();
        let mut paths = vec![
            "/v1/master/jp/current".to_string(),
            format!("/v1/master/jp/manifests/{hash}"),
            "/v1/master/jp/history".to_string(),
            "/v1/master/jp/files/nope.json".to_string(),
            "/v1/master/jp/files/..json".to_string(),
            format!("/v1/master/jp/blob/{}", "0".repeat(64)),
            "/v1/master/jp/blob/xyz".to_string(),
            "/v1/master/kr/files/cards.json".to_string(),
        ];
        for file in current["files"].as_array().unwrap() {
            paths.push(format!(
                "/v1/master/jp/files/{}",
                file["name"].as_str().unwrap()
            ));
            paths.push(format!(
                "/v1/master/jp/blob/{}",
                file["sha256"].as_str().unwrap()
            ));
        }
        for path in &paths {
            let a = client
                .get(format!("{file_base}{path}"))
                .send()
                .await
                .unwrap();
            let b = client.get(format!("{pg_base}{path}")).send().await.unwrap();
            assert_eq!(a.status(), b.status(), "{path}");
            for header in ["etag", "cache-control", "content-type", "content-length"] {
                assert_eq!(
                    a.headers().get(header),
                    b.headers().get(header),
                    "{path} {header}"
                );
            }
            assert_eq!(
                a.headers().contains_key("last-modified"),
                b.headers().contains_key("last-modified"),
                "{path}"
            );
            assert_eq!(a.bytes().await.unwrap(), b.bytes().await.unwrap(), "{path}");
        }
        // Misses on digest URLs must not be cached by a CDN.
        for path in [
            format!("/v1/master/jp/blob/{}", "0".repeat(64)),
            format!("/v1/master/jp/manifests/{}", "0".repeat(64)),
        ] {
            let resp = client.get(format!("{pg_base}{path}")).send().await.unwrap();
            assert_eq!(resp.status(), 404, "{path}");
            assert_eq!(resp.headers()["cache-control"], "no-store", "{path}");
        }
        let file_etag = format!("\"{}\"", current["files"][0]["sha256"].as_str().unwrap());
        let resp = client
            .get(format!("{pg_base}/v1/master/jp/files/cards.json"))
            .header("if-none-match", &file_etag)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 304);
        // Bundle: the same entries (the version entry compared parsed).
        let a = client
            .get(format!("{file_base}/v1/master/jp/bundle"))
            .send()
            .await
            .unwrap();
        let b = client
            .get(format!("{pg_base}/v1/master/jp/bundle"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            a.headers().get("content-type"),
            b.headers().get("content-type")
        );
        let (a, b) = (a.bytes().await.unwrap(), b.bytes().await.unwrap());
        let entries = untar(&b);
        assert_eq!(untar(&a), entries);
        assert_eq!(entries.len(), 4);

        // The store now serves without the directory.
        let old_sha = current["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == "musics.json")
            .unwrap()["sha256"]
            .as_str()
            .unwrap()
            .to_string();
        std::fs::write(master.join("musics.json"), "[{\"id\":2}]").unwrap();
        let resp = client
            .get(format!("{pg_base}/v1/master/jp/blob/{old_sha}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.text().await.unwrap(), big);
        // A new publish: the replaced file stays addressable by digest (its
        // snapshot is retained), unchanged files are not stored again.
        write_version(&root, "5.6.1.12");
        let (manifest, changed) = pg.publish(ServerRegion::Jp).await.unwrap();
        assert!(changed);
        assert_eq!(blob_rows(&pg).await, 3);
        let resp = client
            .get(format!("{pg_base}/v1/master/jp/blob/{old_sha}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()["cache-control"], IMMUTABLE);
        assert_eq!(resp.text().await.unwrap(), big);
        let resp = client
            .get(format!("{file_base}/v1/master/jp/blob/{old_sha}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404, "fs serves current files only");
        let resp = client
            .get(format!("{pg_base}/v1/master/jp/files/musics.json"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.text().await.unwrap(), "[{\"id\":2}]");

        // Database unreachable: current files come from disk, the bundle
        // from the directory, historical blobs answer an uncached 503.
        let closed = sea_orm::Database::connect(dsn).await.unwrap();
        closed.close_by_ref().await.unwrap();
        let down = Arc::new(
            Registry::with_state(Arc::new(config.clone()), HashMap::new(), pg.state.clone())
                .with_blob_store(Arc::new(super::super::blobs::DbBlobStore::new(
                    closed,
                    std::time::Duration::from_secs(86_400),
                ))),
        );
        let (down_base, down_server) = serve(router(down)).await;
        let started = std::time::Instant::now();
        let resp = client
            .get(format!("{down_base}/v1/master/jp/blob/{old_sha}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 503);
        assert_eq!(resp.headers()["cache-control"], "no-store");
        let resp = client
            .get(format!("{down_base}/v1/master/jp/files/cards.json"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.text().await.unwrap(), "[{\"id\":1}]");
        let cards_sha = manifest
            .files
            .iter()
            .find(|f| f.name == "cards.json")
            .unwrap()
            .sha256
            .clone();
        let resp = client
            .get(format!("{down_base}/v1/master/jp/blob/{cards_sha}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()["cache-control"], IMMUTABLE);
        let resp = client
            .get(format!("{down_base}/v1/master/jp/bundle"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let names: Vec<String> = untar(&resp.bytes().await.unwrap())
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert!(names.contains(&"musics.json".to_string()), "{names:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        down_server.abort();

        // A manifest listing an unstored digest never becomes current.
        let mut bogus = manifest.clone();
        bogus.files[0].sha256 = "1".repeat(64);
        bogus.content_hash = String::new();
        assert!(pg.state.publish(ServerRegion::Jp, &bogus).await.is_err());
        assert_eq!(
            pg.state
                .current(ServerRegion::Jp)
                .await
                .unwrap()
                .unwrap()
                .content_hash,
            manifest.content_hash
        );

        // GC: nothing while every blob is referenced or within the grace.
        assert_eq!(pg.blobs.collect_garbage(&pg.state).await.unwrap(), 0);
        let db = pg.state.database().unwrap().clone();
        // PostgreSQL: the publish check's row lock holds off GC's DELETE
        // until the publish commits.
        if db.get_database_backend() == sea_orm::DatabaseBackend::Postgres {
            use sea_orm::{ConnectionTrait, TransactionTrait};
            let publish = db.begin().await.unwrap();
            let checked = super::super::blobs::missing_digests(
                &publish,
                std::slice::from_ref(&old_sha),
                true,
            )
            .await
            .unwrap();
            assert!(checked.is_empty());
            let gc = db.begin().await.unwrap();
            gc.execute_unprepared("SET LOCAL lock_timeout = '200ms'")
                .await
                .unwrap();
            let blocked = gc
                .execute_unprepared(&format!(
                    "DELETE FROM registry_blobs WHERE sha256 = '{old_sha}'"
                ))
                .await;
            assert!(blocked.is_err(), "DELETE must wait for the publish");
            gc.rollback().await.unwrap();
            publish.rollback().await.unwrap();
        }
        let eager = super::super::blobs::DbBlobStore::new(db.clone(), std::time::Duration::ZERO);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        use super::super::blobs::BlobStore as _;
        assert_eq!(eager.collect_garbage(&pg.state).await.unwrap(), 0);
        // Drop the old snapshot: its only exclusive blob becomes garbage.
        {
            use crate::db::entity::registry_state;
            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
            registry_state::Entity::delete_many()
                .filter(registry_state::Column::Kind.eq("manifest_snapshot"))
                .filter(registry_state::Column::Name.eq(hash.clone()))
                .exec(&db)
                .await
                .unwrap();
        }
        assert_eq!(pg.blobs.collect_garbage(&pg.state).await.unwrap(), 0);
        // An unreadable snapshot aborts GC: nothing is deleted.
        {
            use crate::db::entity::registry_state;
            use sea_orm::{ActiveValue::Set, EntityTrait};
            registry_state::Entity::insert(registry_state::ActiveModel {
                region: Set("jp".to_string()),
                kind: Set("manifest_snapshot".to_string()),
                name: Set("f".repeat(64)),
                value: Set(serde_json::json!({"broken": true})),
                updated_at: Set(chrono::Utc::now()),
            })
            .exec(&db)
            .await
            .unwrap();
        }
        assert!(eager.collect_garbage(&pg.state).await.is_err());
        assert_eq!(blob_rows(&pg).await, 3);
        {
            use crate::db::entity::registry_state;
            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
            registry_state::Entity::delete_many()
                .filter(registry_state::Column::Name.eq("f".repeat(64)))
                .exec(&db)
                .await
                .unwrap();
        }
        assert_eq!(eager.collect_garbage(&pg.state).await.unwrap(), 1);
        assert_eq!(blob_rows(&pg).await, 2);
        let resp = client
            .get(format!("{pg_base}/v1/master/jp/blob/{old_sha}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
        assert_eq!(resp.headers()["cache-control"], "no-store");
        let resp = client
            .get(format!("{pg_base}/v1/master/jp/files/cards.json"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.text().await.unwrap(), "[{\"id\":1}]");

        // An empty manifest's bundle is a 404, like an empty directory.
        let mut empty = manifest.clone();
        empty.files.clear();
        empty.content_hash = String::new();
        pg.state.publish(ServerRegion::Jp, &empty).await.unwrap();
        let resp = client
            .get(format!("{pg_base}/v1/master/jp/bundle"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);

        file_server.abort();
        pg_server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn blob_store_on_sqlite_matches_the_directory() {
        let db = std::env::temp_dir().join(format!("haruki_blobs_{}.db", uuid::Uuid::new_v4()));
        exercise_blob_store(&format!("sqlite://{}?mode=rwc", db.display())).await;
        let _ = std::fs::remove_file(db);
    }

    /// Set `HARUKI_TEST_REGISTRY_DSN` to a scratch PostgreSQL database (its
    /// registry tables are dropped first), then `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore] // Requires a running PostgreSQL
    async fn blob_store_on_postgres_matches_the_directory() {
        use sea_orm::ConnectionTrait;
        let dsn = std::env::var("HARUKI_TEST_REGISTRY_DSN")
            .unwrap_or_else(|_| "postgres://haruki:sekai@localhost:5432/registry_test".to_string());
        let db = sea_orm::Database::connect(&dsn).await.unwrap();
        db.execute_unprepared(
            "DROP TABLE IF EXISTS registry_state; DROP TABLE IF EXISTS registry_publish_history; \
             DROP TABLE IF EXISTS registry_blobs;",
        )
        .await
        .unwrap();
        exercise_blob_store(&dsn).await;
    }

    #[test]
    fn pg_blob_store_requires_database_state() {
        let root = temp_dir();
        let mut config = base_config(&root, "");
        config.registry.blob_store = BlobStoreKind::Pg;
        let mut state = super::super::state::RegistryState::new(&config.registry.state_dir);
        let opened =
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(super::super::blobs::open_blob_store(
                    &config.registry,
                    &mut state,
                ));
        assert!(opened.is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Import recovers files of retained snapshots that are gone from the
    /// directory from the manifest's git commit.
    #[tokio::test]
    async fn blob_import_reads_replaced_files_from_git() {
        let root = temp_dir();
        let master = root.join("master");
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(status.status.success(), "git {args:?}: {status:?}");
        };
        git(&["init", "-q"]);
        let mut config = base_config(&root, "");
        config.registry.music_metas.enabled = false;
        std::fs::write(master.join("cards.json"), "[{\"id\":1}]").unwrap();
        write_version(&root, "1");
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "v1"]);
        let files = Registry::new(Arc::new(config.clone()), HashMap::new());
        let (first, _) = files.publish(ServerRegion::Jp).await.unwrap();
        assert!(first.git_commit.is_some());
        std::fs::write(master.join("cards.json"), "[{\"id\":2}]").unwrap();
        write_version(&root, "2");
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "v2"]);
        files.publish(ServerRegion::Jp).await.unwrap();

        let db = root.join("state.db");
        let pg = blob_registry(&config, &format!("sqlite://{}?mode=rwc", db.display())).await;
        let stats = pg.import_blobs().await;
        assert_eq!((stats.stored, stats.unavailable), (2, 0));
        let old = &first.files[0].sha256;
        let blob = pg.blobs.get(old, None).await.unwrap().unwrap();
        let mut body = String::new();
        use std::io::Read;
        blob.into_reader()
            .unwrap()
            .read_to_string(&mut body)
            .unwrap();
        assert_eq!(body, "[{\"id\":1}]");
        std::fs::remove_dir_all(root).unwrap();
    }

    async fn current_etag(client: &reqwest::Client, base: &str) -> String {
        client
            .get(format!("{base}/v1/master/jp/current"))
            .send()
            .await
            .unwrap()
            .headers()["etag"]
            .to_str()
            .unwrap()
            .to_string()
    }

    #[tokio::test]
    async fn serves_manifest_files_bundle_and_app_identity_with_token_gating() {
        let root = temp_dir();
        let config = Arc::new(base_config(&root, "secret"));
        std::fs::write(root.join("master/cards.json"), "[{\"id\":1}]").unwrap();
        write_version(&root, "5.6.1.11");
        let registry = Arc::new(Registry::new(config, HashMap::new()));
        registry.publish_missing().await;
        let (base, server) = serve(router(registry.clone())).await;
        let client = reqwest::Client::new();

        let (status, health) = get_json(&client, &format!("{base}/health")).await;
        assert_eq!(status, 200);
        assert_eq!(health["regions"]["jp"]["dataVersion"], "5.6.1.11");
        assert_eq!(health["regions"]["jp"]["synced"], false);

        let (status, current) = get_json(&client, &format!("{base}/v1/master/jp/current")).await;
        assert_eq!(status, 200);
        assert_eq!(current["files"][0]["name"], "cards.json");
        assert_eq!(current["appHash"], "owner-hash");
        let (status, _) = get_json(&client, &format!("{base}/v1/master/kr/current")).await;
        assert_eq!(status, 404);
        let (status, _) = get_json(&client, &format!("{base}/v1/master/xx/current")).await;
        assert_eq!(status, 400);

        let (status, history) =
            get_json(&client, &format!("{base}/v1/master/jp/history?limit=5")).await;
        assert_eq!(status, 200);
        assert_eq!(history.as_array().unwrap().len(), 1);

        let resp = client
            .get(format!("{base}/v1/master/jp/files/cards.json"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let file_etag = resp.headers()["etag"].to_str().unwrap().to_string();
        assert_eq!(
            file_etag,
            format!("\"{}\"", current["files"][0]["sha256"].as_str().unwrap())
        );
        assert_eq!(resp.headers()["content-length"], "10");
        assert!(resp.headers().contains_key("last-modified"));
        assert_eq!(resp.text().await.unwrap(), "[{\"id\":1}]");
        let resp = client
            .get(format!("{base}/v1/master/jp/files/cards.json"))
            .header("if-none-match", &file_etag)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 304);
        assert!(resp.bytes().await.unwrap().is_empty());
        let resp = client
            .get(format!("{base}/v1/master/jp/files/cards.json"))
            .header("if-none-match", "W/\"stale\", \"other\"")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        // Digest-addressed blob: immutable, same bytes, stale digests are 404.
        let sha = current["files"][0]["sha256"].as_str().unwrap().to_string();
        let resp = client
            .get(format!("{base}/v1/master/jp/blob/{sha}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()["cache-control"], IMMUTABLE);
        assert_eq!(resp.headers()["etag"], format!("\"{sha}\"").as_str());
        assert_eq!(resp.text().await.unwrap(), "[{\"id\":1}]");
        for bad in [
            "0000000000000000000000000000000000000000000000000000000000000000",
            "nothex",
            "..%2Fx",
        ] {
            let resp = client
                .get(format!("{base}/v1/master/jp/blob/{bad}"))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 404, "{bad}");
        }
        // Immutable manifest snapshot by content hash.
        let content_hash = current["contentHash"].as_str().unwrap().to_string();
        let resp = client
            .get(format!("{base}/v1/master/jp/manifests/{content_hash}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()["cache-control"], IMMUTABLE);
        let snapshot: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(snapshot["dataVersion"], "5.6.1.11");
        let resp = client
            .get(format!("{base}/v1/master/jp/manifests/deadbeef"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
        // The manifest revalidates the same way.
        let resp = client
            .get(format!("{base}/v1/master/jp/current"))
            .send()
            .await
            .unwrap();
        let manifest_etag = resp.headers()["etag"].to_str().unwrap().to_string();
        assert_eq!(resp.headers()["cache-control"], "no-cache");
        assert_eq!(resp.headers()["x-haruki-data-version"], "5.6.1.11");
        let resp = client
            .get(format!("{base}/v1/master/jp/current"))
            .header("if-none-match", &manifest_etag)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 304);
        for bad in [
            "missing.json",
            "..%2Fversion.json",
            "note.txt",
            ".hidden.json",
        ] {
            let resp = client
                .get(format!("{base}/v1/master/jp/files/{bad}"))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 404, "{bad}");
        }

        let resp = client
            .get(format!("{base}/v1/master/jp/bundle"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()["content-type"], "application/x-tar");
        assert!(resp.bytes().await.unwrap().len() > 512);

        // App identity falls back to the synced version file...
        let (status, app) = get_json(&client, &format!("{base}/v1/app/jp")).await;
        assert_eq!(status, 200);
        assert_eq!(app["appVersion"], "5.6.0");
        assert_eq!(app["appHash"], "owner-hash");
        // ...until an operator override is stored (token required).
        let body = serde_json::json!({"appVersion": "5.7.0", "appHash": "manual"});
        let resp = client
            .put(format!("{base}/v1/app/jp"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);
        let resp = client
            .put(format!("{base}/v1/app/jp"))
            .bearer_auth("secret")
            .body("not json")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
        let resp = client
            .put(format!("{base}/v1/app/jp"))
            .bearer_auth("secret")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let put_body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(put_body["appVersion"], "5.7.0");
        assert_eq!(put_body["pushed"], serde_json::json!([]));
        let (_, app) = get_json(&client, &format!("{base}/v1/app/jp")).await;
        assert_eq!(app["appVersion"], "5.7.0");
        // A partial PUT keeps the other field from the effective identity.
        let resp = client
            .put(format!("{base}/v1/app/jp"))
            .bearer_auth("secret")
            .json(&serde_json::json!({"appVersion": "5.7.1"}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let (_, app) = get_json(&client, &format!("{base}/v1/app/jp")).await;
        assert_eq!(app["appVersion"], "5.7.1");
        assert_eq!(app["appHash"], "manual");
        // A region with no current identity at all cannot take a partial PUT.
        let resp = client
            .put(format!("{base}/v1/app/kr"))
            .bearer_auth("secret")
            .json(&serde_json::json!({"appVersion": "1.0.0"}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
        let resp = client
            .put(format!("{base}/v1/app/jp"))
            .bearer_auth("secret")
            .json(&serde_json::json!({"appVersion": "", "appHash": ""}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
        let resp = client
            .delete(format!("{base}/v1/app/jp"))
            .bearer_auth("secret")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let (_, app) = get_json(&client, &format!("{base}/v1/app/jp")).await;
        assert_eq!(app["appVersion"], "5.6.0");

        // Publish re-scans: a changed file becomes a new history entry.
        std::fs::write(root.join("master/cards.json"), "[{\"id\":2}]").unwrap();
        let resp = client
            .post(format!("{base}/v1/master/jp/publish"))
            .bearer_auth("secret")
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["changed"], true);
        let (_, history) = get_json(&client, &format!("{base}/v1/master/jp/history")).await;
        assert_eq!(history.as_array().unwrap().len(), 2);
        // The old blob digest no longer resolves once the file changed; the
        // new manifest's digest does.
        let resp = client
            .get(format!("{base}/v1/master/jp/blob/{sha}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
        let (_, refreshed) = get_json(&client, &format!("{base}/v1/master/jp/current")).await;
        let new_sha = refreshed["files"][0]["sha256"].as_str().unwrap();
        let resp = client
            .get(format!("{base}/v1/master/jp/blob/{new_sha}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        // Changed content: the old ETags no longer match on file or manifest.
        let resp = client
            .get(format!("{base}/v1/master/jp/files/cards.json"))
            .header("if-none-match", &file_etag)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_ne!(resp.headers()["etag"], file_etag.as_str());
        let resp = client
            .get(format!("{base}/v1/master/jp/current"))
            .header("if-none-match", &manifest_etag)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        // No syncer for jp: refresh and webhook report not found.
        let resp = client
            .post(format!("{base}/v1/master/jp/refresh"))
            .bearer_auth("secret")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
        let resp = client
            .post(format!("{base}/internal/master-updated"))
            .bearer_auth("secret")
            .json(&serde_json::json!({"server": "jp", "dataVersion": "x"}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
        server.abort();

        // With no token configured, mutations do not exist.
        let config = Arc::new(base_config(&root, ""));
        let registry = Arc::new(Registry::new(config, HashMap::new()));
        let (base, server) = serve(router(registry)).await;
        let resp = client
            .put(format!("{base}/v1/app/jp"))
            .body("not json")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404, "disabled endpoints hide body errors");
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Fake owner node: serves the version file and a one-file bundle.
    fn owner_bundle(root: &std::path::Path, data_version: &str) -> Vec<u8> {
        let source = root.join("owner");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("cards.json"), "[{\"id\":7}]").unwrap();
        let version = format!(
            r#"{{"appVersion":"5.6.0","appHash":"owner-hash","dataVersion":"{data_version}","assetVersion":"5.6.1.10","cdnVersion":0}}"#
        );
        std::fs::write(source.join(BUNDLE_VERSION_ENTRY), &version).unwrap();
        let tar_path = root.join("owner.tar");
        let mut builder = tar::Builder::new(std::fs::File::create(&tar_path).unwrap());
        builder
            .append_path_with_name(source.join("cards.json"), "cards.json")
            .unwrap();
        builder
            .append_path_with_name(source.join(BUNDLE_VERSION_ENTRY), BUNDLE_VERSION_ENTRY)
            .unwrap();
        builder.finish().unwrap();
        std::fs::read(tar_path).unwrap()
    }

    #[derive(Clone)]
    struct Owner {
        version: Vec<u8>,
        bundle: Vec<u8>,
        hits: Arc<parking_lot::Mutex<Vec<String>>>,
    }

    async fn owner_handler(
        State(owner): State<Owner>,
        uri: axum::http::Uri,
        body: String,
    ) -> HttpResponse<Body> {
        owner.hits.lock().push(format!("{} {}", uri.path(), body));
        let (content_type, bytes) = if uri.path().ends_with("/version") {
            ("application/json", owner.version.clone())
        } else if uri.path().ends_with("/bundle") {
            ("application/x-tar", owner.bundle.clone())
        } else {
            ("application/json", b"{\"ok\":true}".to_vec())
        };
        HttpResponse::builder()
            .header("content-type", content_type)
            .body(Body::from(bytes))
            .unwrap()
    }

    #[tokio::test]
    async fn webhook_pulls_from_owner_publishes_and_notifies_subscribers() {
        let root = temp_dir();
        let version = r#"{"appVersion":"5.6.0","appHash":"owner-hash","dataVersion":"9.0.0.1","assetVersion":"9.0.0.1","cdnVersion":0}"#.to_string();
        let owner = Owner {
            version: version.into_bytes(),
            bundle: owner_bundle(&root, "9.0.0.1"),
            hits: Arc::new(parking_lot::Mutex::new(Vec::new())),
        };
        let owner_app = Router::new()
            .fallback(any(owner_handler))
            .with_state(owner.clone());
        let (owner_url, owner_server) = serve(owner_app).await;

        let mut config = base_config(&root, "secret");
        {
            let server = config.servers.get_mut(&ServerRegion::Jp).unwrap();
            server.master_sync.source_url = owner_url.clone();
            server.master_sync.source_token = "owner-token".to_string();
        }
        // The owner doubles as the subscriber so the notice lands in `hits`.
        config.registry.subscribers = vec![MasterSyncPeer {
            url: owner_url.clone(),
            token: "sub-token".to_string(),
        }];
        let version_locks = HashMap::new();
        let syncers = build_syncers(&config, &HashMap::new(), None, &version_locks);
        assert!(syncers.contains_key(&ServerRegion::Jp));
        let registry = Arc::new(Registry::new(Arc::new(config), syncers));
        let (base, server) = serve(router(registry.clone())).await;
        let client = reqwest::Client::new();

        let (status, _) = get_json(&client, &format!("{base}/v1/master/jp/current")).await;
        assert_eq!(status, 404);
        let resp = client
            .post(format!("{base}/internal/master-updated"))
            .bearer_auth("secret")
            .json(&serde_json::json!({"server": "jp", "dataVersion": "9.0.0.1"}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        let mut published = None;
        for _ in 0..100 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if let Ok(Some(m)) = registry.state.current(ServerRegion::Jp).await {
                published = Some(m);
                break;
            }
        }
        let manifest = published.expect("webhook sync should publish");
        assert_eq!(manifest.data_version, "9.0.0.1");
        assert_eq!(manifest.files[0].name, "cards.json");
        assert!(root.join("master/cards.json").exists());
        let hits = owner.hits.lock().clone();
        assert!(hits
            .iter()
            .any(|h| h.contains("/internal/master/jp/version")));
        assert!(hits
            .iter()
            .any(|h| h.contains("/internal/master/jp/bundle")));
        let notice = hits
            .iter()
            .find(|h| h.contains("/internal/master-updated") && h.contains("9.0.0.1"))
            .expect("subscriber notified");
        let body: serde_json::Value =
            serde_json::from_str(notice.split_once(' ').unwrap().1).unwrap();
        assert_eq!(body["server"], "jp");
        assert_eq!(body["dataVersion"], "9.0.0.1");
        assert_eq!(
            body["contentHash"],
            super::super::state::content_hash(&manifest)
        );
        assert_eq!(body["changedFiles"], serde_json::json!(["cards.json"]));
        assert_eq!(body["removedFiles"], serde_json::json!([]));
        // Old receivers read only `server`/`dataVersion` and still parse it.
        let old: MasterUpdatedNotice = serde_json::from_value(body).unwrap();
        assert_eq!(old.data_version, "9.0.0.1");

        // A second refresh finds nothing new and publishes nothing.
        assert!(!registry.refresh(ServerRegion::Jp).await.unwrap());
        let (_, history) = get_json(&client, &format!("{base}/v1/master/jp/history")).await;
        assert_eq!(history.as_array().unwrap().len(), 1);

        let resp = client
            .post(format!("{base}/v1/master/jp/refresh"))
            .bearer_auth("secret")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        server.abort();
        owner_server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn put_app_identity_pushes_to_account_nodes() {
        let root = temp_dir();
        let received = Arc::new(parking_lot::Mutex::new(Vec::<serde_json::Value>::new()));
        let node = Router::new().route(
            "/internal/app-identity",
            axum::routing::post({
                let received = received.clone();
                move |headers: HeaderMap, body: String| {
                    let received = received.clone();
                    async move {
                        let authed = headers.get("authorization").and_then(|v| v.to_str().ok())
                            == Some("Bearer node-token");
                        received.lock().push(serde_json::from_str(&body).unwrap());
                        if authed {
                            json(&serde_json::json!({"ok": true, "data": {"changed": true}}))
                        } else {
                            (StatusCode::UNAUTHORIZED, "").into_response()
                        }
                    }
                }
            }),
        );
        let (node_url, node_server) = serve(node).await;
        let mut config = base_config(&root, "secret");
        config.registry.account_nodes = vec![
            MasterSyncPeer {
                url: node_url.clone(),
                token: "node-token".to_string(),
            },
            MasterSyncPeer {
                url: node_url.clone(),
                token: "wrong".to_string(),
            },
            MasterSyncPeer {
                url: "http://127.0.0.1:1".to_string(),
                token: String::new(),
            },
        ];
        let registry = Arc::new(Registry::new(Arc::new(config), HashMap::new()));
        let (base, server) = serve(router(registry)).await;
        let client = reqwest::Client::new();
        let resp = client
            .put(format!("{base}/v1/app/en"))
            .bearer_auth("secret")
            .json(&serde_json::json!({"appVersion": "5.7.0", "appHash": "new-hash"}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        let pushed = body["pushed"].as_array().unwrap();
        assert_eq!(pushed.len(), 3);
        assert_eq!(pushed[0]["ok"], true);
        assert_eq!(pushed[1]["ok"], false);
        assert_eq!(pushed[1]["message"], "HTTP 401 Unauthorized");
        assert_eq!(pushed[2]["ok"], false);
        let seen = received.lock().clone();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0]["server"], "en");
        assert_eq!(seen[0]["appVersion"], "5.7.0");
        assert_eq!(seen[0]["appHash"], "new-hash");
        server.abort();
        node_server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn serves_music_metas_pointer_file_and_blobs() {
        let root = temp_dir();
        let upstream_body = serde_json::to_vec(&serde_json::json!([
            {"music_id": 1, "difficulty": "master", "music_time": 100.0, "event_rate": 100, "base_score": 1.0,
             "base_score_auto": 1.0, "fever_score": 1.0, "fever_end_time": 1.0, "tap_count": 10,
             "skill_score_solo": [1,1,1,1,1,1], "skill_score_auto": [1,1,1,1,1,1], "skill_score_multi": [1,1,1,1,1,1]}
        ]))
        .unwrap();
        let upstream = Router::new().fallback(any({
            let body = upstream_body.clone();
            move || {
                let body = body.clone();
                async move {
                    (
                        StatusCode::OK,
                        [("content-type", "application/json"), ("etag", "\"u1\"")],
                        body,
                    )
                }
            }
        }));
        let (upstream_url, upstream_server) = serve(upstream).await;

        let mut config = base_config(&root, "secret");
        config
            .registry
            .music_metas
            .sources
            .insert(ServerRegion::Jp, format!("{upstream_url}/music_metas.json"));
        // Disable the other regions so nothing reaches the real upstream.
        for region in [
            ServerRegion::En,
            ServerRegion::Tw,
            ServerRegion::Kr,
            ServerRegion::Cn,
        ] {
            config
                .registry
                .music_metas
                .sources
                .insert(region, String::new());
        }
        let registry = Arc::new(Registry::new(Arc::new(config), HashMap::new()));
        assert_eq!(
            registry.metas.as_ref().unwrap().regions(),
            vec![ServerRegion::Jp]
        );
        let (base, server) = serve(router(registry.clone())).await;
        let client = reqwest::Client::new();

        let (status, _) = get_json(&client, &format!("{base}/v1/metas/jp/current")).await;
        assert_eq!(status, 404);
        let resp = client
            .post(format!("{base}/v1/metas/jp/refresh"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);
        let resp = client
            .post(format!("{base}/v1/metas/jp/refresh"))
            .bearer_auth("secret")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let outcome: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(outcome["changed"], true);
        assert_eq!(outcome["rows"], 4);
        let sha = outcome["sha256"].as_str().unwrap().to_string();

        let resp = client
            .get(format!("{base}/v1/metas/jp/current"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()["cache-control"], "no-cache");
        let pointer_etag = resp.headers()["etag"].to_str().unwrap().to_string();
        assert_eq!(pointer_etag, format!("\"{sha}\""));
        let record: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(record["omakaseInjected"], true);
        assert_eq!(record["sourceEtag"], "\"u1\"");
        let resp = client
            .get(format!("{base}/v1/metas/jp/current"))
            .header("if-none-match", &pointer_etag)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 304);

        let resp = client
            .get(format!("{base}/v1/metas/jp/music_metas.json"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()["cache-control"], "no-cache");
        assert_eq!(resp.headers()["etag"], pointer_etag.as_str());
        let served: Vec<serde_json::Value> = resp.json().await.unwrap();
        assert_eq!(served.len(), 4);
        assert_eq!(served[3]["music_id"], 10000);
        let resp = client
            .get(format!("{base}/v1/metas/jp/music_metas.json"))
            .header("if-none-match", &pointer_etag)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 304);

        let resp = client
            .get(format!("{base}/v1/metas/jp/blob/{sha}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()["cache-control"], IMMUTABLE);
        assert_eq!(
            resp.headers()["content-length"],
            record["size"].to_string().as_str()
        );
        let resp = client
            .get(format!("{base}/v1/metas/jp/blob/nothex"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
        let (status, _) = get_json(&client, &format!("{base}/v1/metas/en/current")).await;
        assert_eq!(status, 404);
        server.abort();
        upstream_server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }
}
