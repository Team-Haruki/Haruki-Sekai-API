use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};

use crate::db::entity::{sekai_user, sekai_user_server};
use crate::error::ApiErrorResponse;
use crate::AppState;

/// Auth cache TTL. Deliberately short: revocation by deleting the user or the
/// per-server grant row does not rotate the credential (which is part of the
/// cache key), so this TTL bounds how long a revoked user keeps access.
const AUTH_CACHE_TTL_SECS: u64 = 300;
type AuthFailure = (StatusCode, String);

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub uid: String,
    pub credential: String,
    #[serde(default)]
    pub exp: Option<usize>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AuthUser {
    pub id: String,
    pub credential: String,
}

pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    req.extensions_mut().insert(None::<AuthUser>);
    // Open mode is INTENTIONAL: a deployment that does not enable the user
    // database (or does not configure a JWT signing key) runs without
    // authentication, and every endpoint is deliberately public. Do not "fix"
    // this to fail closed — auth is opted into via configuration.
    let Some(ref db) = state.db else {
        return next.run(req).await;
    };
    let jwt_secret = match &state.jwt_secret {
        Some(s) if !s.is_empty() => s,
        _ => return next.run(req).await,
    };
    let token = match extract_token(req.headers()) {
        Ok(token) => token,
        Err((status, message)) => return error_response(status, &message),
    };
    let claims = match decode_claims(&token, jwt_secret) {
        Ok(claims) => claims,
        Err((status, message)) => return error_response(status, &message),
    };
    if claims.uid.is_empty() || claims.credential.is_empty() {
        return error_response(StatusCode::UNAUTHORIZED, "Invalid token payload");
    }
    let path = req.uri().path();
    tracing::debug!("Extracting server from path: {}", path);
    let server = extract_server_from_path(path);
    tracing::debug!("Extracted server: {}", server);
    if authorization_is_cached(state.redis.as_ref(), &claims, &server).await {
        insert_auth_user(&mut req, &claims);
        return next.run(req).await;
    }
    let user = match authorize_user(db, &claims, &server).await {
        Ok(user) => user,
        Err((status, message)) => return error_response(status, &message),
    };
    cache_authorization(state.redis.as_ref(), &user, &server).await;
    req.extensions_mut().insert(Some(user));
    next.run(req).await
}

fn extract_token(headers: &axum::http::HeaderMap) -> Result<String, AuthFailure> {
    let header = headers
        .get("x-haruki-sekai-token")
        .ok_or_else(|| auth_failure(StatusCode::UNAUTHORIZED, "Missing token"))?;
    header
        .to_str()
        .map(str::to_owned)
        .map_err(|_| auth_failure(StatusCode::UNAUTHORIZED, "Invalid token header"))
}

fn decode_claims(token: &str, jwt_secret: &str) -> Result<Claims, AuthFailure> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.required_spec_claims.clear();
    validation.validate_exp = false;
    decode::<Claims>(
        &token,
        &DecodingKey::from_secret(jwt_secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|e| {
        tracing::warn!("JWT decode failed: {:?}", e);
        auth_failure(StatusCode::UNAUTHORIZED, format!("Invalid token: {}", e))
    })
}

fn auth_failure(status: StatusCode, message: impl Into<String>) -> AuthFailure {
    (status, message.into())
}

fn cache_key(user_id: &str, server: &str, credential: &str) -> String {
    format!("haruki_sekai_api:{user_id}:{server}:{credential}")
}

async fn authorization_is_cached(
    redis: Option<&redis::aio::ConnectionManager>,
    claims: &Claims,
    server: &str,
) -> bool {
    let Some(redis) = redis else {
        return false;
    };
    let mut conn = redis.clone();
    let key = cache_key(&claims.uid, server, &claims.credential);
    redis::AsyncCommands::get::<_, Option<String>>(&mut conn, key)
        .await
        .ok()
        .flatten()
        .is_some()
}

