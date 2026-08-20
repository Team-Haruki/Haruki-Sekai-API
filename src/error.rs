use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use thiserror::Error;

#[derive(Debug, Clone, Error)]
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

    #[error("Upstream data error: {0}")]
    UpstreamData(String),

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
    /// Stable machine-readable tag for this error, used by the internal
    /// upstream-forwarding envelope so a remote node's failure class survives
    /// the HTTP hop and the primary can classify it (failover vs terminal).
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::SessionError => "session_error",
            AppError::CookieExpired => "cookie_expired",
            AppError::UpgradeRequired => "upgrade_required",
            AppError::UnderMaintenance => "under_maintenance",
            AppError::SignatureError => "signature_error",
            AppError::NoAccountError => "no_account",
            AppError::NoClientAvailable => "no_client",
            AppError::InvalidServerRegion(_) => "invalid_server_region",
            AppError::InvalidHttpStatus(_) => "invalid_http_status",
            AppError::CryptoError(_) => "crypto",
            AppError::ParseError(_) => "parse",
            AppError::UpstreamData(_) => "upstream_data",
            AppError::NetworkError(_) => "network",
            AppError::DatabaseError(_) => "database",
            AppError::RedisError(_) => "redis",
            AppError::IoError(_) => "io",
            AppError::AuthError(_) => "auth",
            AppError::NotFound(_) => "not_found",
            AppError::Forbidden(_) => "forbidden",
            AppError::Internal(_) => "internal",
            AppError::Unknown { .. } => "unknown",
        }
    }

    /// Reconstruct an error from an envelope `(kind, status, message)` triple.
    /// Inverse of `kind()` up to message contents; an unrecognized kind (e.g.
    /// from a newer remote node) degrades to `Internal` rather than failing.
    pub fn from_kind(kind: &str, status: Option<u16>, message: String) -> Self {
        match kind {
            "session_error" => AppError::SessionError,
            "cookie_expired" => AppError::CookieExpired,
            "upgrade_required" => AppError::UpgradeRequired,
            "under_maintenance" => AppError::UnderMaintenance,
            "signature_error" => AppError::SignatureError,
            "no_account" => AppError::NoAccountError,
            "no_client" => AppError::NoClientAvailable,
            "invalid_server_region" => AppError::InvalidServerRegion(message),
            "invalid_http_status" => AppError::InvalidHttpStatus(status.unwrap_or(0)),
            "crypto" => AppError::CryptoError(message),
            "parse" => AppError::ParseError(message),
            "upstream_data" => AppError::UpstreamData(message),
            "network" => AppError::NetworkError(message),
            "database" => AppError::DatabaseError(message),
            "redis" => AppError::RedisError(message),
            "io" => AppError::IoError(message),
            "auth" => AppError::AuthError(message),
            "not_found" => AppError::NotFound(message),
            "forbidden" => AppError::Forbidden(message),
            "internal" => AppError::Internal(message),
            "unknown" => AppError::Unknown {
                status: status.unwrap_or(0),
                body: message,
            },
            other => AppError::Internal(format!("unrecognized error kind '{other}': {message}")),
        }
    }

    pub fn status_code(&self) -> StatusCode {
        match self {
            AppError::SessionError | AppError::CookieExpired => StatusCode::FORBIDDEN,
            AppError::UpgradeRequired => StatusCode::UPGRADE_REQUIRED,
            AppError::UnderMaintenance => StatusCode::SERVICE_UNAVAILABLE,
            AppError::InvalidServerRegion(_) | AppError::ParseError(_) => StatusCode::BAD_REQUEST,
            // Upstream-fault classes: the game server (or the path to it) broke,
            // not this service — surface as 502 so callers don't misattribute.
            AppError::UpstreamData(_)
            | AppError::NetworkError(_)
            | AppError::InvalidHttpStatus(_)
            | AppError::Unknown { .. } => StatusCode::BAD_GATEWAY,
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
        match sonic_rs::to_string(&body) {
            Ok(json) => (status, [("content-type", "application/json")], json).into_response(),
            // Keep the HTTP status consistent with the fallback body's status.
            Err(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                [("content-type", "application/json")],
                r#"{"result":"failed","status":500,"message":"Internal error"}"#.to_string(),
            )
                .into_response(),
        }
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
        // Decoding an upstream game-server response that failed to deserialize:
        // the upstream payload is at fault, not the API caller.
        AppError::UpstreamData(format!("MsgPack decode error: {}", e))
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
