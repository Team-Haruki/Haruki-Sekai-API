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
//! - `GET /v1/master/{region}/blob/{sha256}`      immutable master file by digest
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
use tracing::{error, info};

use super::Registry;
use crate::api::internal::{build_master_tar, file_sha256, MasterUpdatedNotice};
use crate::client::helper::AppInfo;
use crate::config::ServerRegion;
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

async fn health(State(registry): State<Shared>) -> Response {
    let mut regions = serde_json::Map::new();
    for region in registry.regions() {
        let current = registry.state.current(region).await.ok().flatten();
        regions.insert(
            region.as_str().to_string(),
            serde_json::json!({
                "dataVersion": current.as_ref().map(|m| m.data_version.clone()),
                "publishedAt": current.as_ref().map(|m| m.generated_at.clone()),
                "synced": registry.syncers.contains_key(&region),
            }),
        );
    }
    json(&serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
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

/// Stream a file as an immutable, digest-tagged response.
async fn immutable_file(path: &std::path::Path, digest: &str, content_type: &str) -> Response {
    let meta = match tokio::fs::metadata(path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return AppError::NotFound(format!("no blob {digest}")).into_response()
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
        Ok(None) => AppError::NotFound(format!("no manifest {hash}")).into_response(),
        Err(e) => e.into_response(),
    }
}

/// A master file by its SHA-256 (from the manifest): immutable, so a CDN
/// keeps serving it across versions for every table that did not change.
/// Only files of the current manifest are addressable; a stale digest is a
/// 404 and the consumer re-reads `current`.
async fn blob(
    State(registry): State<Shared>,
    Path((region, sha256)): Path<(String, String)>,
) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    if !super::state::is_hex_digest(&sha256) {
        return AppError::NotFound(format!("no blob {sha256}")).into_response();
    }
    let manifest = match registry.state.current(region).await {
        Ok(Some(m)) => m,
        Ok(None) => return AppError::NotFound(format!("no blob {sha256}")).into_response(),
        Err(e) => return e.into_response(),
    };
    let Some(entry) = manifest.files.iter().find(|f| f.sha256 == sha256) else {
        return AppError::NotFound(format!("no blob {sha256}")).into_response();
    };
    let (master_dir, _) = match registry.region_paths(region) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let path = std::path::Path::new(&master_dir).join(&entry.name);
    // The file may have been rewritten since the manifest was published;
    // never serve different bytes under a digest URL.
    let actual = {
        let path = path.clone();
        match tokio::fs::metadata(&path).await {
            Ok(meta) => {
                match tokio::task::spawn_blocking(move || file_sha256(&path, &meta)).await {
                    Ok(Ok(d)) => d,
                    Ok(Err(e)) => return e.into_response(),
                    Err(e) => {
                        return AppError::Internal(format!("digest task: {e}")).into_response()
                    }
                }
            }
            Err(_) => return AppError::NotFound(format!("no blob {sha256}")).into_response(),
        }
    };
    if actual != sha256 {
        return AppError::NotFound(format!("no blob {sha256}")).into_response();
    }
    immutable_file(&path, &sha256, "application/json").await
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
        return AppError::NotFound(format!("no blob {sha256}")).into_response();
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

async fn bundle(State(registry): State<Shared>, Path(region): Path<String>) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
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
    match registry.state.set_app_identity(region, &info).await {
        Ok(()) => {
            info!(
                "{} App identity override set: appVersion={} appHash={}",
                region.as_str().to_uppercase(),
                info.app_version,
                info.app_hash.chars().take(16).collect::<String>()
            );
            let pushed = registry.push_app_identity(region, &info).await;
            json(&serde_json::json!({
                "appVersion": info.app_version,
                "appHash": info.app_hash,
                "pushed": pushed,
            }))
        }
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
        assert!(hits
            .iter()
            .any(|h| h.contains("/internal/master-updated") && h.contains("9.0.0.1")));

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