fn insert_auth_user(req: &mut Request<Body>, claims: &Claims) {
    req.extensions_mut().insert(Some(AuthUser {
        id: claims.uid.clone(),
        credential: claims.credential.clone(),
    }));
}

async fn authorize_user(
    db: &DatabaseConnection,
    claims: &Claims,
    server: &str,
) -> Result<AuthUser, AuthFailure> {
    tracing::debug!("Checking user {} for server {}", claims.uid, server);
    let user = sekai_user::Entity::find_by_id(&claims.uid)
        .one(db)
        .await
        .map_err(|e| {
            tracing::error!("Database error looking up user: {:?}", e);
            auth_failure(StatusCode::INTERNAL_SERVER_ERROR, "Database error")
        })?
        .ok_or_else(|| {
            tracing::warn!("User {} not found in database", claims.uid);
            auth_failure(StatusCode::UNAUTHORIZED, "User not found")
        })?;
    if user.credential != claims.credential {
        return Err(auth_failure(StatusCode::UNAUTHORIZED, "Invalid credential"));
    }
    tracing::debug!(
        "Checking server authorization: user={}, server={}",
        user.id,
        server
    );
    let authorized = sekai_user_server::Entity::find()
        .filter(sekai_user_server::Column::UserId.eq(&user.id))
        .filter(sekai_user_server::Column::Server.eq(server))
        .one(db)
        .await
        .map_err(|e| {
            tracing::error!("Database error checking server auth: {:?}", e);
            auth_failure(StatusCode::INTERNAL_SERVER_ERROR, "Database error")
        })?
        .is_some();
    if !authorized {
        tracing::warn!("User {} not authorized for server {}", user.id, server);
        return Err(auth_failure(
            StatusCode::FORBIDDEN,
            "Not authorized for this server",
        ));
    }
    tracing::debug!("User {} authorized for server {}", user.id, server);
    Ok(AuthUser {
        id: claims.uid.clone(),
        credential: claims.credential.clone(),
    })
}

async fn cache_authorization(
    redis: Option<&redis::aio::ConnectionManager>,
    user: &AuthUser,
    server: &str,
) {
    let Some(redis) = redis else {
        return;
    };
    let mut conn = redis.clone();
    let key = cache_key(&user.id, server, &user.credential);
    let _: Result<(), _> =
        redis::AsyncCommands::set_ex(&mut conn, key, "1", AUTH_CACHE_TTL_SECS).await;
}

fn extract_server_from_path(path: &str) -> String {
    let mut parts = path.split('/').filter(|s| !s.is_empty());
    let first = match parts.next() {
        Some(part) => part,
        None => return String::new(),
    };
    let server = if first.eq_ignore_ascii_case("api") || first.eq_ignore_ascii_case("image") {
        parts.next().unwrap_or_default()
    } else {
        first
    };
    server.to_ascii_lowercase()
}

fn error_response(status: StatusCode, message: &str) -> Response {
    let body = ApiErrorResponse {
        result: "failed",
        status: status.as_u16(),
        message: message.to_string(),
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

#[cfg(test)]
mod tests {
    use super::extract_server_from_path;

    #[test]
    fn extract_server_uses_segment_after_api_prefix() {
        assert_eq!(extract_server_from_path("/api/jp/system"), "jp");
    }

    #[test]
    fn extract_server_falls_back_to_first_segment_without_api_prefix() {
        assert_eq!(extract_server_from_path("/jp/system"), "jp");
    }

    #[test]
    fn extract_server_handles_empty_or_incomplete_paths() {
        assert_eq!(extract_server_from_path("/"), "");
        assert_eq!(extract_server_from_path("/api"), "");
    }

    #[test]
    fn extract_server_uses_segment_after_image_prefix() {
        assert_eq!(extract_server_from_path("/image/tw/mysekai/1/2"), "tw");
    }
}
