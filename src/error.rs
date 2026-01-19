use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Session expired")]
    SessionError,

    #[error("Cookie expired")]
    CookieExpired,

    #[error("Upgrade required")]
    UpgradeRequired,

    #[error("Server under maintenance")]
    UnderMaintenance,

    #[error("Invalid signature")]
    SignatureError,

    #[error("No accounts configured")]
    NoAccountError,

    #[error("No client available")]
    NoClientAvailable,

    #[error("Invalid server region: {0}")]
    InvalidServerRegion(String),

    #[error("Invalid HTTP status: {0}")]
    InvalidHttpStatus(u16),

    #[error("Crypto error: {0}")]
    CryptoError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Redis error: {0}")]
    RedisError(String),

    #[error("IO error: {0}")]
    IoError(String),

    #[error("Authentication error: {0}")]
    AuthError(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Unknown error: status={status}, body={body}")]
    Unknown { status: u16, body: String },
}

impl AppError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            AppError::SessionError | AppError::CookieExpired => StatusCode::FORBIDDEN,
            AppError::UpgradeRequired => StatusCode::UPGRADE_REQUIRED,
            AppError::UnderMaintenance => StatusCode::SERVICE_UNAVAILABLE,
            AppError::InvalidServerRegion(_) | AppError::ParseError(_) => StatusCode::BAD_REQUEST,
            AppError::AuthError(_) => StatusCode::UNAUTHORIZED,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::NoClientAvailable | AppError::NoAccountError => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ApiErrorResponse {
    pub result: &'static str,
    pub status: u16,
    pub message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = ApiErrorResponse {
            result: "failed",
            status: status.as_u16(),
            message: self.to_string(),
        };
        let json = sonic_rs::to_string(&body).unwrap_or_else(|_| {
            r#"{"result":"failed","status":500,"message":"Internal error"}"#.to_string()
        });
        (status, [("content-type", "application/json")], json).into_response()
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        AppError::NetworkError(e.to_string())
    }
}

impl From<sea_orm::DbErr> for AppError {
    fn from(e: sea_orm::DbErr) -> Self {
        AppError::DatabaseError(e.to_string())
    }
}

impl From<redis::RedisError> for AppError {
    fn from(e: redis::RedisError) -> Self {
        AppError::RedisError(e.to_string())
    }
}

impl From<sonic_rs::Error> for AppError {
    fn from(e: sonic_rs::Error) -> Self {
        AppError::ParseError(e.to_string())
    }
}

impl From<rmp_serde::decode::Error> for AppError {
    fn from(e: rmp_serde::decode::Error) -> Self {
        AppError::ParseError(format!("MsgPack decode error: {}", e))
    }
}

impl From<rmp_serde::encode::Error> for AppError {
    fn from(e: rmp_serde::encode::Error) -> Self {
        AppError::ParseError(format!("MsgPack encode error: {}", e))
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::IoError(e.to_string())
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, num_enum::TryFromPrimitive, num_enum::IntoPrimitive,
)]
#[repr(u16)]
pub enum SekaiHttpStatus {
    Ok = 200,
    ClientError = 400,
    SessionError = 403,
    NotFound = 404,
    Conflict = 409,
    GameUpgrade = 426,
    ServerError = 500,
    UnderMaintenance = 503,
}

impl SekaiHttpStatus {
    pub fn from_code(code: u16) -> Result<Self, AppError> {
        Self::try_from(code).map_err(|_| AppError::InvalidHttpStatus(code))
    }
}
