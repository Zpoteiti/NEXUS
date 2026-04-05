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

use nexus_common::error::{ApiError, ErrorCode};

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
            let msg = e.to_string();
            if msg.contains("idx_device_tokens_user_device") || msg.contains("duplicate key") {
                ApiError::new(ErrorCode::Conflict, format!("device '{}' already exists", payload.device_name)).into_response()
            } else {
                tracing::error!("create device token error: {e}");
                ApiError::new(ErrorCode::InternalError, "failed to create device token").into_response()
            }
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
            ApiError::new(ErrorCode::InternalError, "failed to save discord config").into_response()
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
            ApiError::new(ErrorCode::InternalError, "failed to list tokens").into_response()
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
        Ok(true) => Json(json!({"message": "Token revoked"})).into_response(),
        Ok(false) => ApiError::new(ErrorCode::NotFound, "token not found or already revoked").into_response(),
        Err(e) => {
            tracing::error!("revoke_device_token error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to revoke token").into_response()
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
        Ok(None) => ApiError::new(ErrorCode::NotFound, "no discord config found").into_response(),
        Err(e) => {
            tracing::error!("get_discord_config error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to get config").into_response()
        }
    }
}

/// DELETE /api/discord-config — delete current user's Discord config
pub async fn delete_discord_config(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    match db::delete_discord_config(&state.db, &claims.sub).await {
        Ok(true) => Json(json!({"message": "Discord config deleted"})).into_response(),
        Ok(false) => ApiError::new(ErrorCode::NotFound, "no discord config found").into_response(),
        Err(e) => {
            tracing::error!("delete_discord_config error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to delete config").into_response()
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
            ApiError::new(ErrorCode::InternalError, "failed to list sessions").into_response()
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
            Json(json!({"message": "Session deleted"})).into_response()
        }
        Ok(false) => ApiError::new(ErrorCode::NotFound, "session not found").into_response(),
        Err(e) => {
            tracing::error!("delete_session error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to delete session").into_response()
        }
    }
}

/// GET /api/llm-config — get current LLM config (admin only, api_key masked)
pub async fn get_llm_config(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    if !claims.is_admin {
        return ApiError::new(ErrorCode::Forbidden, "admin access required").into_response();
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
                "provider": llm.provider,
                "model": llm.model,
                "api_key": masked_key,
                "api_base": llm.api_base,
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
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub api_base: Option<String>,
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
        return ApiError::new(ErrorCode::Forbidden, "admin access required").into_response();
    }
    let mut llm_guard = state.config.llm.write().await;
    let llm = llm_guard.get_or_insert_with(|| crate::config::LlmConfig {
        provider: String::new(),
        model: String::new(),
        api_key: String::new(),
        api_base: None,
        context_window: 204800,
        max_output_tokens: 131072,
    });
    if let Some(provider) = payload.provider {
        llm.provider = provider;
    }
    if let Some(model) = payload.model {
        llm.model = model;
    }
    if let Some(api_key) = payload.api_key {
        llm.api_key = api_key;
    }
    // api_base is optional — only set if explicitly provided
    if let Some(api_base) = payload.api_base {
        if api_base.is_empty() {
            llm.api_base = None;
        } else {
            llm.api_base = Some(api_base);
        }
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

    // Register updated model with LiteLLM proxy
    if !llm.provider.is_empty() && !llm.model.is_empty() && !llm.api_key.is_empty() {
        if let Err(e) = state.litellm.add_model(
            &llm.provider,
            &llm.model,
            &llm.api_key,
            llm.api_base.as_deref(),
        ).await {
            tracing::error!("Failed to register model with LiteLLM: {e}");
            return Json(json!({"message": "LLM config saved but LiteLLM registration failed", "error": e})).into_response();
        }
    }

    Json(json!({"message": "LLM config updated"})).into_response()
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
        Err(_) => ApiError::new(ErrorCode::NotFound, "device not found").into_response(),
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
        Ok(false) => ApiError::new(ErrorCode::NotFound, "device not found or revoked").into_response(),
        Err(e) => {
            tracing::error!("update_device_policy error: {e}");
            ApiError::new(ErrorCode::InternalError, "operation failed").into_response()
        },
    }
}

// ============================================================================
// Device MCP config handlers
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct UpdateDeviceMcpRequest {
    pub mcp_servers: Vec<nexus_common::protocol::McpServerEntry>,
}

