use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use regex::Regex;

use crate::config::ServerRegion;
use crate::error::AppError;
use crate::upstream::ImageKind;
use crate::AppState;

/// Map an upstream image-fetch error to a client response: a 404 from the game
/// server means the image does not exist and is propagated as 404; everything
/// else uses the error's own status mapping (upstream faults surface as 502).
/// Always emits the standard JSON error body.
fn image_error_response(context: &str, e: AppError) -> Response {
    let (status, message) = match e {
        AppError::Unknown { status: 404, .. } => {
            (StatusCode::NOT_FOUND, format!("{}: not found", context))
        }
        other => (other.status_code(), format!("{}: {}", context, other)),
    };
    let body = crate::error::ApiErrorResponse {
        result: "failed",
        status: status.as_u16(),
        message,
    };
    match sonic_rs::to_string(&body) {
        Ok(json) => (status, [("content-type", "application/json")], json).into_response(),
        // Keep the HTTP status consistent with the fallback body's status field.
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [("content-type", "application/json")],
            r#"{"result":"failed","status":500,"message":"Internal error"}"#.to_string(),
        )
            .into_response(),
    }
}

pub async fn get_mysekai_image(
    State(state): State<std::sync::Arc<AppState>>,
    Path((server, param1, param2)): Path<(String, String, String)>,
) -> Response {
    let region: ServerRegion = match server.parse() {
        Ok(r) => r,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Invalid server: {}", server),
            )
                .into_response();
        }
    };
    let Some(router) = state.routers.get(&region) else {
        return (StatusCode::SERVICE_UNAVAILABLE, "Server not initialized").into_response();
    };
    static HEX64: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static DIGITS: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let hex64 = HEX64.get_or_init(|| Regex::new(r"^[a-f0-9]{64}$").unwrap());
    let digits = DIGITS.get_or_init(|| Regex::new(r"^\d+$").unwrap());
    let image_result = if region.is_cp_server() {
        if !hex64.is_match(&param1) || !hex64.is_match(&param2) {
            return (
                StatusCode::BAD_REQUEST,
                "Invalid path format for colorful palette servers (expected 64-char hex)",
            )
                .into_response();
        }
        router
            .get_image(ImageKind::CpMysekai, &param1, &param2)
            .await
    } else {
        if !digits.is_match(&param1) || !digits.is_match(&param2) {
            return (
                StatusCode::BAD_REQUEST,
                "Invalid path format for nuverse servers (expected numeric user_id and index)",
            )
                .into_response();
        }
        router
            .get_image(ImageKind::NuverseMysekai, &param1, &param2)
            .await
    };
    match image_result {
        Ok(bytes) => (StatusCode::OK, [("content-type", "image/png")], bytes).into_response(),
        Err(e) => image_error_response("Fetch image failed", e),
    }
}

pub async fn get_mysekai_housing_thumbnail(
    State(state): State<std::sync::Arc<AppState>>,
    Path((server, param1, param2)): Path<(String, String, String)>,
) -> Response {
    let region: ServerRegion = match server.parse() {
        Ok(r) => r,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Invalid server: {}", server),
            )
                .into_response();
        }
    };
    if !region.is_cp_server() {
        return (
            StatusCode::BAD_REQUEST,
            "MySekai housing thumbnails are only supported for colorful palette servers",
        )
            .into_response();
    }
    let Some(router) = state.routers.get(&region) else {
        return (StatusCode::SERVICE_UNAVAILABLE, "Server not initialized").into_response();
    };
    static HEX64: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static UUID36: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let hex64 = HEX64.get_or_init(|| Regex::new(r"^[a-f0-9]{64}$").unwrap());
    let uuid36 = UUID36.get_or_init(|| {
        Regex::new(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$")
            .unwrap()
    });
    if !hex64.is_match(&param1) || !uuid36.is_match(&param2) {
        return (
            StatusCode::BAD_REQUEST,
            "Invalid path format for MySekai housing thumbnail",
        )
            .into_response();
    }
    match router
        .get_image(ImageKind::CpHousingThumbnail, &param1, &param2)
        .await
    {
        Ok(bytes) => (StatusCode::OK, [("content-type", "image/png")], bytes).into_response(),
        Err(e) => image_error_response("Fetch MySekai housing thumbnail failed", e),
    }
}

