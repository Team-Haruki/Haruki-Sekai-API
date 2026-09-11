//! Node-to-node internal API: lets a peer Haruki Sekai API node execute a game
//! API call on this node's local accounts.
//!
//! Trust model: the caller is another Haruki node holding this node's
//! `backend.internal_token`, reached over the internal network (Tailscale).
//! With the token unset the endpoint is disabled and answers 404, so a node
//! never exposes forwarding by default. Requests execute on the local
//! [`SekaiClient`] only — never through this node's own upstream router — so
//! forwarding cannot loop between nodes.

use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::Value as JsonValue;

use crate::config::ServerRegion;
use crate::error::AppError;
use crate::upstream::{
    GameStreamRequest, InternalApiRequest, InternalApiResponse, InternalImageRequest,
    LoginProbeRequest, LoginProbeResponse,
};
use crate::AppState;

/// Timeout for a relayed master-split GET; matches the CP master split timeout
/// used by the local master updater.
const GAME_STREAM_TIMEOUT_SECS: u64 = 120;

fn envelope_response(envelope: &InternalApiResponse) -> Response {
    // serde_json (preserve_order) keeps game response key order intact;
    // sonic_rs must not be used here.
    match serde_json::to_string(envelope) {
        Ok(json) => (StatusCode::OK, [("content-type", "application/json")], json).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("envelope serialization failed: {e}"),
        )
            .into_response(),
    }
}

