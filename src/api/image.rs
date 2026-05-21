use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use regex::Regex;

use crate::config::ServerRegion;
use crate::AppState;

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
    let Some(client) = state.clients.get(&region) else {
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
        let combined = format!("{}/{}", param1, param2);
        client.get_cp_mysekai_image(&combined).await
    } else {
        if !digits.is_match(&param1) || !digits.is_match(&param2) {
            return (
                StatusCode::BAD_REQUEST,
                "Invalid path format for nuverse servers (expected numeric user_id and index)",
            )
                .into_response();
        }
        client.get_nuverse_mysekai_image(&param1, &param2).await
    };
    match image_result {
        Ok(bytes) => (StatusCode::OK, [("content-type", "image/png")], bytes).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            format!("Fetch image failed: {}", e),
        )
            .into_response(),
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
    let Some(client) = state.clients.get(&region) else {
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
    let combined = format!("{}/{}", param1, param2);
    match client.get_cp_custom_profile_card_thumbnail(&combined).await {
        Ok(bytes) => (StatusCode::OK, [("content-type", "image/png")], bytes).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            format!("Fetch image failed: {}", e),
        )
            .into_response(),
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
    let Some(client) = state.clients.get(&region) else {
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
    let combined = format!("{}/{}", param1, param2);
    match client.get_cp_custom_music_score(&combined).await {
        Ok(bytes) => (
            StatusCode::OK,
            [("content-type", "application/octet-stream")],
            bytes,
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            format!("Fetch custom music score failed: {}", e),
        )
            .into_response(),
    }
}