#[derive(Debug, Serialize)]
pub struct DeviceMcpResponse {
    pub device_name: String,
    pub mcp_servers: Vec<nexus_common::protocol::McpServerEntry>,
}

/// GET /api/devices/{device_name}/mcp — get MCP config for a device
pub async fn get_device_mcp(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(device_name): Path<String>,
) -> Response {
    match db::get_device_mcp_config(&state.db, &claims.sub, &device_name).await {
        Ok(servers) => Json(DeviceMcpResponse {
            device_name,
            mcp_servers: servers,
        }).into_response(),
        Err(_) => ApiError::new(ErrorCode::NotFound, "device not found").into_response(),
    }
}

/// PUT /api/devices/{device_name}/mcp — update MCP config for a device
pub async fn update_device_mcp(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(device_name): Path<String>,
    Json(payload): Json<UpdateDeviceMcpRequest>,
) -> Response {
    match db::update_device_mcp_config(&state.db, &claims.sub, &device_name, &payload.mcp_servers).await {
        Ok(true) => Json(DeviceMcpResponse {
            device_name,
            mcp_servers: payload.mcp_servers,
        }).into_response(),
        Ok(false) => ApiError::new(ErrorCode::NotFound, "device not found or revoked").into_response(),
        Err(e) => {
            tracing::error!("update_device_mcp error: {e}");
            ApiError::new(ErrorCode::InternalError, "operation failed").into_response()
        },
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
        return ApiError::new(ErrorCode::Forbidden, "admin access required").into_response();
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
                "max_input_length": emb.max_input_length,
                "max_concurrency": emb.max_concurrency,
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
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    pub max_input_length: usize,
    pub max_concurrency: usize,
}

/// PUT /api/embedding-config — update embedding config at runtime (admin only).
/// Replaces the entire config and triggers background re-embedding of all memory chunks.
pub async fn update_embedding_config(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Json(payload): Json<UpdateEmbeddingConfigRequest>,
) -> Response {
    if !claims.is_admin {
        return ApiError::new(ErrorCode::Forbidden, "admin access required").into_response();
    }

    let new_config = crate::config::EmbeddingConfig {
        api_base: payload.api_base,
        api_key: payload.api_key,
        model: payload.model,
        max_input_length: payload.max_input_length,
        max_concurrency: payload.max_concurrency,
    };

    // Update config in memory
    *state.config.embedding.write().await = Some(new_config.clone());

    // Persist to database
    if let Ok(json_str) = serde_json::to_string(&new_config) {
        if let Err(e) = db::set_system_config(&state.db, "embedding_config", &json_str).await {
            tracing::error!("Failed to persist embedding config to DB: {e}");
        }
    }

    // Spawn background re-embed task
    let db = state.db.clone();
    let emb_config = new_config;
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(emb_config.max_concurrency));

    tokio::spawn(async move {
        tracing::info!("re-embed: starting background re-embedding of all memory chunks");

        if let Err(e) = db::clear_all_embeddings(&db).await {
            tracing::error!("re-embed: failed to clear embeddings: {}", e);
            return;
        }

        let chunks = match db::get_all_memory_chunks_for_reembed(&db).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("re-embed: failed to fetch chunks: {}", e);
                return;
            }
        };

        tracing::info!("re-embed: processing {} chunks", chunks.len());
        let mut success = 0u32;
        let mut truncated_count = 0u32;
        let mut failed = 0u32;

        for chunk in &chunks {
            let db::MemoryChunkForReembed { id, ref memory_text } = *chunk;
            // Truncate if needed (rough estimate: 1 token ~ 3 chars)
            let max_chars = emb_config.max_input_length * 3;
            let (text_to_embed, is_truncated) = if memory_text.len() > max_chars {
                (&memory_text[..max_chars], true)
            } else {
                (memory_text.as_str(), false)
            };

            let embedding = crate::context::embed_text_throttled(&emb_config, text_to_embed, &semaphore).await;
            if embedding.is_empty() {
                tracing::warn!("re-embed: failed to embed chunk id={}", id);
                failed += 1;
                continue;
            }

            if let Err(e) = db::update_memory_embedding(&db, id, &embedding, is_truncated).await {
                tracing::warn!("re-embed: failed to update chunk id={}: {}", id, e);
                failed += 1;
            } else {
                success += 1;
                if is_truncated { truncated_count += 1; }
            }
        }

        tracing::info!("re-embed: done. {} success ({} truncated), {} failed", success, truncated_count, failed);
    });

    Json(json!({"message": "Embedding config updated. Re-embedding started in background."})).into_response()
}