fn error_envelope(e: &AppError) -> InternalApiResponse {
    let status = match e {
        AppError::Unknown { status, .. } => Some(*status),
        AppError::InvalidHttpStatus(status) => Some(*status),
        _ => None,
    };
    InternalApiResponse {
        ok: false,
        status,
        data: None,
        kind: Some(e.kind().to_string()),
        message: Some(e.to_string()),
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// Gate an internal endpoint on `backend.internal_token`. Returns the error
/// response to send when the caller is not an authorized peer node, None when
/// the call is authorized.
fn check_internal_auth(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    let expected = &state.config.backend.internal_token;
    if expected.is_empty() {
        // Endpoints disabled: indistinguishable from an unknown route.
        return Some(StatusCode::NOT_FOUND.into_response());
    }
    if bearer_token(headers) != Some(expected.as_str()) {
        return Some(StatusCode::UNAUTHORIZED.into_response());
    }
    None
}

pub async fn post_sekai_api(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<InternalApiRequest>,
) -> Response {
    if let Some(resp) = check_internal_auth(&state, &headers) {
        return resp;
    }

    let region: ServerRegion = match req.server.parse() {
        Ok(r) => r,
        Err(_) => {
            return envelope_response(&error_envelope(&AppError::InvalidServerRegion(
                req.server.clone(),
            )));
        }
    };
    let Some(client) = state.clients.get(&region) else {
        return envelope_response(&error_envelope(&AppError::NoClientAvailable));
    };

    let result = match req.method.as_str() {
        "GET" => client.get_game_api(&req.path, req.params.as_ref()).await,
        "POST" => {
            let body = req.body.clone().unwrap_or(JsonValue::Null);
            client
                .post_game_api_body(&req.path, &body, req.params.as_ref())
                .await
        }
        other => Err(AppError::ParseError(format!(
            "unsupported method '{other}'"
        ))),
    };

    match result {
        Ok((data, status)) => envelope_response(&InternalApiResponse {
            ok: true,
            status: Some(status),
            data: Some(data),
            kind: None,
            message: None,
        }),
        Err(e) => envelope_response(&error_envelope(&e)),
    }
}

/// POST /internal/login-probe — log in with this node's own accounts for the
/// given region and return only the version metadata the login yields, so a
/// peer can drive master production without holding accounts. Session tokens
/// and credentials never leave this node.
pub async fn post_login_probe(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<LoginProbeRequest>,
) -> Response {
    if let Some(resp) = check_internal_auth(&state, &headers) {
        return resp;
    }
    let region: ServerRegion = match req.server.parse() {
        Ok(r) => r,
        Err(_) => {
            return envelope_response(&error_envelope(&AppError::InvalidServerRegion(
                req.server.clone(),
            )));
        }
    };
    let Some(client) = state.clients.get(&region) else {
        return envelope_response(&error_envelope(&AppError::NoClientAvailable));
    };
    let Some(session) = client.get_session() else {
        return envelope_response(&error_envelope(&AppError::NoAccountError));
    };
    // Hold the account's api lock for the whole login: the probe rotates the
    // one-time session token, so racing an in-flight serving request on the
    // same account would invalidate that request's token mid-call and force a
    // relogin retry. Serializing here keeps probe ticks invisible to serving.
    let _api_guard = session.lock_api().await;
    // Mirror the master updater's own login recovery: a 426 means the version
    // file moved on, so refresh it and retry once.
    let login = match client.login(&session).await {
        Ok(r) => Ok(r),
        Err(AppError::UpgradeRequired) => match client.refresh_version().await {
            Ok(()) => client.login(&session).await,
            Err(e) => Err(e),
        },
        Err(e) => Err(e),
    };
    let probe = match login {
        Ok(login) => {
            // Report the identity this login actually used (post-426 refresh
            // included), not whatever the caller has on disk.
            let app = client.version_helper.get();
            LoginProbeResponse {
                ok: true,
                kind: None,
                message: None,
                data_version: login.data_version,
                asset_version: login.asset_version,
                asset_hash: login.asset_hash,
                cdn_version: login.cdn_version,
                suite_master_split_path: login.suite_master_split_path,
                app_version: app.app_version,
                app_hash: app.app_hash,
            }
        }
        Err(e) => LoginProbeResponse {
            ok: false,
            kind: Some(e.kind().to_string()),
            message: Some(e.to_string()),
            data_version: String::new(),
            asset_version: String::new(),
            asset_hash: String::new(),
            cdn_version: 0,
            suite_master_split_path: Vec::new(),
            app_version: String::new(),
            app_hash: String::new(),
        },
    };
    match serde_json::to_string(&probe) {
        Ok(json) => (StatusCode::OK, [("content-type", "application/json")], json).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("probe serialization failed: {e}"),
        )
            .into_response(),
    }
}

/// POST /internal/game-stream — execute an authenticated game GET with this
/// node's accounts and relay the response body back UNTOUCHED (still
/// AES-encrypted), streamed chunk by chunk so this node never buffers or
/// decodes it. Success is an octet stream; errors come back as a 200 JSON
/// envelope (content type disambiguates), keeping the "non-200 transport
/// status = node fault" convention.
pub async fn post_game_stream(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<GameStreamRequest>,
) -> Response {
    if let Some(resp) = check_internal_auth(&state, &headers) {
        return resp;
    }
    let region: ServerRegion = match req.server.parse() {
        Ok(r) => r,
        Err(_) => {
            return envelope_response(&error_envelope(&AppError::InvalidServerRegion(
                req.server.clone(),
            )));
        }
    };
    let Some(client) = state.clients.get(&region) else {
        return envelope_response(&error_envelope(&AppError::NoClientAvailable));
    };
    match client
        .get_game_api_raw(
            &req.path,
            std::time::Duration::from_secs(GAME_STREAM_TIMEOUT_SECS),
        )
        .await
    {
        Ok(resp) => {
            let game_status = resp.status().as_u16();
            let stream = resp.bytes_stream();
            (
                StatusCode::OK,
                [
                    ("content-type", "application/octet-stream".to_string()),
                    ("x-haruki-game-status", game_status.to_string()),
                ],
                axum::body::Body::from_stream(stream),
            )
                .into_response()
        }
        Err(e) => envelope_response(&error_envelope(&e)),
    }
}

/// POST /internal/sekai-image — fetch an image with this node's local client
/// for a peer node. Success is raw bytes with the kind's content type; errors
/// come back as a 200 JSON envelope (the content type disambiguates), keeping
/// the "non-200 transport status = node fault" failover convention intact.
pub async fn post_sekai_image(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<InternalImageRequest>,
) -> Response {
    if let Some(resp) = check_internal_auth(&state, &headers) {
        return resp;
    }
    let region: ServerRegion = match req.server.parse() {
        Ok(r) => r,
        Err(_) => {
            return envelope_response(&error_envelope(&AppError::InvalidServerRegion(
                req.server.clone(),
            )));
        }
    };
    let Some(client) = state.clients.get(&region) else {
        return envelope_response(&error_envelope(&AppError::NoClientAvailable));
    };
    match crate::upstream::execute_local_image(client, req.kind, &req.param1, &req.param2).await {
        Ok(bytes) => (
            StatusCode::OK,
            [("content-type", req.kind.content_type())],
            bytes,
        )
            .into_response(),
        Err(e) => envelope_response(&error_envelope(&e)),
    }
}

/// Resolve a `{server}` path segment to the region's ServerConfig. Master
/// endpoints work from config alone so an owner node can serve them even for
/// a region whose client failed to initialize.
fn region_config<'a>(
    state: &'a AppState,
    server: &str,
) -> Result<(ServerRegion, &'a crate::config::ServerConfig), Box<Response>> {
    let region: ServerRegion = server.parse().map_err(|_| {
        Box::new(envelope_response(&error_envelope(
            &AppError::InvalidServerRegion(server.to_string()),
        )))
    })?;
    let config = state.config.servers.get(&region).ok_or_else(|| {
        Box::new(envelope_response(&error_envelope(&AppError::NotFound(
            format!("region {} not configured", server),
        ))))
    })?;
    Ok((region, config))
}

