//! Ingester HTTP surface:
//! - `GET /health`                                    last outcome per target and region
//! - `POST /internal/master-updated`                  the registry's publish webhook
//!   (bearer `ingest.webhook_token`): reconcile that region
//! - `POST /v1/ingest/{target}/{region}/allow-shrink` `{"contentHash","tables"}`
//!   (same token): allow those tables of that version to shrink past `min_ratio`
//!
//! With an empty `webhook_token` both POST endpoints answer 404.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use tracing::{info, warn};

use super::run::TargetOutcome;
use super::Ingester;
use crate::config::ServerRegion;

type Shared = Arc<Ingester>;

pub fn router(ingester: Shared) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/internal/master-updated", post(master_updated))
        .route(
            "/v1/ingest/{target}/{region}/allow-shrink",
            post(allow_shrink),
        )
        .with_state(ingester)
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(serde_json::json!({"result": "failed", "status": status.as_u16(), "message": message.into()})),
    )
        .into_response()
}

fn check_token(ingester: &Ingester, headers: &HeaderMap) -> Result<(), Box<Response>> {
    let expected = &ingester.config.webhook_token;
    if expected.is_empty() {
        return Err(Box::new(StatusCode::NOT_FOUND.into_response()));
    }
    let presented = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if !presented.is_some_and(|p| token_matches(p, expected)) {
        return Err(Box::new(StatusCode::UNAUTHORIZED.into_response()));
    }
    Ok(())
}

/// Constant-time comparison: both sides are hashed first, so neither the
/// content nor the length of the token leaks through timing.
fn token_matches(presented: &str, expected: &str) -> bool {
    use sha2::Digest as _;
    let a = sha2::Sha256::digest(presented.as_bytes());
    let b = sha2::Sha256::digest(expected.as_bytes());
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

async fn health(State(ingester): State<Shared>) -> Response {
    let status = ingester.status();
    let failed = status.values().flat_map(|r| r.values()).any(|e| {
        matches!(
            e.outcome,
            TargetOutcome::Failed { .. } | TargetOutcome::Skipped { .. }
        )
    });
    Json(serde_json::json!({
        "status": if failed { "degraded" } else { "ok" },
        "version": env!("CARGO_PKG_VERSION"),
        "registry": ingester.registry.base(),
        "targets": status,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct Notice {
    server: String,
}

async fn master_updated(
    State(ingester): State<Shared>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Err(resp) = check_token(&ingester, &headers) {
        return *resp;
    }
    // Only `server` is read: the notice triggers a reconcile, which reads
    // the registry's `current` itself.
    let notice: Notice = match serde_json::from_slice(&body) {
        Ok(n) => n,
        Err(e) => return error(StatusCode::BAD_REQUEST, format!("invalid body: {e}")),
    };
    let Ok(region) = notice.server.parse::<ServerRegion>() else {
        return error(
            StatusCode::BAD_REQUEST,
            format!("unknown region {:?}", notice.server),
        );
    };
    if !ingester.trigger(region) {
        return error(
            StatusCode::NOT_FOUND,
            format!("no target ingests region {}", region.as_str()),
        );
    }
    info!(
        "{} Publish webhook received; reconciling",
        region.as_str().to_uppercase()
    );
    Json(serde_json::json!({"triggered": true})).into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AllowShrink {
    content_hash: String,
    tables: Vec<String>,
}

async fn allow_shrink(
    State(ingester): State<Shared>,
    Path((target, region)): Path<(String, String)>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Err(resp) = check_token(&ingester, &headers) {
        return *resp;
    }
    let Some(t) = ingester.target(&target).cloned() else {
        return error(StatusCode::NOT_FOUND, format!("unknown target {target:?}"));
    };
    let Ok(region) = region.parse::<ServerRegion>() else {
        return error(
            StatusCode::BAD_REQUEST,
            format!("unknown region {region:?}"),
        );
    };
    if !t.serves(region) {
        return error(
            StatusCode::NOT_FOUND,
            format!("target {target} does not ingest {}", region.as_str()),
        );
    }
    let req: AllowShrink = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return error(StatusCode::BAD_REQUEST, format!("invalid body: {e}")),
    };
    if !crate::registry::state::is_hex_digest(&req.content_hash) || req.tables.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "contentHash must be a hex digest and tables non-empty",
        );
    }
    let mut tables = Vec::new();
    for name in &req.tables {
        match t.resolve(name) {
            Some(table) => tables.push(table),
            None => return error(StatusCode::BAD_REQUEST, format!("unknown table {name:?}")),
        }
    }
    if let Err(e) = t
        .store_allow_shrink(region, &req.content_hash, &tables)
        .await
    {
        warn!("allow-shrink for {target}: {e:#}");
        return error(StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}"));
    }
    info!(
        "{} Target {}: allow-shrink stored for contentHash {}: {:?}",
        region.as_str().to_uppercase(),
        target,
        req.content_hash,
        tables
    );
    ingester.trigger(region);
    Json(serde_json::json!({"stored": true, "tables": tables})).into_response()
}
