/// Admin-only handlers: LLM config, embedding config, server MCP.

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
// Embedding config handlers
// ============================================================================

/// GET /api/embedding-config -- get current embedding config (admin only, api_key masked)
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

/// PUT /api/embedding-config -- update embedding config at runtime (admin only).
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
