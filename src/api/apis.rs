use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::Value as JsonValue;

use crate::config::ServerRegion;
use crate::error::AppError;
use crate::AppState;

pub struct ApiResponse {
    status: StatusCode,
    body: JsonValue,
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
    let mut resp = proxy_game_api(&state, &server, &path).await?;

    // Nuverse servers (TW/KR/CN) may return userHonors elements as flat arrays; restore to dicts
    let region: ServerRegion = server
        .parse()
        .map_err(|_| AppError::InvalidServerRegion(server.to_string()))?;
    if !region.is_cp_server() {
        crate::client::nuverse::restore_profile_user_honors(&mut resp.body);
    }

    Ok(resp)
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
    let mut resp = proxy_game_api(&state, &server, &path).await?;

    // Nuverse servers (TW/KR/CN) return userCard as a flat array; restore to keyed dict
    let region: ServerRegion = server
        .parse()
        .map_err(|_| AppError::InvalidServerRegion(server.to_string()))?;
    if !region.is_cp_server() {
        crate::client::nuverse::restore_ranking_user_cards(&mut resp.body);
    }

    Ok(resp)
}

pub async fn get_event_ranking_border(
    State(state): State<Arc<AppState>>,
    Path((server, event_id)): Path<(String, String)>,
) -> Result<ApiResponse, AppError> {
    if !event_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::ParseError("event_id must be numeric".to_string()));
    }
    let path = format!("/event/{}/ranking-border", event_id);
    let mut resp = proxy_game_api(&state, &server, &path).await?;

    // Nuverse servers (TW/KR/CN) return userCard as a flat array; restore to keyed dict
    let region: ServerRegion = server
        .parse()
        .map_err(|_| AppError::InvalidServerRegion(server.to_string()))?;
    if !region.is_cp_server() {
        crate::client::nuverse::restore_ranking_user_cards(&mut resp.body);
    }

    Ok(resp)
}