/// GET /internal/master/{server}/version — the region's version file, for
/// cheap peer polling.
pub async fn get_master_version(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(server): axum::extract::Path<String>,
) -> Response {
    if let Some(resp) = check_internal_auth(&state, &headers) {
        return resp;
    }
    let (_region, config) = match region_config(&state, &server) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    if config.version_path.is_empty() {
        return envelope_response(&error_envelope(&AppError::NotFound(
            "version_path not configured".to_string(),
        )));
    }
    match tokio::fs::read(&config.version_path).await {
        Ok(data) => match sonic_rs::from_slice::<crate::client::helper::VersionInfo>(&data) {
            Ok(info) => match serde_json::to_string(&info) {
                Ok(json) => {
                    (StatusCode::OK, [("content-type", "application/json")], json).into_response()
                }
                Err(e) => envelope_response(&error_envelope(&AppError::ParseError(e.to_string()))),
            },
            Err(e) => envelope_response(&error_envelope(&AppError::ParseError(format!(
                "version file: {e}"
            )))),
        },
        Err(e) => envelope_response(&error_envelope(&AppError::IoError(format!(
            "version file: {e}"
        )))),
    }
}

/// One master file as listed by the manifest.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MasterManifestFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

/// What a region's master directory currently holds: the version identity
/// (as the version file records it) plus every `*.json` with its size and
/// SHA-256. Consumers and the master data manager use it to decide whether
/// to pull and which files changed, without downloading the bundle.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasterManifest {
    pub server: String,
    pub app_version: String,
    pub app_hash: String,
    pub data_version: String,
    pub asset_version: String,
    pub asset_hash: String,
    pub cdn_version: i32,
    pub generated_at: String,
    /// SHA-256 over the `name:sha256` lines of `files`: identifies the file
    /// set independently of when it was published. Immutable manifest and
    /// blob URLs key on it.
    #[serde(default)]
    pub content_hash: String,
    /// HEAD of the git repository the master directory lives in, when it is
    /// one (the five region mirrors are). The version reference consumers
    /// pin; `None` outside a repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,
    pub files: Vec<MasterManifestFile>,
}

/// Digest of a manifest's file set (names and digests only), independent of
/// the manifest's timestamp and version fields.
pub fn manifest_content_hash(files: &[MasterManifestFile]) -> String {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    for file in files {
        hasher.update(file.name.as_bytes());
        hasher.update(b":");
        hasher.update(file.sha256.as_bytes());
        hasher.update(b"\n");
    }
    hex::encode(hasher.finalize())
}

