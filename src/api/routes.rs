use std::sync::Arc;
use std::time::Instant;

use axum::{middleware, routing::get, Json, Router};
use serde::Serialize;
use tower_http::trace::TraceLayer;

use crate::AppState as MainAppState;

use super::api;
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

pub fn create_router(state: Arc<MainAppState>) -> Router {
    START_TIME.get_or_init(Instant::now);

    let public_routes = Router::new()
        .route("/health", get(health_check))
        .route(
            "/image/{server}/mysekai/{param1}/{param2}",
            get(image::get_mysekai_image),
        );

    let api_routes = Router::new()
        .route("/{server}/{user_id}/profile", get(api::get_user_profile))
        .route("/{server}/system", get(api::get_system))
        .route("/{server}/information", get(api::get_information))
        .route(
            "/{server}/event/{event_id}/ranking-top100",
            get(api::get_event_ranking_top100),
        )
        .route(
            "/{server}/event/{event_id}/ranking-border",
            get(api::get_event_ranking_border),
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
