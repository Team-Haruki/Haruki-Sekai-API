use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use crate::config::ServerRegion;
use crate::error::AppError;
use crate::AppState;

pub struct ApiResponse {
    status: StatusCode,
    body: JsonValue,
}

#[derive(Debug, Deserialize)]
pub struct MySekaiHousingCompetitionListQuery {
    #[serde(rename = "isLottery", default)]
    pub is_lottery: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MySekaiHousingCompetitionEntryQuery {
    #[serde(rename = "isBackNumber", default)]
    pub is_back_number: Option<String>,
    #[serde(rename = "mysekaiOwnerUserSubmittedAt")]
    pub mysekai_owner_user_submitted_at: i64,
}

impl IntoResponse for ApiResponse {
    fn into_response(self) -> Response {
        let json = sonic_rs::to_string(&self.body).unwrap_or_else(|_| "{}".to_string());
        (self.status, [("content-type", "application/json")], json).into_response()
    }
}

fn get_router(
    state: &AppState,
    server: &str,
) -> Result<Arc<crate::upstream::RegionRouter>, AppError> {
    let region: ServerRegion = server
        .parse()
        .map_err(|_| AppError::InvalidServerRegion(server.to_string()))?;

    state
        .routers
        .get(&region)
        .cloned()
        .ok_or(AppError::NoClientAvailable)
}

async fn proxy_game_api(
    state: &AppState,
    server: &str,
    path: &str,
) -> Result<ApiResponse, AppError> {
    let router = get_router(state, server)?;
    let (data, status) = router.get_game_api(path, None).await?;

    Ok(ApiResponse {
        status: StatusCode::from_u16(status).unwrap_or(StatusCode::OK),
        body: data,
    })
}

async fn proxy_game_api_with_params(
    state: &AppState,
    server: &str,
    path: &str,
    params: &HashMap<String, String>,
) -> Result<ApiResponse, AppError> {
    let router = get_router(state, server)?;
    let (data, status) = router.get_game_api(path, Some(params)).await?;

    Ok(ApiResponse {
        status: StatusCode::from_u16(status).unwrap_or(StatusCode::OK),
        body: data,
    })
}

/// Cache TTLs (seconds) for the global, non-user-specific read endpoints.
// Cache TTLs bounded by each endpoint's freshness requirement: top100 is near
// real-time (<=1s), border tolerates 10-30s, system/information change rarely.
const RANKING_TOP100_CACHE_TTL_SECS: u64 = 1;
const RANKING_BORDER_CACHE_TTL_SECS: u64 = 30;
const STATIC_CACHE_TTL_SECS: u64 = 300;

fn json_string_response(status: StatusCode, json: String) -> Response {
    (status, [("content-type", "application/json")], json).into_response()
}

/// `proxy_game_api` with a short-lived Redis response cache for global read-only
/// endpoints (ranking / system / information). On a hit it returns the cached
/// JSON directly, skipping the upstream call, AES decrypt, Nuverse restore,
/// re-serialization, and the per-account request lock. The cache key omits any
/// account (the path keeps the literal `{userId}` placeholder), so all callers
/// share one entry. Only successful (200) responses are cached.
async fn cache_get(state: &AppState, key: &str) -> Option<String> {
    let mut conn = state.redis.as_ref()?.clone();
    redis::AsyncCommands::get::<_, Option<String>>(&mut conn, key)
        .await
        .ok()
        .flatten()
}

async fn cache_set(state: &AppState, key: &str, json: &str, ttl_secs: u64) {
    if let Some(ref redis) = state.redis {
        let mut conn = redis.clone();
        let _: Result<(), redis::RedisError> =
            redis::AsyncCommands::set_ex(&mut conn, key, json, ttl_secs).await;
    }
}

async fn proxy_game_api_cached(
    state: &AppState,
    server: &str,
    path: &str,
    ttl_secs: u64,
) -> Result<Response, AppError> {
    let cache_key = format!("haruki_sekai_resp:{server}:{path}");

    // Fast path: serve from cache while the entry is within its freshness window.
    if let Some(cached) = cache_get(state, &cache_key).await {
        return Ok(json_string_response(StatusCode::OK, cached));
    }

    // Coalesce concurrent misses for the same key onto a single upstream call;
    // followers await and share the leader's outcome (success or failure).
    let (outcome, _is_leader) = state
        .coalescer
        .coalesce(&cache_key, || async {
            let resp = proxy_game_api(state, server, path).await?;
            let status = resp.status.as_u16();
            let json: Arc<str> =
                Arc::from(sonic_rs::to_string(&resp.body).unwrap_or_else(|_| "{}".to_string()));
            // Populate the cache inside the in-flight window (200 only), so
            // requests arriving between slot release and SETEX completion still
            // hit the cache instead of stampeding upstream.
            if status == 200 {
                cache_set(state, &cache_key, &json, ttl_secs).await;
            }
            Ok((status, json))
        })
        .await;

    let (status, json) = outcome?;
    Ok(json_string_response(
        StatusCode::from_u16(status).unwrap_or(StatusCode::OK),
        json.to_string(),
    ))
}

async fn proxy_post_game_api_body<T: serde::Serialize>(
    state: &AppState,
    server: &str,
    path: &str,
    body: &T,
) -> Result<ApiResponse, AppError> {
    let router = get_router(state, server)?;
    let (data, status) = router.post_game_api_body(path, body, None).await?;

    Ok(ApiResponse {
        status: StatusCode::from_u16(status).unwrap_or(StatusCode::OK),
        body: data,
    })
}

fn parse_optional_bool(value: Option<&str>, name: &str, default: bool) -> Result<bool, AppError> {
    match value {
        None | Some("") => Ok(default),
        Some(value) => match value.to_ascii_lowercase().as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(AppError::ParseError(format!(
                "{} must be true or false",
                name
            ))),
        },
    }
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

pub async fn get_user_profile(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<Option<crate::api::middleware::AuthUser>>,
    Path((server, user_id)): Path<(String, String)>,
) -> Result<ApiResponse, AppError> {
    if !user_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::ParseError("user_id must be numeric".to_string()));
    }
    if let Some(user) = auth_user {
        tracing::debug!("User {} requesting profile for {}", user.id, user_id);
    }
    let path = format!("/user/{{userId}}/{}/profile", user_id);
    // Nuverse userHonors/userProfileHonors array->dict restoration is handled by
    // the schema bundle inside get_game_api (restore_nuverse_api_response), so no
    // endpoint-specific fixup is needed here.
    proxy_game_api(&state, &server, &path).await
}