pub async fn get_custom_profile_card_thumbnail(
    State(state): State<std::sync::Arc<AppState>>,
    Path((server, param1, param2)): Path<(String, String, String)>,
) -> Response {
    let region: ServerRegion = match server.parse() {
        Ok(r) => r,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Invalid server: {}", server),
            )
                .into_response();
        }
    };
    if !region.is_cp_server() {
        return (
            StatusCode::BAD_REQUEST,
            "Custom profile card thumbnails are only supported for colorful palette servers",
        )
            .into_response();
    }
    let Some(router) = state.routers.get(&region) else {
        return (StatusCode::SERVICE_UNAVAILABLE, "Server not initialized").into_response();
    };
    static HEX64: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static UUID36: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let hex64 = HEX64.get_or_init(|| Regex::new(r"^[a-f0-9]{64}$").unwrap());
    let uuid36 = UUID36.get_or_init(|| {
        Regex::new(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$")
            .unwrap()
    });
    if !hex64.is_match(&param1) || !uuid36.is_match(&param2) {
        return (
            StatusCode::BAD_REQUEST,
            "Invalid path format for custom profile card thumbnail",
        )
            .into_response();
    }
    match router
        .get_image(ImageKind::CpProfileCardThumbnail, &param1, &param2)
        .await
    {
        Ok(bytes) => (StatusCode::OK, [("content-type", "image/png")], bytes).into_response(),
        Err(e) => image_error_response("Fetch image failed", e),
    }
}

