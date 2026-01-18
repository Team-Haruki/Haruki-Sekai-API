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
    let region = match ServerRegion::from_str(&server) {
        Some(r) => r,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Invalid server: {}", server),
            )
                .into_response();
        }
    };
    let Some(manager) = state.managers.get(&region) else {
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
        manager.get_cp_mysekai_image(&combined).await
    } else {
        if !digits.is_match(&param1) || !digits.is_match(&param2) {
            return (
                StatusCode::BAD_REQUEST,
                "Invalid path format for nuverse servers (expected numeric user_id and index)",
            )
                .into_response();
        }
        manager.get_nuverse_mysekai_image(&param1, &param2).await
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
