use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{middleware, routing::get, Json, Router};
use serde::Serialize;
use tower_http::trace::TraceLayer;

use crate::AppState as MainAppState;

use super::apis;
use super::image;
use super::middleware::auth_middleware;

static START_TIME: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub uptime_secs: u64,
}

pub async fn health_check() -> Json<HealthResponse> {
    let start = START_TIME.get_or_init(Instant::now);
    let uptime = start.elapsed().as_secs();
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: uptime,
    })
}

/// Liveness probe: process is up and the async runtime can serve requests.
/// Always 200 — never gates on external dependencies, otherwise a transient
/// Redis/DB outage would cause Kubernetes to kill (and not just unready) pods.
pub async fn liveness() -> &'static str {
    "ok"
}

#[derive(Debug, Serialize)]
struct ReadinessResponse {
    status: &'static str,
    database: ComponentStatus,
    master_database: ComponentStatus,
    redis: ComponentStatus,
}

#[derive(Debug, Serialize)]
struct ComponentStatus {
    enabled: bool,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl ComponentStatus {
    fn disabled() -> Self {
        Self { enabled: false, ok: true, error: None }
    }
    fn ok() -> Self {
        Self { enabled: true, ok: true, error: None }
    }
    fn fail(e: impl ToString) -> Self {
        Self { enabled: true, ok: false, error: Some(e.to_string()) }
    }
}

/// Readiness probe: every enabled backing service must be reachable.
/// Returns 200 when all green, 503 otherwise. Probes that ping a disabled
/// dependency are treated as ok=true so an empty/local config stays ready.
pub async fn readiness(State(state): State<Arc<MainAppState>>) -> impl IntoResponse {
    let database = match &state.db {
        None => ComponentStatus::disabled(),
        Some(db) => match db.ping().await {
            Ok(()) => ComponentStatus::ok(),
            Err(e) => ComponentStatus::fail(e),
        },
    };

    let master_database = match &state.master_db {
        None => ComponentStatus::disabled(),
        Some(db) => match db.ping().await {
            Ok(()) => ComponentStatus::ok(),
            Err(e) => ComponentStatus::fail(e),
        },
    };

    let redis = match &state.redis {
        None => ComponentStatus::disabled(),
        Some(mgr) => {
            let mut conn = mgr.clone();
            match redis::cmd("PING").query_async::<String>(&mut conn).await {
                Ok(_) => ComponentStatus::ok(),
                Err(e) => ComponentStatus::fail(e),
            }
        }
    };

    let all_ok = database.ok && master_database.ok && redis.ok;
    let body = ReadinessResponse {
        status: if all_ok { "ready" } else { "not_ready" },
        database,
        master_database,
        redis,
    };
    let code = if all_ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (code, Json(body))
}

pub fn create_router(state: Arc<MainAppState>) -> Router {
    START_TIME.get_or_init(Instant::now);

    // /healthz and /readyz are the standard Kubernetes probe endpoints; /health
    // is preserved for back-compat with existing operators.
    // Note: tower_http TraceLayer logs at DEBUG by default, so probe traffic
    // does not pollute INFO-level logs. The legacy `access_log` /
    // `access_log_path` config fields are currently not wired up — all logging
    // goes through `tracing` to stdout regardless of those values.
    let public_routes = Router::new()
        .route("/health", get(health_check))
        .route("/healthz", get(liveness))
        .route("/readyz", get(readiness))
        .route(
            "/image/{server}/mysekai/{param1}/{param2}",
            get(image::get_mysekai_image),
        )
        .route(
            "/image/{server}/custom-profile-card/thumbnail/{param1}/{param2}",
            get(image::get_custom_profile_card_thumbnail),
        )
        .route(
            "/image/{server}/blob/custom-music-score/full/{param1}/{param2}",
            get(image::get_custom_music_score),
        );

    let api_routes = Router::new()
        .route("/{server}/{user_id}/profile", get(apis::get_user_profile))
        .route("/{server}/system", get(apis::get_system))
        .route("/{server}/information", get(apis::get_information))
        .route(
            "/{server}/user/{user_id}/custom-music-score/published/search/{score_id}",
            get(apis::get_custom_music_score_published_search),
        )
        .route(
            "/{server}/event/{event_id}/ranking-top100",
            get(apis::get_event_ranking_top100),
        )
        .route(
            "/{server}/event/{event_id}/ranking-border",
            get(apis::get_event_ranking_border),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    Router::new()
        .merge(public_routes)
        .nest("/api", api_routes)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
