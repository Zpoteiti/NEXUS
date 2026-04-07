/// Authentication module: register, login, JWT sign/verify, and Axum middleware.
/// POST /api/auth/register -> register()
/// POST /api/auth/login    -> login()

mod admin;
mod cron_api;
mod device;
mod discord_config;
mod skills_api;

use axum::{
    extract::{Json, Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use nexus_common::error::{ApiError, ErrorCode};

use crate::db;
use crate::state::AppState;

// Re-export everything so `crate::auth::handler_name` continues to work.
pub use admin::*;
pub use cron_api::*;
pub use device::*;
pub use discord_config::*;
pub use skills_api::*;

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub admin_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user_id: String,
    pub is_admin: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub is_admin: bool,
    pub exp: usize,
}

pub fn verify_jwt(token: &str, secret: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    let key = DecodingKey::from_secret(secret.as_bytes());
    let data = decode::<Claims>(token, &key, &Validation::default())?;
    Ok(data.claims)
}

fn sign_jwt(
    user_id: &str,
    is_admin: bool,
    secret: &str,
) -> Result<String, jsonwebtoken::errors::Error> {
    let exp = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::days(7))
        .unwrap()
        .timestamp() as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        is_admin,
        exp,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> Response {
    let is_admin = payload
        .admin_token
        .as_deref()
        .map(|t| t == state.config.admin_token)
        .unwrap_or(false);

    let cost = state.config.bcrypt_cost;
    let password = payload.password.clone();
    let hash_result = tokio::task::spawn_blocking(move || bcrypt::hash(password, cost)).await;

    let password_hash = match hash_result {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => {
            tracing::error!("bcrypt hash error: {e}");
            return ApiError::new(ErrorCode::InternalError, "password hashing failed").into_response();
        }
        Err(e) => {
            tracing::error!("spawn_blocking join error: {e}");
            return ApiError::new(ErrorCode::InternalError, "internal error").into_response();
        }
    };

    match db::create_user(&state.db, &payload.email, &password_hash, is_admin).await {
        Ok(user_id) => match sign_jwt(&user_id, is_admin, &state.config.jwt_secret) {
            Ok(token) => Json(AuthResponse {
                token,
                user_id,
                is_admin,
            })
            .into_response(),
            Err(e) => {
                tracing::error!("JWT sign error: {e}");
                ApiError::new(ErrorCode::InternalError, "failed to sign token").into_response()
            }
        },
        Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => {
            ApiError::new(ErrorCode::Conflict, "email already registered").into_response()
        }
        Err(e) => {
            tracing::error!("create_user error: {e}");
            ApiError::new(ErrorCode::InternalError, "registration failed").into_response()
        }
    }
}

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Response {
    let user = match db::get_user_by_email(&state.db, &payload.email).await {
        Ok(Some(u)) => u,
        Ok(None) => return ApiError::new(ErrorCode::Unauthorized, "invalid credentials").into_response(),
        Err(e) => {
            tracing::error!("get_user_by_email error: {e}");
            return ApiError::new(ErrorCode::InternalError, "login failed").into_response();
        }
    };

    let password = payload.password.clone();
    let hash = user.password_hash.clone();
    let verify_result = tokio::task::spawn_blocking(move || bcrypt::verify(password, &hash)).await;

    let valid = match verify_result {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            tracing::error!("bcrypt verify error: {e}");
            return ApiError::new(ErrorCode::InternalError, "login failed").into_response();
        }
        Err(e) => {
            tracing::error!("spawn_blocking join error: {e}");
            return ApiError::new(ErrorCode::InternalError, "internal error").into_response();
        }
    };

    if !valid {
        return ApiError::new(ErrorCode::Unauthorized, "invalid credentials").into_response();
    }

    match sign_jwt(&user.user_id, user.is_admin, &state.config.jwt_secret) {
        Ok(token) => Json(AuthResponse {
            token,
            user_id: user.user_id,
            is_admin: user.is_admin,
        })
        .into_response(),
        Err(e) => {
            tracing::error!("JWT sign error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to sign token").into_response()
        }
    }
}

// ============================================================================
// JWT Middleware
// ============================================================================

pub async fn jwt_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match token {
        Some(token) => match verify_jwt(token, &state.config.jwt_secret) {
            Ok(claims) => {
                req.extensions_mut().insert(claims);
                next.run(req).await
            }
            Err(e) => {
                tracing::debug!("JWT verification failed: {e}");
                ApiError::new(ErrorCode::Unauthorized, "invalid or expired token").into_response()
            }
        },
        None => ApiError::new(ErrorCode::Unauthorized, "missing authorization header").into_response(),
    }
}

/// GET /api/sessions -- list current user's sessions
pub async fn list_sessions(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    match db::list_sessions_by_user(&state.db, &claims.sub).await {
        Ok(sessions) => Json(sessions).into_response(),
        Err(e) => {
            tracing::error!("list_sessions error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to list sessions").into_response()
        }
    }
}

/// DELETE /api/sessions/:session_id -- delete a session and its messages
pub async fn delete_session(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    axum::extract::Path(session_id): axum::extract::Path<String>,
) -> Response {
    match db::delete_session(&state.db, &session_id, &claims.sub).await {
        Ok(true) => {
            // Clean up in-memory session if active
            state.bus.unregister_session(&session_id);
            state.session_manager.remove_session(&session_id).await;
            Json(serde_json::json!({"message": "Session deleted"})).into_response()
        }
        Ok(false) => ApiError::new(ErrorCode::NotFound, "session not found").into_response(),
        Err(e) => {
            tracing::error!("delete_session error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to delete session").into_response()
        }
    }
}
