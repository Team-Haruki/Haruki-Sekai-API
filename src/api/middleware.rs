use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};

use crate::db::entity::{sekai_user, sekai_user_server};
use crate::error::ApiErrorResponse;
use crate::AppState;

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
    if state.db.is_none() {
        return next.run(req).await;
    }
    let jwt_secret = match &state.jwt_secret {
        Some(s) if !s.is_empty() => s,
        _ => return next.run(req).await,
    };
    let token = match req.headers().get("x-haruki-sekai-token") {
        Some(h) => match h.to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return error_response(StatusCode::UNAUTHORIZED, "Invalid token header"),
        },
        None => return error_response(StatusCode::UNAUTHORIZED, "Missing token"),
    };
    let mut validation = Validation::new(Algorithm::HS256);
    validation.required_spec_claims.clear();
    validation.validate_exp = false;
    let claims = match decode::<Claims>(
        &token,
        &DecodingKey::from_secret(jwt_secret.as_bytes()),
        &validation,
    ) {
        Ok(data) => data.claims,
        Err(e) => {
            tracing::warn!("JWT decode failed: {:?}", e);
            return error_response(StatusCode::UNAUTHORIZED, &format!("Invalid token: {}", e));
        }
    };
    if claims.uid.is_empty() || claims.credential.is_empty() {
        return error_response(StatusCode::UNAUTHORIZED, "Invalid token payload");
    }
    let path = req.uri().path();
    tracing::debug!("Extracting server from path: {}", path);
    let server = extract_server_from_path(path);
    tracing::debug!("Extracted server: {}", server);
    if let Some(ref redis) = state.redis {
        let cache_key = format!("haruki_sekai_api:{}:{}", claims.uid, server);
        let mut conn = redis.clone();
        if let Ok(val) = redis::AsyncCommands::get::<_, Option<String>>(&mut conn, &cache_key).await
        {
            if val.is_some() {
                req.extensions_mut().insert(Some(AuthUser {
                    id: claims.uid,
                    credential: claims.credential,
                }));
                return next.run(req).await;
            }
        }
    }
    if let Some(ref db) = state.db {
        tracing::debug!("Checking user {} for server {}", claims.uid, server);
        let user_result = sekai_user::Entity::find_by_id(&claims.uid).one(db).await;
        match user_result {
            Ok(Some(user)) => {
                if user.credential != claims.credential {
                    return error_response(StatusCode::UNAUTHORIZED, "Invalid credential");
                }
                tracing::debug!(
                    "Checking server authorization: user={}, server={}",
                    user.id,
                    server
                );
                let server_result = sekai_user_server::Entity::find()
                    .filter(sekai_user_server::Column::UserId.eq(&user.id))
                    .filter(sekai_user_server::Column::Server.eq(&server))
                    .one(db)
                    .await;
                match server_result {
                    Ok(Some(_)) => {
                        tracing::debug!("User {} authorized for server {}", user.id, server);
                        if let Some(ref redis) = state.redis {
                            let cache_key = format!("haruki_sekai_api:{}:{}", user.id, server);
                            let mut conn = redis.clone();
                            let _: Result<(), _> =
                                redis::AsyncCommands::set_ex(&mut conn, &cache_key, "1", 43200u64)
                                    .await;
                        }
                        req.extensions_mut().insert(Some(AuthUser {
                            id: claims.uid,
                            credential: claims.credential,
                        }));
                        return next.run(req).await;
                    }
                    Ok(None) => {
                        tracing::warn!("User {} not authorized for server {}", user.id, server);
                        return error_response(
                            StatusCode::FORBIDDEN,
                            "Not authorized for this server",
                        );
                    }
                    Err(e) => {
                        tracing::error!("Database error checking server auth: {:?}", e);
                        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
                    }
                }
            }
            Ok(None) => {
                tracing::warn!("User {} not found in database", claims.uid);
                return error_response(StatusCode::UNAUTHORIZED, "User not found");
            }
            Err(e) => {
                tracing::error!("Database error looking up user: {:?}", e);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
            }
        }
    }
    next.run(req).await
}

fn extract_server_from_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if !parts.is_empty() {
        parts[0].to_lowercase()
    } else {
        String::new()
    }
}

fn error_response(status: StatusCode, message: &str) -> Response {
    let body = ApiErrorResponse {
        result: "failed",
        status: status.as_u16(),
        message: message.to_string(),
    };
    let json = sonic_rs::to_string(&body).unwrap_or_else(|_| {
        r#"{"result":"failed","status":500,"message":"Internal error"}"#.to_string()
    });
    (status, [("content-type", "application/json")], json).into_response()
}