pub async fn get_system(
    State(state): State<Arc<AppState>>,
    Path(server): Path<String>,
) -> Result<Response, AppError> {
    proxy_game_api_cached(&state, &server, "/system", STATIC_CACHE_TTL_SECS).await
}

pub async fn get_information(
    State(state): State<Arc<AppState>>,
    Path(server): Path<String>,
) -> Result<Response, AppError> {
    proxy_game_api_cached(&state, &server, "/information", STATIC_CACHE_TTL_SECS).await
}

pub async fn get_mysekai_housing_competition_list(
    State(state): State<Arc<AppState>>,
    Path((server, housing_id)): Path<(String, String)>,
    Query(query): Query<MySekaiHousingCompetitionListQuery>,
) -> Result<ApiResponse, AppError> {
    if !housing_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::ParseError(
            "housing_id must be numeric".to_string(),
        ));
    }

    let is_lottery = parse_optional_bool(query.is_lottery.as_deref(), "isLottery", true)?;
    let path = format!(
        "/user/{{userId}}/mysekai/housing-competition/{}/list",
        housing_id
    );
    let mut params = HashMap::new();
    params.insert(
        "isLottery".to_string(),
        if is_lottery { "True" } else { "False" }.to_string(),
    );

    proxy_game_api_with_params(&state, &server, &path, &params).await
}

pub async fn post_mysekai_housing_competition_entry(
    State(state): State<Arc<AppState>>,
    Path((server, housing_id, owner_user_id)): Path<(String, String, String)>,
    Query(query): Query<MySekaiHousingCompetitionEntryQuery>,
) -> Result<ApiResponse, AppError> {
    if !housing_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::ParseError(
            "housing_id must be numeric".to_string(),
        ));
    }
    if !owner_user_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::ParseError(
            "owner_user_id must be numeric".to_string(),
        ));
    }

    let is_back_number =
        parse_optional_bool(query.is_back_number.as_deref(), "isBackNumber", false)?;
    let body = json!({
        "isBackNumber": is_back_number,
        "mysekaiOwnerUserSubmittedAt": query.mysekai_owner_user_submitted_at,
    });
    let path = format!(
        "/user/{{userId}}/mysekai/housing-competition/{}/mysekai-owner/{}/entry",
        housing_id, owner_user_id
    );

    proxy_post_game_api_body(&state, &server, &path, &body).await
}

pub async fn get_mysekai_housing_competition_back_number_top_list(
    State(state): State<Arc<AppState>>,
    Path(server): Path<String>,
) -> Result<ApiResponse, AppError> {
    proxy_game_api(
        &state,
        &server,
        "/user/{userId}/mysekai/housing-competition/back-number-top-list",
    )
    .await
}

