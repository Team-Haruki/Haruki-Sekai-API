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
use crate::upstream::{InternalApiRequest, InternalApiResponse};
use crate::AppState;

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
fn build_master_tar(
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
