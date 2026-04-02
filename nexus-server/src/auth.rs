/// Authentication module: register, login, JWT sign/verify, and Axum middleware.
/// POST /api/auth/register → register()
/// POST /api/auth/login    → login()

use axum::{
    extract::{Json, Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use serde_json::json;

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

    match db::create_device_token(&state.db, &token, user_id, &payload.device_name).await {
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
    /// Discord user ID of the bot owner (for sender identity verification)
    pub owner_discord_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DiscordConfigResponse {
    pub user_id: String,
    pub enabled: bool,
    pub allowed_users: Vec<String>,
    pub owner_discord_id: Option<String>,
}

pub async fn upsert_discord_config(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Json(payload): Json<UpsertDiscordConfigRequest>,
) -> Response {
    let user_id = &claims.sub;

    match db::upsert_discord_config(&state.db, user_id, &payload.bot_token, &payload.allowed_users, payload.owner_discord_id.as_deref())
        .await
    {
        Ok(_) => Json(DiscordConfigResponse {
            user_id: user_id.clone(),
            enabled: true,
            allowed_users: payload.allowed_users,
            owner_discord_id: payload.owner_discord_id,
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

// ============================================================================
// Admin API handlers
// ============================================================================

/// GET /api/device-tokens — list all device tokens for current user
pub async fn list_device_tokens(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    match db::list_device_tokens(&state.db, &claims.sub).await {
        Ok(tokens) => Json(tokens).into_response(),
        Err(e) => {
            tracing::error!("list_device_tokens error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to list tokens").into_response()
        }
    }
}

/// DELETE /api/device-tokens/:token — revoke a device token
pub async fn revoke_device_token(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(token): Path<String>,
) -> Response {
    match db::revoke_device_token(&state.db, &token, &claims.sub).await {
        Ok(true) => (StatusCode::OK, "Token revoked").into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "Token not found or already revoked").into_response(),
        Err(e) => {
            tracing::error!("revoke_device_token error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to revoke token").into_response()
        }
    }
}

/// GET /api/discord-config — get current user's Discord config
pub async fn get_discord_config(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    match db::get_discord_config_by_user_id(&state.db, &claims.sub).await {
        Ok(Some(config)) => Json(json!({
            "user_id": config.user_id,
            "bot_user_id": config.bot_user_id,
            "enabled": config.enabled,
            "allowed_users": config.allowed_users,
        })).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "No Discord config found").into_response(),
        Err(e) => {
            tracing::error!("get_discord_config error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to get config").into_response()
        }
    }
}

/// DELETE /api/discord-config — delete current user's Discord config
pub async fn delete_discord_config(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    match db::delete_discord_config(&state.db, &claims.sub).await {
        Ok(true) => (StatusCode::OK, "Discord config deleted").into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "No Discord config found").into_response(),
        Err(e) => {
            tracing::error!("delete_discord_config error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete config").into_response()
        }
    }
}

/// GET /api/sessions — list current user's sessions
pub async fn list_sessions(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    match db::list_sessions_by_user(&state.db, &claims.sub).await {
        Ok(sessions) => Json(sessions).into_response(),
        Err(e) => {
            tracing::error!("list_sessions error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to list sessions").into_response()
        }
    }
}

/// DELETE /api/sessions/:session_id — delete a session and its messages
pub async fn delete_session(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(session_id): Path<String>,
) -> Response {
    match db::delete_session(&state.db, &session_id, &claims.sub).await {
        Ok(true) => {
            // Clean up in-memory session if active
            state.bus.unregister_session(&session_id);
            state.session_manager.remove_session(&session_id).await;
            (StatusCode::OK, "Session deleted").into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, "Session not found").into_response(),
        Err(e) => {
            tracing::error!("delete_session error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete session").into_response()
        }
    }
}

/// GET /api/llm-config — get current LLM config (admin only, api_key masked)
pub async fn get_llm_config(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    if !claims.is_admin {
        return (StatusCode::FORBIDDEN, "Admin only").into_response();
    }
    let llm = state.config.llm.read().await;
    match llm.as_ref() {
        Some(llm) => {
            let masked_key = if llm.api_key.len() > 12 {
                format!("{}...{}", &llm.api_key[..8], &llm.api_key[llm.api_key.len()-4..])
            } else {
                "***".to_string()
            };
            Json(json!({
                "api_base": llm.api_base,
                "api_key": masked_key,
                "model": llm.model,
                "context_window": llm.context_window,
                "max_output_tokens": llm.max_output_tokens,
            })).into_response()
        }
        None => {
            Json(json!({
                "status": "not_configured",
                "message": "LLM provider has not been configured yet. Use PUT /api/llm-config to set it up.",
            })).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateLlmConfigRequest {
    pub api_base: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub context_window: Option<usize>,
    pub max_output_tokens: Option<usize>,
}

/// PUT /api/llm-config — update LLM config at runtime (admin only)
pub async fn update_llm_config(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Json(payload): Json<UpdateLlmConfigRequest>,
) -> Response {
    if !claims.is_admin {
        return (StatusCode::FORBIDDEN, "Admin only").into_response();
    }
    let mut llm_guard = state.config.llm.write().await;
    let llm = llm_guard.get_or_insert_with(|| crate::config::LlmConfig {
        api_base: String::new(),
        api_key: String::new(),
        model: String::new(),
        context_window: 204800,
        max_output_tokens: 131072,
    });
    if let Some(api_base) = payload.api_base {
        llm.api_base = api_base;
    }
    if let Some(api_key) = payload.api_key {
        llm.api_key = api_key;
    }
    if let Some(model) = payload.model {
        llm.model = model;
    }
    if let Some(context_window) = payload.context_window {
        llm.context_window = context_window;
    }
    if let Some(max_output_tokens) = payload.max_output_tokens {
        llm.max_output_tokens = max_output_tokens;
    }

    // Persist to database
    if let Ok(json_str) = serde_json::to_string(llm) {
        if let Err(e) = db::set_system_config(&state.db, "llm_config", &json_str).await {
            tracing::error!("Failed to persist LLM config to DB: {e}");
        }
    }

    (StatusCode::OK, "LLM config updated").into_response()
}

// ============================================================================
// Device policy handlers
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct UpdateDevicePolicyRequest {
    pub fs_policy: nexus_common::protocol::FsPolicy,
}

#[derive(Debug, Serialize)]
pub struct DevicePolicyResponse {
    pub device_name: String,
    pub fs_policy: nexus_common::protocol::FsPolicy,
}

/// GET /api/devices/{device_name}/policy — get the fs_policy for a device
pub async fn get_device_policy(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(device_name): Path<String>,
) -> Response {
    match db::get_device_policy(&state.db, &claims.sub, &device_name).await {
        Ok(policy) => Json(DevicePolicyResponse {
            device_name,
            fs_policy: policy,
        })
        .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "Device not found").into_response(),
    }
}

/// PATCH /api/devices/{device_name}/policy — update the fs_policy for a device
pub async fn update_device_policy(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(device_name): Path<String>,
    Json(payload): Json<UpdateDevicePolicyRequest>,
) -> Response {
    match db::update_device_policy(&state.db, &claims.sub, &device_name, &payload.fs_policy).await {
        Ok(true) => Json(DevicePolicyResponse {
            device_name,
            fs_policy: payload.fs_policy,
        })
        .into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "Device not found or revoked").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {e}")).into_response(),
    }
}

// ============================================================================
// Embedding config handlers
// ============================================================================

/// GET /api/embedding-config — get current embedding config (admin only, api_key masked)
pub async fn get_embedding_config(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    if !claims.is_admin {
        return (StatusCode::FORBIDDEN, "Admin only").into_response();
    }
    let emb = state.config.embedding.read().await;
    match emb.as_ref() {
        Some(emb) => {
            let masked_key = if emb.api_key.len() > 12 {
                format!("{}...{}", &emb.api_key[..8], &emb.api_key[emb.api_key.len()-4..])
            } else {
                "***".to_string()
            };
            Json(json!({
                "api_base": emb.api_base,
                "api_key": masked_key,
                "model": emb.model,
                "dimensions": emb.dimensions,
            })).into_response()
        }
        None => {
            Json(json!({
                "status": "not_configured",
                "message": "Embedding provider has not been configured yet. Use PUT /api/embedding-config to set it up.",
            })).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateEmbeddingConfigRequest {
    pub api_base: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub dimensions: Option<usize>,
}

/// PUT /api/embedding-config — update embedding config at runtime (admin only)
pub async fn update_embedding_config(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Json(payload): Json<UpdateEmbeddingConfigRequest>,
) -> Response {
    if !claims.is_admin {
        return (StatusCode::FORBIDDEN, "Admin only").into_response();
    }
    let mut emb_guard = state.config.embedding.write().await;
    let emb = emb_guard.get_or_insert_with(|| crate::config::EmbeddingConfig {
        api_base: String::new(),
        api_key: String::new(),
        model: String::new(),
        dimensions: 1536,
    });
    if let Some(api_base) = payload.api_base {
        emb.api_base = api_base;
    }
    if let Some(api_key) = payload.api_key {
        emb.api_key = api_key;
    }
    if let Some(model) = payload.model {
        emb.model = model;
    }
    if let Some(dimensions) = payload.dimensions {
        emb.dimensions = dimensions;
    }

    // Persist to database
    if let Ok(json_str) = serde_json::to_string(emb) {
        if let Err(e) = db::set_system_config(&state.db, "embedding_config", &json_str).await {
            tracing::error!("Failed to persist embedding config to DB: {e}");
        }
    }

    (StatusCode::OK, "Embedding config updated").into_response()
}