// ============================================================================
// Server MCP config (admin only)
// ============================================================================

/// GET /api/server-mcp
pub async fn get_server_mcp(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    if !claims.is_admin {
        return ApiError::new(ErrorCode::Forbidden, "admin access required").into_response();
    }
    match crate::db::get_system_config(&state.db, "server_mcp_config").await {
        Ok(Some(json)) => {
            match serde_json::from_str::<Vec<nexus_common::protocol::McpServerEntry>>(&json) {
                Ok(entries) => Json(serde_json::json!({"mcp_servers": entries})).into_response(),
                Err(_) => Json(serde_json::json!({"mcp_servers": []})).into_response(),
            }
        }
        Ok(None) => Json(serde_json::json!({"mcp_servers": []})).into_response(),
        Err(e) => {
            tracing::error!("get_server_mcp error: {e}");
            ApiError::new(ErrorCode::InternalError, "operation failed").into_response()
        },
    }
}

/// PUT /api/server-mcp
pub async fn update_server_mcp(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Json(payload): Json<crate::auth::UpdateDeviceMcpRequest>,
) -> Response {
    if !claims.is_admin {
        return ApiError::new(ErrorCode::Forbidden, "admin access required").into_response();
    }
    let json = match serde_json::to_string(&payload.mcp_servers) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!("update_server_mcp json error: {e}");
            return ApiError::new(ErrorCode::ValidationFailed, "invalid json payload").into_response();
        }
    };

    if let Err(e) = crate::db::set_system_config(&state.db, "server_mcp_config", &json).await {
        tracing::error!("update_server_mcp db error: {e}");
        return ApiError::new(ErrorCode::InternalError, "operation failed").into_response();
    }

    // Reinitialize server MCP manager with new config
    let mut manager = state.server_mcp.write().await;
    manager.initialize(&payload.mcp_servers).await;

    Json(serde_json::json!({"mcp_servers": payload.mcp_servers})).into_response()
}

// ============================================================================
// Skills API handlers
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateSkillRequest {
    pub name: String,
    pub content: String,
}

/// GET /api/skills — list current user's skills (metadata only)
pub async fn list_skills(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    match db::list_skills(&state.db, &claims.sub).await {
        Ok(skills) => Json(json!({ "skills": skills })).into_response(),
        Err(e) => {
            tracing::error!("list_skills error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to list skills").into_response()
        }
    }
}

/// POST /api/skills — create or update a skill
pub async fn create_skill(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Json(payload): Json<CreateSkillRequest>,
) -> Response {
    let user_id = &claims.sub;

    // Validate name: alphanumeric, hyphens, underscores only
    if payload.name.is_empty() || !payload.name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return ApiError::new(ErrorCode::ValidationFailed, "invalid skill name: must be non-empty and contain only alphanumeric, hyphens, or underscores").into_response();
    }

    // Parse frontmatter
    let (fm_name, description, always_on) = crate::server_tools::skills::parse_frontmatter(&payload.content);
    let skill_name = fm_name.unwrap_or_else(|| payload.name.clone());

    // Create skill directory: skills_dir / user_id / name /
    let skill_dir = std::path::PathBuf::from(&state.config.skills_dir)
        .join(user_id)
        .join(&payload.name);

    if let Err(e) = tokio::fs::create_dir_all(&skill_dir).await {
        tracing::error!("create_skill: failed to create directory {:?}: {}", skill_dir, e);
        return ApiError::new(ErrorCode::InternalError, "failed to create skill directory").into_response();
    }

    // Write SKILL.md
    let skill_md_path = skill_dir.join("SKILL.md");
    if let Err(e) = tokio::fs::write(&skill_md_path, &payload.content).await {
        tracing::error!("create_skill: failed to write SKILL.md: {}", e);
        return ApiError::new(ErrorCode::InternalError, "failed to write skill file").into_response();
    }

    let skill_path = skill_dir.to_string_lossy().to_string();

    // Insert/update in DB
    match db::create_skill(&state.db, user_id, &skill_name, &description, always_on, &skill_path).await {
        Ok(skill_id) => Json(json!({
            "skill_id": skill_id,
            "name": skill_name,
            "description": description,
            "always_on": always_on,
        })).into_response(),
        Err(e) => {
            tracing::error!("create_skill db error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to save skill metadata").into_response()
        }
    }
}

