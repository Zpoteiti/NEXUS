//! Authentication: JWT sign/verify, register/login handlers.

pub mod device;

use crate::state::AppState;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use axum::{Json, Router};
use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use nexus_common::consts::{BCRYPT_COST, JWT_EXPIRY_DAYS};
use nexus_common::error::{ApiError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub is_admin: bool,
    pub exp: i64,
}

pub fn sign_jwt(user_id: &str, is_admin: bool, secret: &str) -> String {
    let exp = Utc::now().timestamp() + JWT_EXPIRY_DAYS * 86400;
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
    .expect("JWT encoding failed")
}

pub fn verify_jwt(token: &str, secret: &str) -> Result<Claims, ApiError> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map(|data| data.claims)
    .map_err(|e| ApiError::new(ErrorCode::AuthFailed, format!("Invalid token: {e}")))
}

pub fn extract_claims(headers: &HeaderMap, secret: &str) -> Result<Claims, ApiError> {
    let header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::new(ErrorCode::Unauthorized, "Missing Authorization header"))?;
    let token = header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::new(ErrorCode::Unauthorized, "Invalid Authorization format"))?;
    verify_jwt(token, secret)
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub admin_token: Option<String>,
}

#[derive(Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user_id: String,
    pub is_admin: bool,
}

pub async fn register(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    let is_admin = req
        .admin_token
        .as_deref()
        .map(|t| t == state.config.admin_token)
        .unwrap_or(false);

    let password_hash = bcrypt::hash(&req.password, BCRYPT_COST)
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("Hash error: {e}")))?;

    let user_id = uuid::Uuid::new_v4().to_string();

    crate::db::users::create_user(&state.db, &user_id, &req.email, &password_hash, is_admin)
        .await
        .map_err(|e| {
            if e.to_string().contains("duplicate") || e.to_string().contains("unique") {
                ApiError::new(ErrorCode::Conflict, "Email already registered")
            } else {
                ApiError::new(ErrorCode::InternalError, format!("DB error: {e}"))
            }
        })?;

    let token = sign_jwt(&user_id, is_admin, &state.config.jwt_secret);
    info!("User registered: {}", req.email);

    Ok(Json(AuthResponse {
        token,
        user_id,
        is_admin,
    }))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    let user = crate::db::users::find_by_email(&state.db, &req.email)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("DB error: {e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::AuthFailed, "Invalid credentials"))?;

    let valid = bcrypt::verify(&req.password, &user.password_hash).unwrap_or(false);
    if !valid {
        return Err(ApiError::new(ErrorCode::AuthFailed, "Invalid credentials"));
    }

    let token = sign_jwt(&user.user_id, user.is_admin, &state.config.jwt_secret);

    Ok(Json(AuthResponse {
        token,
        user_id: user.user_id,
        is_admin: user.is_admin,
    }))
}

pub fn auth_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
}
