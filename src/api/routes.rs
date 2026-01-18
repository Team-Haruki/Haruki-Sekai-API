use std::sync::Arc;

use axum::{middleware, routing::get, Router};
use tower_http::trace::TraceLayer;

use crate::AppState as MainAppState;

use super::api;
use super::image;
use super::middleware::auth_middleware;

pub fn create_router(state: Arc<MainAppState>) -> Router {
    let public_routes = Router::new()
        .route(
            "/image/{server}/mysekai/{param1}/{param2}",
            get(image::get_mysekai_image),
        );

    let api_routes = Router::new()
        .route(
            "/{server}/{user_id}/profile",
            get(api::get_user_profile),
        )
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
