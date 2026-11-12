use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use axum::{
    extract::State,
    middleware,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use tower_http::compression::predicate::{NotForContentType, Predicate};
use tower_http::compression::{CompressionLayer, DefaultPredicate};
use tower_http::trace::TraceLayer;

use crate::AppState as MainAppState;

use super::apis;
use super::image;
use super::internal;
use super::middleware::auth_middleware;

static START_TIME: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Whether a local region can serve game calls right now.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionHealth {
    pub sessions: usize,
    /// Active egress route and whether it is the primary one.
    pub egress: String,
    pub primary: bool,
    /// Logins are being refused locally (see `client::egress`).
    pub gated: bool,
    /// Unix ms of the last successful game call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    /// `degraded` when a local region has no sessions or its login gate is
    /// closed; the process being up is not the same as regions serving.
    pub status: &'static str,
    pub version: &'static str,
    pub uptime_secs: u64,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub regions: BTreeMap<&'static str, RegionHealth>,
}

pub async fn health_check(State(state): State<Arc<MainAppState>>) -> Json<HealthResponse> {
    let start = START_TIME.get_or_init(Instant::now);
    let uptime = start.elapsed().as_secs();
    let mut regions: BTreeMap<&'static str, RegionHealth> = state
        .clients
        .iter()
        .map(|(region, client)| {
            let egress = client.egress.status();
            (
                region.as_str(),
                RegionHealth {
                    sessions: client.session_count(),
                    egress: egress.active,
                    primary: egress.primary,
                    gated: egress.gated,
                    last_success_at: egress.last_success_at,
                },
            )
        })
        .collect();
    // A region that is enabled but whose client failed to initialise is not
    // in `clients` at all. It cannot serve, so it must not vanish from here.
    for (region, server) in &state.config.servers {
        if server.enabled && !state.clients.contains_key(region) {
            regions.insert(
                region.as_str(),
                RegionHealth {
                    sessions: 0,
                    egress: String::new(),
                    primary: false,
                    gated: false,
                    last_success_at: None,
                },
            );
        }
    }
    let serving = regions
        .values()
        .all(|region| region.sessions > 0 && !region.gated);
    Json(HealthResponse {
        status: if serving { "ok" } else { "degraded" },
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: uptime,
        regions,
    })
}

/// The default predicate plus a content-type exclusion for octet streams:
/// those bodies are AES ciphertext (the `/internal/game-stream` relay of a
/// whole master split, custom music scores), so compressing them only burns
/// CPU on the account node for no size gain.
fn compression_predicate() -> impl Predicate {
    DefaultPredicate::new().and(NotForContentType::const_new("application/octet-stream"))
}

pub fn create_router(state: Arc<MainAppState>) -> Router {
    START_TIME.get_or_init(Instant::now);

    let public_routes = Router::new()
        .route("/health", get(health_check))
        .route(
            "/image/{server}/mysekai-housing/{param1}/{param2}",
            get(image::get_mysekai_housing_thumbnail),
        )
        .route(
            "/image/{server}/custom-profile-card/thumbnail/{param1}/{param2}",
            get(image::get_custom_profile_card_thumbnail),
        )
        .route(
            "/image/{server}/blob/custom-music-score/full/{param1}/{param2}",
            get(image::get_custom_music_score),
        );

    // The mysekai image route is authenticated: unlike the other image routes
    // (CP-only, gated by unguessable 64-hex hashes), its Nuverse branch takes a
    // guessable numeric user_id/index and drives an authenticated game API call
    // with the shared bot account, so it must not be publicly reachable.
    let authed_image_routes = Router::new()
        .route(
            "/image/{server}/mysekai/{param1}/{param2}",
            get(image::get_mysekai_image),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    let api_routes = Router::new()
        .route("/{server}/{user_id}/profile", get(apis::get_user_profile))
        .route("/{server}/system", get(apis::get_system))
        .route("/{server}/information", get(apis::get_information))
        .route(
            "/{server}/user/mysekai/housing-competition/{housing_id}/list",
            get(apis::get_mysekai_housing_competition_list),
        )
        .route(
            "/{server}/user/mysekai/housing-competition/{housing_id}/mysekai-owner/{owner_user_id}/entry",
            post(apis::post_mysekai_housing_competition_entry),
        )
        .route(
            "/{server}/mysekai/housing-competition/back-number-top-list",
            get(apis::get_mysekai_housing_competition_back_number_top_list),
        )
        .route(
            "/{server}/mysekai/housing-competition/{competition_id}/back-number-list",
            get(apis::get_mysekai_housing_competition_back_number_list),
        )
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

    // Node-to-node endpoints; guarded inside the handlers by
    // backend.internal_token (404 when unconfigured), not by user auth.
    let internal_routes = Router::new()
        .route("/internal/sekai-api", post(internal::post_sekai_api))
        .route("/internal/sekai-image", post(internal::post_sekai_image))
        .route("/internal/login-probe", post(internal::post_login_probe))
        .route("/internal/game-stream", post(internal::post_game_stream))
        .route("/internal/app-identity", post(internal::post_app_identity))
        .route("/internal/accounts", post(internal::post_accounts))
        .route(
            "/internal/egress",
            get(internal::get_egress).put(internal::put_egress),
        )
        .route(
            "/internal/master/{server}/version",
            get(internal::get_master_version),
        )
        .route(
            "/internal/master/{server}/bundle",
            get(internal::get_master_bundle),
        )
        .route(
            "/internal/master/{server}/manifest",
            get(internal::get_master_manifest),
        )
        .route(
            "/internal/master-updated",
            post(internal::post_master_updated),
        );

    Router::new()
        .merge(public_routes)
        .merge(authed_image_routes)
        .merge(internal_routes)
        .nest("/api", api_routes)
        // Compress responses when the client sends Accept-Encoding — both end
        // users and peer nodes forwarding over WAN links benefit; the default
        // predicate already skips small and incompressible (image) bodies.
        .layer(CompressionLayer::new().compress_when(compression_predicate()))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use super::*;
    use crate::{AppState, RequestCoalescer};

    fn empty_state() -> Arc<AppState> {
        Arc::new(AppState {
            config: serde_yaml::from_str("backend: {}").unwrap(),
            clients: HashMap::new(),
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

    #[test]
    fn compression_skips_octet_streams_but_not_json() {
        use axum::http::{header, HeaderValue, Response};
        use tower_http::compression::predicate::Predicate;

        let predicate = compression_predicate();
        // Bodies above the default 32-byte size threshold, so only the
        // content type decides.
        let response = |content_type: &'static str| {
            let mut response = Response::new(axum::body::Body::from(vec![b'x'; 1024]));
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
            response
        };
        assert!(predicate.should_compress(&response("application/json")));
        assert!(!predicate.should_compress(&response("application/octet-stream")));
        assert!(!predicate.should_compress(&response("image/png")));
    }

    // The process being up must not read as healthy while a region cannot
    // serve: no sessions, or logins refused behind the gate.
    #[tokio::test]
    async fn health_degrades_when_a_region_cannot_serve() {
        let root = std::env::temp_dir().join(format!("haruki_health_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut server: crate::config::ServerConfig = serde_yaml::from_str("{}").unwrap();
        server.aes_key_hex = "00112233445566778899aabbccddeeff".to_string();
        server.aes_iv_hex = "ffeeddccbbaa99887766554433221100".to_string();
        server.version_path = root.join("version.json").to_string_lossy().into_owned();
        let client = crate::client::SekaiClient::new(
            crate::config::ServerRegion::Cn,
            server,
            None,
            None,
            crate::client::SekaiClient::build_http_client(None).unwrap(),
            None,
        )
        .await
        .unwrap();
        let mut state = Arc::try_unwrap(empty_state()).ok().unwrap();
        state
            .clients
            .insert(crate::config::ServerRegion::Cn, Arc::new(client));
        // An enabled region whose client never initialised still shows up.
        state.config =
            serde_yaml::from_str("backend: {}\nservers:\n  jp: {enabled: true}\n  kr: {}").unwrap();
        let Json(health) = health_check(State(Arc::new(state))).await;
        assert_eq!(health.status, "degraded");
        assert_eq!(health.regions["jp"].sessions, 0);
        assert!(
            !health.regions.contains_key("kr"),
            "disabled regions are not local"
        );
        let cn = &health.regions["cn"];
        assert_eq!((cn.sessions, cn.egress.as_str()), (0, "default"));
        assert!(cn.primary && !cn.gated);
        let json = serde_json::to_value(&health).unwrap();
        assert_eq!(json["regions"]["cn"]["primary"], true);
        assert!(json["regions"]["cn"].get("lastSuccessAt").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn health_reports_version_and_router_builds_all_layers() {
        let Json(health) = health_check(State(empty_state())).await;
        assert_eq!(health.status, "ok");
        assert_eq!(health.version, env!("CARGO_PKG_VERSION"));
        assert!(health.regions.is_empty());
        let _router = create_router(empty_state());
        let Json(second) = health_check(State(empty_state())).await;
        assert!(second.uptime_secs >= health.uptime_secs);
    }
}