pub async fn get_mysekai_housing_competition_back_number_list(
    State(state): State<Arc<AppState>>,
    Path((server, competition_id)): Path<(String, String)>,
) -> Result<ApiResponse, AppError> {
    if !competition_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::ParseError(
            "competition_id must be numeric".to_string(),
        ));
    }
    let path = format!(
        "/user/{{userId}}/mysekai/housing-competition/{}/back-number-list",
        competition_id
    );
    proxy_game_api(&state, &server, &path).await
}

pub async fn get_custom_music_score_published_search(
    State(state): State<Arc<AppState>>,
    Path((server, user_id, score_id)): Path<(String, String, String)>,
) -> Result<ApiResponse, AppError> {
    let user_path = match user_id.as_str() {
        "%user_id" | "%25user_id" | "{userId}" => "{userId}",
        value if value.chars().all(|c| c.is_ascii_digit()) => value,
        _ => {
            return Err(AppError::ParseError(
                "user_id must be numeric or %user_id".to_string(),
            ));
        }
    };
    let trimmed_score_id = score_id.trim();
    if trimmed_score_id.is_empty() {
        return Err(AppError::ParseError("score_id is empty".to_string()));
    }
    // Reject dot segments: '.' is unreserved in encode_path_segment, so "." / ".."
    // would survive into the upstream URL where reqwest's WHATWG URL parser
    // collapses them, steering the authenticated request to a different endpoint.
    if matches!(trimmed_score_id, "." | "..") {
        return Err(AppError::ParseError(
            "score_id must not be a dot segment".to_string(),
        ));
    }
    let path = format!(
        "/user/{}/custom-music-score/published/search/{}",
        user_path,
        encode_path_segment(&score_id)
    );
    proxy_game_api(&state, &server, &path).await
}

pub async fn get_event_ranking_top100(
    State(state): State<Arc<AppState>>,
    Path((server, event_id)): Path<(String, String)>,
) -> Result<Response, AppError> {
    if !event_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::ParseError("event_id must be numeric".to_string()));
    }
    let path = format!(
        "/user/{{userId}}/event/{}/ranking?rankingViewType=top100",
        event_id
    );
    proxy_game_api_cached(&state, &server, &path, RANKING_TOP100_CACHE_TTL_SECS).await
}