/// `git rev-parse HEAD` for the repository containing `master_dir`, if any.
fn git_head_of(master_dir: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(master_dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let head = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (head.len() == 40 && head.chars().all(|c| c.is_ascii_hexdigit())).then_some(head)
}

/// Digest cache keyed by path and validated by (mtime, size), so a manifest
/// request only hashes files that changed since the last one. Process-wide:
/// master directories are few and their entries change only on updates.
type DigestCache =
    std::collections::HashMap<std::path::PathBuf, (std::time::SystemTime, u64, String)>;
static DIGEST_CACHE: std::sync::LazyLock<parking_lot::Mutex<DigestCache>> =
    std::sync::LazyLock::new(Default::default);

/// SHA-256 of a master file, served from the (mtime, size)-validated cache.
pub fn file_sha256(path: &std::path::Path, meta: &std::fs::Metadata) -> Result<String, AppError> {
    use sha2::Digest as _;
    let key = (meta.modified()?, meta.len());
    if let Some((mtime, len, digest)) = DIGEST_CACHE.lock().get(path) {
        if (*mtime, *len) == key {
            return Ok(digest.clone());
        }
    }
    let mut file = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    let digest = hex::encode(hasher.finalize());
    DIGEST_CACHE
        .lock()
        .insert(path.to_path_buf(), (key.0, key.1, digest.clone()));
    Ok(digest)
}

/// Build the manifest for `master_dir` (blocking: hashes changed files).
pub fn build_master_manifest(
    region: ServerRegion,
    master_dir: &str,
    version_path: &str,
) -> Result<MasterManifest, AppError> {
    let version: crate::client::helper::VersionInfo = if version_path.is_empty() {
        Default::default()
    } else {
        let data = std::fs::read(version_path)
            .map_err(|e| AppError::IoError(format!("version file: {e}")))?;
        sonic_rs::from_slice(&data)
            .map_err(|e| AppError::ParseError(format!("version file: {e}")))?
    };
    let mut entries: Vec<(String, std::path::PathBuf, std::fs::Metadata)> = Vec::new();
    for entry in std::fs::read_dir(master_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Follow symlinks so the manifest describes the same files the bundle
        // archives (`build_master_tar` uses `Path::is_file`, which follows).
        let meta = std::fs::metadata(entry.path())?;
        if !meta.is_file() || !name.ends_with(".json") || name.starts_with('.') {
            continue;
        }
        entries.push((name, entry.path(), meta));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let files = entries
        .iter()
        .map(|(name, path, meta)| {
            Ok(MasterManifestFile {
                name: name.clone(),
                size: meta.len(),
                sha256: file_sha256(path, meta)?,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    Ok(MasterManifest {
        server: region.as_str().to_string(),
        app_version: version.app_version,
        app_hash: version.app_hash,
        data_version: version.data_version,
        asset_version: version.asset_version,
        asset_hash: version.asset_hash,
        cdn_version: version.cdn_version,
        generated_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        content_hash: manifest_content_hash(&files),
        git_commit: git_head_of(master_dir),
        files,
    })
}

/// GET /internal/master/{server}/manifest — version identity plus the
/// name/size/sha256 of every master file, for consumers that pin a version
/// or fetch only what changed.
pub async fn get_master_manifest(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(server): axum::extract::Path<String>,
) -> Response {
    if let Some(resp) = check_internal_auth(&state, &headers) {
        return resp;
    }
    let (region, config) = match region_config(&state, &server) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    if config.master_dir.is_empty() {
        return envelope_response(&error_envelope(&AppError::NotFound(
            "master_dir not configured".to_string(),
        )));
    }
    let master_dir = config.master_dir.clone();
    let version_path = config.version_path.clone();
    let built = tokio::task::spawn_blocking(move || {
        build_master_manifest(region, &master_dir, &version_path)
    })
    .await;
    let manifest = match built {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => return envelope_response(&error_envelope(&e)),
        Err(e) => {
            return envelope_response(&error_envelope(&AppError::Internal(format!(
                "manifest task: {e}"
            ))))
        }
    };
    match serde_json::to_string(&manifest) {
        Ok(json) => (StatusCode::OK, [("content-type", "application/json")], json).into_response(),
        Err(e) => envelope_response(&error_envelope(&AppError::ParseError(e.to_string()))),
    }
}

/// GET /internal/master/{server}/bundle — plain tar of the region's master
/// directory plus its version file (as `__haruki_version__.json`), streamed
/// from a temp file so large masters never sit in memory. Transport
/// compression is left to the CompressionLayer / Accept-Encoding negotiation.
pub async fn get_master_bundle(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(server): axum::extract::Path<String>,
) -> Response {
    if let Some(resp) = check_internal_auth(&state, &headers) {
        return resp;
    }
    let (region, config) = match region_config(&state, &server) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    if config.master_dir.is_empty() {
        return envelope_response(&error_envelope(&AppError::NotFound(
            "master_dir not configured".to_string(),
        )));
    }

    let tmp_path = std::env::temp_dir().join(format!(
        "haruki-master-bundle-{}-{}.tar",
        region.as_str(),
        uuid::Uuid::new_v4()
    ));
    let master_dir = config.master_dir.clone();
    let version_path = config.version_path.clone();
    let build_path = tmp_path.clone();
    let build = tokio::task::spawn_blocking(move || {
        build_master_tar(&master_dir, &version_path, &build_path)
    })
    .await;
    let build_result = match build {
        Ok(r) => r,
        Err(e) => Err(AppError::Internal(format!("bundle task: {e}"))),
    };
    if let Err(e) = build_result {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return envelope_response(&error_envelope(&e));
    }

    match tokio::fs::File::open(&tmp_path).await {
        Ok(file) => {
            // Unlink while the fd stays open: the file vanishes from the
            // filesystem now and its blocks are freed when streaming ends,
            // even if the response is aborted midway.
            let _ = tokio::fs::remove_file(&tmp_path).await;
            let stream = tokio_util::io::ReaderStream::new(file);
            (
                StatusCode::OK,
                [("content-type", "application/x-tar")],
                axum::body::Body::from_stream(stream),
            )
                .into_response()
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            envelope_response(&error_envelope(&AppError::IoError(e.to_string())))
        }
    }
}

/// Tar up every `*.json` in `master_dir` (flat, no directories) plus the
/// version file under `BUNDLE_VERSION_ENTRY`, writing to `out_path`.
pub fn build_master_tar(
    master_dir: &str,
    version_path: &str,
    out_path: &std::path::Path,
) -> Result<(), AppError> {
    let file = std::fs::File::create(out_path)?;
    let mut builder = tar::Builder::new(std::io::BufWriter::new(file));
    let mut count = 0usize;
    for entry in std::fs::read_dir(master_dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !path.is_file() || !name.ends_with(".json") || name.starts_with('.') {
            continue;
        }
        builder
            .append_path_with_name(&path, &name)
            .map_err(AppError::from)?;
        count += 1;
    }
    if count == 0 {
        return Err(AppError::NotFound("master directory is empty".to_string()));
    }
    if !version_path.is_empty() && std::path::Path::new(version_path).is_file() {
        builder
            .append_path_with_name(version_path, crate::updater::sync::BUNDLE_VERSION_ENTRY)
            .map_err(AppError::from)?;
    }
    let writer = builder.into_inner().map_err(AppError::from)?;
    use std::io::Write;
    writer
        .into_inner()
        .map_err(|e| AppError::IoError(e.to_string()))?
        .flush()
        .map_err(AppError::from)?;
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasterUpdatedNotice {
    pub server: String,
    #[serde(default)]
    pub data_version: String,
}

/// POST /internal/master-updated — webhook from a region's owner node after it
/// finished a master update. Triggers this node's syncer for the region in the
/// background; the fallback poll covers any failure of the spawned sync.
pub async fn post_master_updated(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(notice): Json<MasterUpdatedNotice>,
) -> Response {
    if let Some(resp) = check_internal_auth(&state, &headers) {
        return resp;
    }
    let region: ServerRegion = match notice.server.parse() {
        Ok(r) => r,
        Err(_) => {
            return envelope_response(&error_envelope(&AppError::InvalidServerRegion(
                notice.server.clone(),
            )));
        }
    };
    let Some(syncer) = state.syncers.get(&region).cloned() else {
        return envelope_response(&error_envelope(&AppError::NotFound(format!(
            "no master sync configured for region {}",
            notice.server
        ))));
    };
    tracing::info!(
        "{} Master-updated webhook received (dataVersion {:?}), starting sync",
        region.as_str().to_uppercase(),
        notice.data_version
    );
    tokio::spawn(async move {
        if let Err(e) = syncer.sync_once().await {
            tracing::error!(
                "{} Webhook-triggered master sync failed (fallback poll will retry): {}",
                region.as_str().to_uppercase(),
                e
            );
        }
    });
    envelope_response(&InternalApiResponse {
        ok: true,
        status: None,
        data: None,
        kind: None,
        message: Some("sync triggered".to_string()),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::body::to_bytes;
    use axum::extract::{Path, State};
    use axum::http::{HeaderMap, HeaderValue, StatusCode};
    use axum::Json;
    use axum::{http::Uri, Router};

    use super::*;
    use crate::client::SekaiClient;
    use crate::config::{Config, ServerConfig};
    use crate::upstream::{ImageKind, InternalImageRequest};
    use crate::RequestCoalescer;

    const KEY: &str = "00112233445566778899aabbccddeeff";
    const IV: &str = "ffeeddccbbaa99887766554433221100";

    fn temp_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("haruki_internal_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn auth_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static("Bearer secret"));
        headers
    }

    async fn state(root: &std::path::Path, include_client: bool) -> Arc<AppState> {
        let mut config: Config =
            serde_yaml::from_str("backend:\n  internal_token: secret").unwrap();
        let mut server_config: ServerConfig = serde_yaml::from_str("{}").unwrap();
        server_config.aes_key_hex = KEY.to_string();
        server_config.aes_iv_hex = IV.to_string();
        server_config.account_dir = root.join("accounts").to_string_lossy().into_owned();
        server_config.version_path = root.join("version.json").to_string_lossy().into_owned();
        server_config.master_dir = root.join("master").to_string_lossy().into_owned();
        std::fs::create_dir_all(&server_config.account_dir).unwrap();
        std::fs::create_dir_all(&server_config.master_dir).unwrap();
        config
            .servers
            .insert(ServerRegion::Jp, server_config.clone());

        let mut clients = HashMap::new();
        if include_client {
            let client = SekaiClient::new(
                ServerRegion::Jp,
                server_config,
                None,
                None,
                SekaiClient::build_http_client(None).unwrap(),
                None,
            )
            .await
            .unwrap();
            clients.insert(ServerRegion::Jp, Arc::new(client));
        }
        Arc::new(AppState {
            config,
            clients,
            routers: HashMap::new(),
            syncers: HashMap::new(),
            version_locks: HashMap::new(),
            db: None,
            master_db: None,
            redis: None,
            jwt_secret: None,
            coalescer: Arc::new(RequestCoalescer::default()),
        })
    }

    async fn json_body(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[derive(Clone)]
    struct GameReply {
        login: Vec<u8>,
        game: Vec<u8>,
    }

    async fn game_handler(State(reply): State<GameReply>, uri: Uri) -> Response {
        if uri.path().contains("/auth") {
            return (
                StatusCode::OK,
                [("content-type", "application/octet-stream")],
                reply.login,
            )
                .into_response();
        }
        if uri.path().starts_with("/api/") {
            return (
                StatusCode::OK,
                [("content-type", "application/octet-stream")],
                reply.game,
            )
                .into_response();
        }
        (
            StatusCode::OK,
            [("content-type", "image/png")],
            vec![1, 2, 3],
        )
            .into_response()
    }

    async fn initialized_state(
        root: &std::path::Path,
    ) -> (Arc<AppState>, tokio::task::JoinHandle<()>) {
        let cryptor = crate::crypto::SekaiCryptor::from_hex(KEY, IV).unwrap();
        let app = Router::new().fallback(game_handler).with_state(GameReply {
            login: cryptor
                .pack(&serde_json::json!({
                    "sessionToken": "token",
                    "dataVersion": "data",
                    "assetVersion": "asset",
                    "assetHash": "hash"
                }))
                .unwrap(),
            game: cryptor.pack(&serde_json::json!({"ok": true})).unwrap(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let mut app_state = (*state(root, false).await).clone();
        let server_config = app_state.config.servers.get_mut(&ServerRegion::Jp).unwrap();
        server_config.api_url = format!("http://{address}");
        std::fs::write(
            &server_config.version_path,
            r#"{"appVersion":"1","appHash":"h","dataVersion":"d","assetVersion":"a"}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("accounts/account.json"),
            r#"{"userId":"1","deviceId":"device","credential":"credential"}"#,
        )
        .unwrap();
        let client = Arc::new(
            SekaiClient::new(
                ServerRegion::Jp,
                server_config.clone(),
                None,
                None,
                SekaiClient::build_http_client(None).unwrap(),
                None,
            )
            .await
            .unwrap(),
        );
        client.init().await.unwrap();
        app_state.clients.insert(ServerRegion::Jp, client);
        (Arc::new(app_state), server)
    }

    #[tokio::test]
    async fn authenticates_internal_endpoints_and_builds_error_envelopes() {
        let root = temp_dir();
        let state = state(&root, false).await;
        assert_eq!(bearer_token(&auth_headers()), Some("secret"));
        assert!(check_internal_auth(&state, &auth_headers()).is_none());
        assert_eq!(
            check_internal_auth(&state, &HeaderMap::new())
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );

        let mut disabled = (*state).clone();
        disabled.config.backend.internal_token.clear();
        assert_eq!(
            check_internal_auth(&disabled, &HeaderMap::new())
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );

        for error in [
            AppError::Unknown {
                status: 404,
                body: "missing".to_string(),
            },
            AppError::InvalidHttpStatus(418),
            AppError::NetworkError("down".to_string()),
        ] {
            let envelope = error_envelope(&error);
            assert!(!envelope.ok);
            let response = envelope_response(&envelope);
            assert_eq!(response.status(), StatusCode::OK);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn api_probe_stream_and_image_return_precise_precondition_errors() {
        let root = temp_dir();
        let missing = state(&root, false).await;
        let invalid_req = InternalApiRequest {
            server: "bad".to_string(),
            method: "GET".to_string(),
            path: "/x".to_string(),
            params: None,
            body: None,
        };
        let body = json_body(
            post_sekai_api(State(missing.clone()), auth_headers(), Json(invalid_req)).await,
        )
        .await;
        assert_eq!(body["kind"], "invalid_server_region");

        let req = InternalApiRequest {
            server: "jp".to_string(),
            method: "GET".to_string(),
            path: "/x".to_string(),
            params: None,
            body: None,
        };
        let body =
            json_body(post_sekai_api(State(missing.clone()), auth_headers(), Json(req)).await)
                .await;
        assert_eq!(body["kind"], "no_client");

        let with_client = state(&root, true).await;
        let unsupported = InternalApiRequest {
            server: "jp".to_string(),
            method: "DELETE".to_string(),
            path: "/x".to_string(),
            params: None,
            body: None,
        };
        let body = json_body(
            post_sekai_api(
                State(with_client.clone()),
                auth_headers(),
                Json(unsupported),
            )
            .await,
        )
        .await;
        assert_eq!(body["kind"], "parse");

        let probe = post_login_probe(
            State(with_client.clone()),
            auth_headers(),
            Json(LoginProbeRequest {
                server: "jp".to_string(),
            }),
        )
        .await;
        assert_eq!(json_body(probe).await["kind"], "no_account");
        let invalid_probe = post_login_probe(
            State(with_client.clone()),
            auth_headers(),
            Json(LoginProbeRequest {
                server: "bad".to_string(),
            }),
        )
        .await;
        assert_eq!(
            json_body(invalid_probe).await["kind"],
            "invalid_server_region"
        );

        let stream = post_game_stream(
            State(with_client.clone()),
            auth_headers(),
            Json(GameStreamRequest {
                server: "jp".to_string(),
                path: "/x".to_string(),
            }),
        )
        .await;
        assert_eq!(json_body(stream).await["kind"], "no_client");
        let image = post_sekai_image(
            State(with_client),
            auth_headers(),
            Json(InternalImageRequest {
                server: "jp".to_string(),
                kind: ImageKind::CpMysekai,
                param1: "a".to_string(),
                param2: "b".to_string(),
            }),
        )
        .await;
        assert_eq!(json_body(image).await["kind"], "no_client");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn executes_successful_local_api_probe_stream_and_image_calls() {
        let root = temp_dir();
        let (state, server) = initialized_state(&root).await;
        for (method, body) in [
            ("GET".to_string(), None),
            ("POST".to_string(), Some(serde_json::json!({"x": 1}))),
        ] {
            let response = post_sekai_api(
                State(state.clone()),
                auth_headers(),
                Json(InternalApiRequest {
                    server: "jp".to_string(),
                    method,
                    path: "/test".to_string(),
                    params: None,
                    body,
                }),
            )
            .await;
            let body = json_body(response).await;
            assert_eq!(body["ok"], true);
            assert_eq!(body["data"]["ok"], true);
        }

        let probe = post_login_probe(
            State(state.clone()),
            auth_headers(),
            Json(LoginProbeRequest {
                server: "jp".to_string(),
            }),
        )
        .await;
        let body = json_body(probe).await;
        assert_eq!(body["ok"], true);
        assert_eq!(body["dataVersion"], "data");

        let stream = post_game_stream(
            State(state.clone()),
            auth_headers(),
            Json(GameStreamRequest {
                server: "jp".to_string(),
                path: "/stream".to_string(),
            }),
        )
        .await;
        assert_eq!(stream.status(), StatusCode::OK);
        assert_eq!(stream.headers()["x-haruki-game-status"], "200");
        assert!(!to_bytes(stream.into_body(), usize::MAX)
            .await
            .unwrap()
            .is_empty());

        let image = post_sekai_image(
            State(state),
            auth_headers(),
            Json(InternalImageRequest {
                server: "jp".to_string(),
                kind: ImageKind::CpMusicScore,
                param1: "a".to_string(),
                param2: "b".to_string(),
            }),
        )
        .await;
        assert_eq!(image.status(), StatusCode::OK);
        assert_eq!(image.headers()["content-type"], "application/octet-stream");
        assert_eq!(
            to_bytes(image.into_body(), usize::MAX).await.unwrap(),
            vec![1, 2, 3]
        );
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn serves_version_and_reports_version_failures() {
        let root = temp_dir();
        let state = state(&root, false).await;
        let response = get_master_version(
            State(state.clone()),
            auth_headers(),
            Path("bad".to_string()),
        )
        .await;
        assert_eq!(json_body(response).await["kind"], "invalid_server_region");
        let response =
            get_master_version(State(state.clone()), auth_headers(), Path("tw".to_string())).await;
        assert_eq!(json_body(response).await["kind"], "not_found");

        let response =
            get_master_version(State(state.clone()), auth_headers(), Path("jp".to_string())).await;
        assert_eq!(json_body(response).await["kind"], "io");
        std::fs::write(root.join("version.json"), "invalid").unwrap();
        let response =
            get_master_version(State(state.clone()), auth_headers(), Path("jp".to_string())).await;
        assert_eq!(json_body(response).await["kind"], "parse");
        std::fs::write(
            root.join("version.json"),
            r#"{"appVersion":"1","appHash":"h","dataVersion":"d","assetVersion":"a"}"#,
        )
        .unwrap();
        let response =
            get_master_version(State(state), auth_headers(), Path("jp".to_string())).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json_body(response).await["appVersion"], "1");
        std::fs::remove_dir_all(root).unwrap();
    }

    fn sha256_hex(data: &[u8]) -> String {
        use sha2::Digest as _;
        hex::encode(sha2::Sha256::digest(data))
    }

    #[tokio::test]
    async fn serves_master_manifest_with_cached_digests() {
        let root = temp_dir();
        let state = state(&root, false).await;
        // No version file yet: manifest still lists files with empty identity.
        std::fs::write(root.join("master/cards.json"), "[1]").unwrap();
        std::fs::write(root.join("master/.tmp.json"), "[]").unwrap();
        std::fs::write(root.join("master/note.txt"), "x").unwrap();
        let response =
            get_master_manifest(State(state.clone()), auth_headers(), Path("jp".to_string())).await;
        assert_eq!(json_body(response).await["kind"], "io");

        std::fs::write(
            root.join("version.json"),
            r#"{"appVersion":"5.6.0","appHash":"h","dataVersion":"5.6.1.11","assetVersion":"5.6.1.10","cdnVersion":0}"#,
        )
        .unwrap();
        let response =
            get_master_manifest(State(state.clone()), auth_headers(), Path("jp".to_string())).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert_eq!(body["server"], "jp");
        assert_eq!(body["appVersion"], "5.6.0");
        assert_eq!(body["dataVersion"], "5.6.1.11");
        assert_eq!(body["files"].as_array().unwrap().len(), 1);
        assert_eq!(body["files"][0]["name"], "cards.json");
        assert_eq!(body["files"][0]["size"], 3);
        assert_eq!(body["files"][0]["sha256"], sha256_hex(b"[1]"));
        assert_eq!(body["contentHash"].as_str().unwrap().len(), 64);
        assert!(
            body.get("gitCommit").is_none(),
            "temp dir is not a git repo"
        );
        // Inside a repository the manifest pins HEAD.
        let repo = root.join("master");
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {:?}: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "add",
            "-A",
        ]);
        git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "x",
        ]);
        let response =
            get_master_manifest(State(state.clone()), auth_headers(), Path("jp".to_string())).await;
        assert_eq!(
            json_body(response).await["gitCommit"]
                .as_str()
                .unwrap()
                .len(),
            40
        );

        // A rewritten file (new size) is re-hashed; an untouched one is served
        // from the cache.
        std::fs::write(root.join("master/cards.json"), "[1,2]").unwrap();
        let cards = root.join("master/cards.json");
        let meta = std::fs::metadata(&cards).unwrap();
        let fresh = file_sha256(&cards, &meta).unwrap();
        assert_eq!(fresh, sha256_hex(b"[1,2]"));
        assert_eq!(file_sha256(&cards, &meta).unwrap(), fresh);

        let missing =
            get_master_manifest(State(state), auth_headers(), Path("kr".to_string())).await;
        assert_eq!(json_body(missing).await["kind"], "not_found");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn builds_and_streams_master_tar() {
        let root = temp_dir();
        let state = state(&root, false).await;
        let out = root.join("direct.tar");
        assert!(matches!(
            build_master_tar(
                root.join("master").to_str().unwrap(),
                root.join("version.json").to_str().unwrap(),
                &out
            ),
            Err(AppError::NotFound(_))
        ));

        std::fs::write(root.join("master/data.json"), "[]").unwrap();
        std::fs::write(root.join("master/.hidden.json"), "[]").unwrap();
        std::fs::write(root.join("master/note.txt"), "ignored").unwrap();
        std::fs::write(root.join("version.json"), "{}").unwrap();
        build_master_tar(
            root.join("master").to_str().unwrap(),
            root.join("version.json").to_str().unwrap(),
            &out,
        )
        .unwrap();
        assert!(out.metadata().unwrap().len() > 0);

        let response =
            get_master_bundle(State(state), auth_headers(), Path("jp".to_string())).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "application/x-tar");
        assert!(!to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn master_updated_validates_region_and_sync_configuration() {
        let root = temp_dir();
        let state = state(&root, false).await;
        let invalid = post_master_updated(
            State(state.clone()),
            auth_headers(),
            Json(MasterUpdatedNotice {
                server: "bad".to_string(),
                data_version: String::new(),
            }),
        )
        .await;
        assert_eq!(json_body(invalid).await["kind"], "invalid_server_region");
        let missing = post_master_updated(
            State(state),
            auth_headers(),
            Json(MasterUpdatedNotice {
                server: "jp".to_string(),
                data_version: "1".to_string(),
            }),
        )
        .await;
        assert_eq!(json_body(missing).await["kind"], "not_found");
        std::fs::remove_dir_all(root).unwrap();
    }
}
