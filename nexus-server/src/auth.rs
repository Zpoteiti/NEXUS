/// Authentication module: register, login, JWT sign/verify, and Axum middleware.
/// POST /api/auth/register → register()
/// POST /api/auth/login    → login()

use axum::{
    extract::{Json, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use crate::db;
use crate::state::AppState;

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
            return (StatusCode::INTERNAL_SERVER_ERROR, "Password hashing failed").into_response();
        }
        Err(e) => {
            tracing::error!("spawn_blocking join error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
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
                (StatusCode::INTERNAL_SERVER_ERROR, "Failed to sign token").into_response()
            }
        },
        Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => {
            (StatusCode::CONFLICT, "Email already registered").into_response()
        }
        Err(e) => {
            tracing::error!("create_user error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Registration failed").into_response()
        }
    }
}

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Response {
    let user = match db::get_user_by_email(&state.db, &payload.email).await {
        Ok(Some(u)) => u,
        Ok(None) => return (StatusCode::UNAUTHORIZED, "Invalid credentials").into_response(),
        Err(e) => {
            tracing::error!("get_user_by_email error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Login failed").into_response();
        }
    };

    let password = payload.password.clone();
    let hash = user.password_hash.clone();
    let verify_result = tokio::task::spawn_blocking(move || bcrypt::verify(password, &hash)).await;

    let valid = match verify_result {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            tracing::error!("bcrypt verify error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Login failed").into_response();
        }
        Err(e) => {
            tracing::error!("spawn_blocking join error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
        }
    };

    if !valid {
        return (StatusCode::UNAUTHORIZED, "Invalid credentials").into_response();
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
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to sign token").into_response()
        }
    }
}

// ============================================================================
// Protected API handlers (require JWT)
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateDeviceTokenRequest {
    pub device_name: String,
}

#[derive(Debug, Serialize)]
pub struct DeviceTokenResponse {
    pub token: String,
    pub device_name: String,
}

pub async fn create_device_token(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Json(payload): Json<CreateDeviceTokenRequest>,
) -> Response {
    let user_id = &claims.sub;

    // Generate token: nexus_dev_ + 32 random hex chars
    let random_part: String = uuid::Uuid::new_v4().to_string().replace("-", "");
    let token = format!(
        "{}{}",
        nexus_common::consts::DEVICE_TOKEN_PREFIX,
        random_part
    );

    match sqlx::query(
        "INSERT INTO device_tokens (token, user_id, device_name) VALUES ($1, $2, $3)",
    )
    .bind(&token)
    .bind(user_id)
    .bind(&payload.device_name)
    .execute(&state.db)
    .await
    {
        Ok(_) => Json(DeviceTokenResponse {
            token,
            device_name: payload.device_name,
        })
        .into_response(),
        Err(e) => {
            tracing::error!("create device token error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create device token").into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpsertDiscordConfigRequest {
    pub bot_token: String,
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DiscordConfigResponse {
    pub user_id: String,
    pub enabled: bool,
    pub allowed_users: Vec<String>,
}

pub async fn upsert_discord_config(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Json(payload): Json<UpsertDiscordConfigRequest>,
) -> Response {
    let user_id = &claims.sub;

    match db::upsert_discord_config(&state.db, user_id, &payload.bot_token, &payload.allowed_users)
        .await
    {
        Ok(_) => Json(DiscordConfigResponse {
            user_id: user_id.clone(),
            enabled: true,
            allowed_users: payload.allowed_users,
        })
        .into_response(),
        Err(e) => {
            tracing::error!("upsert discord config error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save discord config").into_response()
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
                (StatusCode::UNAUTHORIZED, "Invalid or expired token").into_response()
            }
        },
        None => (StatusCode::UNAUTHORIZED, "Missing Authorization header").into_response(),
    }
}
