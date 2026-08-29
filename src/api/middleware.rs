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
    use axum::body::to_bytes;
    use axum::http::{HeaderMap, HeaderValue};
    use jsonwebtoken::{encode, EncodingKey, Header};
    use sea_orm::{ConnectionTrait, Database};

    use super::*;

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

    #[test]
    fn token_header_is_required_and_must_be_text() {
        let headers = HeaderMap::new();
        assert_eq!(
            extract_token(&headers).unwrap_err(),
            (StatusCode::UNAUTHORIZED, "Missing token".to_string())
        );

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-haruki-sekai-token",
            HeaderValue::from_bytes(&[0xff]).unwrap(),
        );
        assert_eq!(
            extract_token(&headers).unwrap_err(),
            (StatusCode::UNAUTHORIZED, "Invalid token header".to_string())
        );

        headers.insert(
            "x-haruki-sekai-token",
            HeaderValue::from_static("signed-token"),
        );
        assert_eq!(extract_token(&headers).unwrap(), "signed-token");
    }

    #[test]
    fn claims_decode_with_matching_secret_and_reject_wrong_secret() {
        let claims = Claims {
            uid: "user".to_string(),
            credential: "credential".to_string(),
            exp: None,
        };
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(b"correct-secret"),
        )
        .unwrap();

        let decoded = decode_claims(&token, "correct-secret").unwrap();
        assert_eq!(decoded.uid, "user");
        assert_eq!(decoded.credential, "credential");
        let failure = decode_claims(&token, "wrong-secret").unwrap_err();
        assert_eq!(failure.0, StatusCode::UNAUTHORIZED);
        assert!(failure.1.starts_with("Invalid token:"));
    }

    #[tokio::test]
    async fn no_redis_cache_is_a_clean_miss_and_noop_write() {
        let claims = Claims {
            uid: "user".to_string(),
            credential: "credential".to_string(),
            exp: None,
        };
        assert!(!authorization_is_cached(None, &claims, "jp").await);
        let user = AuthUser {
            id: claims.uid.clone(),
            credential: claims.credential.clone(),
        };
        cache_authorization(None, &user, "jp").await;
        assert_eq!(
            cache_key(&user.id, "jp", &user.credential),
            "haruki_sekai_api:user:jp:credential"
        );
    }

    #[test]
    fn inserts_authenticated_user_into_request_extensions() {
        let claims = Claims {
            uid: "user".to_string(),
            credential: "credential".to_string(),
            exp: None,
        };
        let mut request = Request::new(Body::empty());
        insert_auth_user(&mut request, &claims);

        let user = request
            .extensions()
            .get::<Option<AuthUser>>()
            .and_then(Option::as_ref)
            .unwrap();
        assert_eq!(user.id, "user");
        assert_eq!(user.credential, "credential");
    }

    #[tokio::test]
    async fn error_response_has_matching_status_and_json_body() {
        let response = error_response(StatusCode::FORBIDDEN, "denied");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = sonic_rs::from_slice(&body).unwrap();
        assert_eq!(json["status"], 403);
        assert_eq!(json["message"], "denied");
    }

    async fn auth_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared(
            "CREATE TABLE sekai_users (id TEXT PRIMARY KEY, credential TEXT NOT NULL, remark TEXT NOT NULL DEFAULT '')",
        )
        .await
        .unwrap();
        db.execute_unprepared(
            "CREATE TABLE sekai_user_servers (user_id TEXT NOT NULL, server TEXT NOT NULL, PRIMARY KEY (user_id, server))",
        )
        .await
        .unwrap();
        db
    }

    #[tokio::test]
    async fn database_authorization_checks_user_credential_and_server_grant() {
        let db = auth_db().await;
        let claims = Claims {
            uid: "user".to_string(),
            credential: "credential".to_string(),
            exp: None,
        };
        let missing = authorize_user(&db, &claims, "jp").await.unwrap_err();
        assert_eq!(missing.0, StatusCode::UNAUTHORIZED);

        db.execute_unprepared(
            "INSERT INTO sekai_users (id, credential, remark) VALUES ('user', 'credential', '')",
        )
        .await
        .unwrap();
        let mut wrong = Claims {
            uid: claims.uid.clone(),
            credential: "wrong".to_string(),
            exp: None,
        };
        assert_eq!(
            authorize_user(&db, &wrong, "jp").await.unwrap_err().0,
            StatusCode::UNAUTHORIZED
        );
        wrong.credential = claims.credential.clone();
        assert_eq!(
            authorize_user(&db, &wrong, "jp").await.unwrap_err().0,
            StatusCode::FORBIDDEN
        );

        db.execute_unprepared(
            "INSERT INTO sekai_user_servers (user_id, server) VALUES ('user', 'jp')",
        )
        .await
        .unwrap();
        let user = authorize_user(&db, &claims, "jp").await.unwrap();
        assert_eq!(user.id, "user");
        assert_eq!(user.credential, "credential");
    }

    #[tokio::test]
    async fn database_authorization_maps_query_failures_to_server_error() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let claims = Claims {
            uid: "user".to_string(),
            credential: "credential".to_string(),
            exp: None,
        };
        assert_eq!(
            authorize_user(&db, &claims, "jp").await.unwrap_err().0,
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
