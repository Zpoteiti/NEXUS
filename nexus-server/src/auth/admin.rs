/// Admin-only handlers: LLM config, server MCP.

use axum::{
    extract::{Json, State},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;

use nexus_common::error::{ApiError, ErrorCode};

use super::Claims;
use super::device::UpdateDeviceMcpRequest;
use crate::db;
use crate::state::AppState;

/// GET /api/llm-config -- get current LLM config (admin only, api_key masked)
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

/// PUT /api/llm-config -- update LLM config at runtime (admin only)
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
    // api_base is optional -- only set if explicitly provided
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
    Json(payload): Json<UpdateDeviceMcpRequest>,
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