pub async fn get_event_ranking_border(
    State(state): State<Arc<AppState>>,
    Path((server, event_id)): Path<(String, String)>,
) -> Result<Response, AppError> {
    if !event_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::ParseError("event_id must be numeric".to_string()));
    }
    let path = format!("/event/{}/ranking-border", event_id);
    proxy_game_api_cached(&state, &server, &path, RANKING_BORDER_CACHE_TTL_SECS).await
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::extract::{Path, Query, State};
    use axum::http::Response as AxumResponse;
    use axum::response::IntoResponse;
    use axum::Router;

    use super::*;
    use crate::config::{Config, ServerRegion, UpstreamConfig};
    use crate::upstream::{build_internal_http_client, InternalApiResponse, RegionRouter};
    use crate::RequestCoalescer;

    async fn response_handler() -> AxumResponse<Body> {
        let envelope = InternalApiResponse {
            ok: true,
            status: Some(200),
            data: Some(json!({"proxied": true})),
            kind: None,
            message: None,
        };
        AxumResponse::builder()
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&envelope).unwrap()))
            .unwrap()
    }

    async fn state_with_router() -> (Arc<AppState>, tokio::task::JoinHandle<()>) {
        let app = Router::new().fallback(response_handler);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let upstream = UpstreamConfig {
            url: format!("http://{address}"),
            token: "token".to_string(),
            priority: 0,
            name: "mock".to_string(),
        };
        let router = RegionRouter::new(
            ServerRegion::Jp,
            None,
            &[upstream],
            build_internal_http_client().unwrap(),
        )
        .unwrap();
        let config: Config = serde_yaml::from_str("backend: {}").unwrap();
        let state = AppState {
            config,
            clients: HashMap::new(),
            routers: HashMap::from([(ServerRegion::Jp, Arc::new(router))]),
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

    #[test]
    fn parses_booleans_encodes_paths_and_serializes_api_response() {
        assert!(parse_optional_bool(None, "flag", true).unwrap());
        assert!(!parse_optional_bool(Some(""), "flag", false).unwrap());
        assert!(parse_optional_bool(Some("TRUE"), "flag", false).unwrap());
        assert!(!parse_optional_bool(Some("false"), "flag", true).unwrap());
        assert!(parse_optional_bool(Some("yes"), "flag", false).is_err());
        assert_eq!(encode_path_segment("a b/中-._~"), "a%20b%2F%E4%B8%AD-._~");

        let response = ApiResponse {
            status: StatusCode::CREATED,
            body: json!({"ok": true}),
        }
        .into_response();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn validates_router_and_endpoint_identifiers() {
        let (state, server) = state_with_router().await;
        assert!(matches!(
            get_router(&state, "bad"),
            Err(AppError::InvalidServerRegion(_))
        ));
        assert!(matches!(
            get_router(&state, "tw"),
            Err(AppError::NoClientAvailable)
        ));

        assert!(get_user_profile(
            State(state.clone()),
            axum::Extension(None),
            Path(("jp".to_string(), "abc".to_string()))
        )
        .await
        .is_err());
        assert!(get_mysekai_housing_competition_list(
            State(state.clone()),
            Path(("jp".to_string(), "x".to_string())),
            Query(MySekaiHousingCompetitionListQuery { is_lottery: None })
        )
        .await
        .is_err());
        assert!(post_mysekai_housing_competition_entry(
            State(state.clone()),
            Path(("jp".to_string(), "x".to_string(), "2".to_string())),
            Query(MySekaiHousingCompetitionEntryQuery {
                is_back_number: None,
                mysekai_owner_user_submitted_at: 1,
            })
        )
        .await
        .is_err());
        assert!(post_mysekai_housing_competition_entry(
            State(state.clone()),
            Path(("jp".to_string(), "1".to_string(), "x".to_string())),
            Query(MySekaiHousingCompetitionEntryQuery {
                is_back_number: None,
                mysekai_owner_user_submitted_at: 1,
            })
        )
        .await
        .is_err());
        assert!(get_mysekai_housing_competition_back_number_list(
            State(state.clone()),
            Path(("jp".to_string(), "x".to_string()))
        )
        .await
        .is_err());
        assert!(get_event_ranking_top100(
            State(state.clone()),
            Path(("jp".to_string(), "x".to_string()))
        )
        .await
        .is_err());
        assert!(get_event_ranking_border(
            State(state.clone()),
            Path(("jp".to_string(), "x".to_string()))
        )
        .await
        .is_err());

        for (user, score) in [("bad", "score"), ("1", ""), ("1", "."), ("1", "..")] {
            assert!(get_custom_music_score_published_search(
                State(state.clone()),
                Path(("jp".to_string(), user.to_string(), score.to_string()))
            )
            .await
            .is_err());
        }
        server.abort();
    }

    #[tokio::test]
    async fn proxies_all_get_and_post_handlers() {
        let (state, server) = state_with_router().await;
        let auth = crate::api::middleware::AuthUser {
            id: "caller".to_string(),
            credential: "credential".to_string(),
        };
        let profile = get_user_profile(
            State(state.clone()),
            axum::Extension(Some(auth)),
            Path(("jp".to_string(), "123".to_string())),
        )
        .await
        .unwrap();
        assert_eq!(profile.body["proxied"], true);

        let list = get_mysekai_housing_competition_list(
            State(state.clone()),
            Path(("jp".to_string(), "1".to_string())),
            Query(MySekaiHousingCompetitionListQuery {
                is_lottery: Some("false".to_string()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(list.status, StatusCode::OK);
        let entry = post_mysekai_housing_competition_entry(
            State(state.clone()),
            Path(("jp".to_string(), "1".to_string(), "2".to_string())),
            Query(MySekaiHousingCompetitionEntryQuery {
                is_back_number: Some("true".to_string()),
                mysekai_owner_user_submitted_at: 10,
            }),
        )
        .await
        .unwrap();
        assert_eq!(entry.body["proxied"], true);

        assert!(get_mysekai_housing_competition_back_number_top_list(
            State(state.clone()),
            Path("jp".to_string())
        )
        .await
        .is_ok());
        assert!(get_mysekai_housing_competition_back_number_list(
            State(state.clone()),
            Path(("jp".to_string(), "7".to_string()))
        )
        .await
        .is_ok());
        for user in ["%user_id", "%25user_id", "{userId}", "123"] {
            assert!(get_custom_music_score_published_search(
                State(state.clone()),
                Path(("jp".to_string(), user.to_string(), "score / id".to_string()))
            )
            .await
            .is_ok());
        }

        for response in [
            get_system(State(state.clone()), Path("jp".to_string()))
                .await
                .unwrap(),
            get_information(State(state.clone()), Path("jp".to_string()))
                .await
                .unwrap(),
            get_event_ranking_top100(
                State(state.clone()),
                Path(("jp".to_string(), "1".to_string())),
            )
            .await
            .unwrap(),
            get_event_ranking_border(
                State(state.clone()),
                Path(("jp".to_string(), "1".to_string())),
            )
            .await
            .unwrap(),
        ] {
            assert_eq!(response.status(), StatusCode::OK);
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert_eq!(
                serde_json::from_slice::<JsonValue>(&body).unwrap()["proxied"],
                true
            );
        }
        server.abort();
    }
}
