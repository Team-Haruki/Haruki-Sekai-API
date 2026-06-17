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

fn get_client(state: &AppState, server: &str) -> Result<Arc<crate::client::SekaiClient>, AppError> {
    let region: ServerRegion = server
        .parse()
        .map_err(|_| AppError::InvalidServerRegion(server.to_string()))?;

    state
        .clients
        .get(&region)
        .cloned()
        .ok_or(AppError::NoClientAvailable)
}

async fn proxy_game_api(
    state: &AppState,
    server: &str,
    path: &str,
) -> Result<ApiResponse, AppError> {
    let client = get_client(state, server)?;
    let (data, status) = client.get_game_api(path, None).await?;

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
    let client = get_client(state, server)?;
    let (data, status) = client.get_game_api(path, Some(params)).await?;

    Ok(ApiResponse {
        status: StatusCode::from_u16(status).unwrap_or(StatusCode::OK),
        body: data,
    })
}

async fn proxy_post_game_api_body<T: serde::Serialize>(
    state: &AppState,
    server: &str,
    path: &str,
    body: &T,
) -> Result<ApiResponse, AppError> {
    let client = get_client(state, server)?;
    let (data, status) = client.post_game_api_body(path, body, None).await?;

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
    proxy_game_api(&state, &server, &path).await
}

pub async fn get_system(
    State(state): State<Arc<AppState>>,
    Path(server): Path<String>,
) -> Result<ApiResponse, AppError> {
    proxy_game_api(&state, &server, "/system").await
}

pub async fn get_information(
    State(state): State<Arc<AppState>>,
    Path(server): Path<String>,
) -> Result<ApiResponse, AppError> {
    proxy_game_api(&state, &server, "/information").await
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
    if score_id.trim().is_empty() {
        return Err(AppError::ParseError("score_id is empty".to_string()));
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
) -> Result<ApiResponse, AppError> {
    if !event_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::ParseError("event_id must be numeric".to_string()));
    }
    let path = format!(
        "/user/{{userId}}/event/{}/ranking?rankingViewType=top100",
        event_id
    );
    proxy_game_api(&state, &server, &path).await
}

pub async fn get_event_ranking_border(
    State(state): State<Arc<AppState>>,
    Path((server, event_id)): Path<(String, String)>,
) -> Result<ApiResponse, AppError> {
    if !event_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::ParseError("event_id must be numeric".to_string()));
    }
    let path = format!("/event/{}/ranking-border", event_id);
    proxy_game_api(&state, &server, &path).await
}