/// DELETE /api/skills/{name} — remove a skill
pub async fn delete_skill(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(name): Path<String>,
) -> Response {
    let user_id = &claims.sub;

    // Look up skill to get filesystem path before deleting from DB
    let skill = match db::get_skill(&state.db, user_id, &name).await {
        Ok(Some(s)) => Some(s),
        Ok(None) => None,
        Err(e) => {
            tracing::error!("delete_skill lookup error: {e}");
            return ApiError::new(ErrorCode::InternalError, "failed to look up skill").into_response();
        }
    };

    match db::delete_skill(&state.db, user_id, &name).await {
        Ok(true) => {
            // Clean up filesystem
            if let Some(skill) = skill {
                if let Err(e) = tokio::fs::remove_dir_all(&skill.skill_path).await {
                    tracing::warn!("delete_skill: failed to remove directory {}: {}", skill.skill_path, e);
                }
            }
            Json(json!({"message": "Skill deleted"})).into_response()
        }
        Ok(false) => ApiError::new(ErrorCode::NotFound, "skill not found").into_response(),
        Err(e) => {
            tracing::error!("delete_skill error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to delete skill").into_response()
        }
    }
}

// ============================================================================
// Cron REST API handlers
// ============================================================================

/// GET /api/cron-jobs — list current user's cron jobs
pub async fn list_cron_jobs_api(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    match db::list_cron_jobs(&state.db, &claims.sub).await {
        Ok(jobs) => Json(json!({ "cron_jobs": jobs })).into_response(),
        Err(e) => {
            tracing::error!("list_cron_jobs_api error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to list cron jobs").into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateCronJobRequest {
    pub message: String,
    pub cron_expr: Option<String>,
    pub every_seconds: Option<i32>,
    pub at: Option<String>,
    pub timezone: Option<String>,
    pub channel: String,
    pub chat_id: String,
    #[serde(default)]
    pub delete_after_run: bool,
    pub name: Option<String>,
}

/// POST /api/cron-jobs — create a cron job
pub async fn create_cron_job_api(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Json(payload): Json<CreateCronJobRequest>,
) -> Response {
    let name = payload.name.as_deref().unwrap_or("api-job");
    let timezone = payload.timezone.as_deref().unwrap_or("UTC");

    match db::create_cron_job(
        &state.db,
        &claims.sub,
        name,
        payload.cron_expr.as_deref(),
        payload.every_seconds,
        payload.at.as_deref(),
        timezone,
        &payload.message,
        &payload.channel,
        &payload.chat_id,
        payload.delete_after_run,
    )
    .await
    {
        Ok(job_id) => (StatusCode::CREATED, Json(json!({ "job_id": job_id }))).into_response(),
        Err(e) => {
            tracing::error!("create_cron_job_api error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to create cron job").into_response()
        }
    }
}

/// DELETE /api/cron-jobs/{job_id} — delete a cron job
pub async fn delete_cron_job_api(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(job_id): Path<String>,
) -> Response {
    match db::delete_cron_job(&state.db, &claims.sub, &job_id).await {
        Ok(true) => Json(json!({"message": "Cron job deleted"})).into_response(),
        Ok(false) => ApiError::new(ErrorCode::NotFound, "cron job not found").into_response(),
        Err(e) => {
            tracing::error!("delete_cron_job_api error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to delete cron job").into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateCronJobRequest {
    pub enabled: Option<bool>,
    pub message: Option<String>,
}

/// PATCH /api/cron-jobs/{job_id} — update a cron job (enable/disable, change message)
pub async fn update_cron_job_api(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(job_id): Path<String>,
    Json(payload): Json<UpdateCronJobRequest>,
) -> Response {
    match db::update_cron_job(
        &state.db,
        &claims.sub,
        &job_id,
        payload.enabled,
        payload.message.as_deref(),
    )
    .await
    {
        Ok(true) => Json(json!({"message": "Cron job updated"})).into_response(),
        Ok(false) => Json(json!({"message": "No changes applied"})).into_response(),
        Err(e) => {
            tracing::error!("update_cron_job_api error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to update cron job").into_response()
        }
    }
}

/// GET /api/admin/skills — list all skills across all users (admin only)
pub async fn admin_list_skills(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    if !claims.is_admin {
        return ApiError::new(ErrorCode::Forbidden, "admin access required").into_response();
    }
    match db::list_all_skills(&state.db).await {
        Ok(skills) => Json(json!({ "skills": skills })).into_response(),
        Err(e) => {
            tracing::error!("admin_list_skills error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to list skills").into_response()
        }
    }
}