pub async fn get_custom_music_score(
    State(state): State<std::sync::Arc<AppState>>,
    Path((server, param1, param2)): Path<(String, String, String)>,
) -> Response {
    let region: ServerRegion = match server.parse() {
        Ok(r) => r,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Invalid server: {}", server),
            )
                .into_response();
        }
    };
    if !region.is_cp_server() {
        return (
            StatusCode::BAD_REQUEST,
            "Custom music scores are only supported for colorful palette servers",
        )
            .into_response();
    }
    let Some(router) = state.routers.get(&region) else {
        return (StatusCode::SERVICE_UNAVAILABLE, "Server not initialized").into_response();
    };
    static HEX64: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let hex64 = HEX64.get_or_init(|| Regex::new(r"^[a-f0-9]{64}$").unwrap());
    if !hex64.is_match(&param1) || !hex64.is_match(&param2) {
        return (
            StatusCode::BAD_REQUEST,
            "Invalid path format for custom music score",
        )
            .into_response();
    }
    match router
        .get_image(ImageKind::CpMusicScore, &param1, &param2)
        .await
    {
        Ok(bytes) => (
            StatusCode::OK,
            [("content-type", "application/octet-stream")],
            bytes,
        )
            .into_response(),
        Err(e) => image_error_response("Fetch custom music score failed", e),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::{Path, State};
    use axum::http::Response as AxumResponse;
    use axum::Router;

    use super::*;
    use crate::config::{Config, UpstreamConfig};
    use crate::upstream::{build_internal_http_client, InternalApiResponse, RegionRouter};
    use crate::RequestCoalescer;

    #[derive(Clone)]
    struct Reply {
        content_type: &'static str,
        body: Vec<u8>,
    }

    async fn handler(State(reply): State<Reply>) -> AxumResponse<Body> {
        AxumResponse::builder()
            .header("content-type", reply.content_type)
            .body(Body::from(reply.body))
            .unwrap()
    }

    async fn test_state(
        reply: Reply,
        region: ServerRegion,
    ) -> (Arc<AppState>, tokio::task::JoinHandle<()>) {
        let app = Router::new().fallback(handler).with_state(reply);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let upstream = UpstreamConfig {
            url: format!("http://{address}"),
            token: String::new(),
            priority: 0,
            name: "image".to_string(),
        };
        let router = RegionRouter::new(
            region,
            None,
            &[upstream],
            build_internal_http_client().unwrap(),
        )
        .unwrap();
        let config: Config = serde_yaml::from_str("backend: {}").unwrap();
        let state = AppState {
            config,
            clients: HashMap::new(),
            routers: HashMap::from([(region, Arc::new(router))]),
            syncers: HashMap::new(),
            version_locks: HashMap::new(),
            db: None,
            master_db: None,
            redis: None,
            jwt_secret: None,
            coalescer: Arc::new(RequestCoalescer::default()),
        };
        (Arc::new(state), server)
    }

    #[tokio::test]
    async fn validates_regions_paths_and_feature_support() {
        let (state, server) = test_state(
            Reply {
                content_type: "image/png",
                body: vec![1],
            },
            ServerRegion::Jp,
        )
        .await;
        let invalid = get_mysekai_image(
            State(state.clone()),
            Path(("bad".to_string(), "a".to_string(), "b".to_string())),
        )
        .await;
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
        let missing = get_mysekai_image(
            State(state.clone()),
            Path(("en".to_string(), "a".to_string(), "b".to_string())),
        )
        .await;
        assert_eq!(missing.status(), StatusCode::SERVICE_UNAVAILABLE);
        let malformed = get_mysekai_image(
            State(state.clone()),
            Path(("jp".to_string(), "a".to_string(), "b".to_string())),
        )
        .await;
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);

        for response in [
            get_mysekai_housing_thumbnail(
                State(state.clone()),
                Path(("tw".to_string(), "a".to_string(), "b".to_string())),
            )
            .await,
            get_custom_profile_card_thumbnail(
                State(state.clone()),
                Path(("tw".to_string(), "a".to_string(), "b".to_string())),
            )
            .await,
            get_custom_music_score(
                State(state.clone()),
                Path(("tw".to_string(), "a".to_string(), "b".to_string())),
            )
            .await,
        ] {
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
        let malformed_housing = get_mysekai_housing_thumbnail(
            State(state.clone()),
            Path(("jp".to_string(), "a".to_string(), "b".to_string())),
        )
        .await;
        assert_eq!(malformed_housing.status(), StatusCode::BAD_REQUEST);
        let malformed_profile = get_custom_profile_card_thumbnail(
            State(state.clone()),
            Path(("jp".to_string(), "a".to_string(), "b".to_string())),
        )
        .await;
        assert_eq!(malformed_profile.status(), StatusCode::BAD_REQUEST);
        let malformed_score = get_custom_music_score(
            State(state.clone()),
            Path(("jp".to_string(), "a".to_string(), "b".to_string())),
        )
        .await;
        assert_eq!(malformed_score.status(), StatusCode::BAD_REQUEST);
        server.abort();
    }

    #[tokio::test]
    async fn returns_successful_images_for_every_route() {
        let (state, server) = test_state(
            Reply {
                content_type: "image/png",
                body: vec![1, 2, 3],
            },
            ServerRegion::Jp,
        )
        .await;
        let hex = "a".repeat(64);
        let uuid = "12345678-1234-1234-1234-123456789abc".to_string();
        for response in [
            get_mysekai_image(
                State(state.clone()),
                Path(("jp".to_string(), hex.clone(), hex.clone())),
            )
            .await,
            get_mysekai_housing_thumbnail(
                State(state.clone()),
                Path(("jp".to_string(), hex.clone(), uuid.clone())),
            )
            .await,
            get_custom_profile_card_thumbnail(
                State(state.clone()),
                Path(("jp".to_string(), hex.clone(), uuid)),
            )
            .await,
            get_custom_music_score(
                State(state.clone()),
                Path(("jp".to_string(), hex.clone(), hex)),
            )
            .await,
        ] {
            assert_eq!(response.status(), StatusCode::OK);
        }
        server.abort();

        let (tw_state, tw_server) = test_state(
            Reply {
                content_type: "image/png",
                body: vec![4],
            },
            ServerRegion::Tw,
        )
        .await;
        let response = get_mysekai_image(
            State(tw_state),
            Path(("tw".to_string(), "123".to_string(), "4".to_string())),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        tw_server.abort();
    }

    #[tokio::test]
    async fn maps_remote_image_errors_to_json_statuses() {
        let envelope = InternalApiResponse {
            ok: false,
            status: Some(404),
            data: None,
            kind: Some("unknown".to_string()),
            message: Some("missing".to_string()),
        };
        let (state, server) = test_state(
            Reply {
                content_type: "application/json",
                body: serde_json::to_vec(&envelope).unwrap(),
            },
            ServerRegion::Jp,
        )
        .await;
        let hex = "b".repeat(64);
        let response =
            get_mysekai_image(State(state), Path(("jp".to_string(), hex.clone(), hex))).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        server.abort();

        let response = image_error_response("context", AppError::NetworkError("down".to_string()));
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }
}
