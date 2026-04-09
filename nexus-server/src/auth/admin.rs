//! Admin-only endpoints: default soul, rate limit, LLM config.

use crate::auth::extract_claims;
use crate::config::LlmConfig;
use crate::state::AppState;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, put};
use axum::{Json, Router};
use nexus_common::error::{ApiError, ErrorCode};
use serde::Deserialize;
use std::sync::Arc;

fn admin_claims(headers: &HeaderMap, state: &AppState) -> Result<crate::auth::Claims, ApiError> {
    let c = extract_claims(headers, &state.config.jwt_secret)?;
    if !c.is_admin {
        return Err(ApiError::new(ErrorCode::Forbidden, "Admin access required"));
    }
    Ok(c)
}

// -- Default Soul --

async fn get_default_soul(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_claims(&headers, &state)?;
    let soul = state.default_soul.read().await.clone();
    Ok(Json(serde_json::json!({ "default_soul": soul })))
}

#[derive(Deserialize)]
struct SoulUpdate {
    soul: String,
}

async fn put_default_soul(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<SoulUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_claims(&headers, &state)?;
    crate::db::system_config::set(&state.db, "default_soul", &req.soul)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    *state.default_soul.write().await = Some(req.soul);
    Ok(Json(
        serde_json::json!({ "message": "Default soul updated" }),
    ))
}

// -- Rate Limit --

async fn get_rate_limit(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_claims(&headers, &state)?;
    let limit = *state.rate_limit_config.read().await;
    Ok(Json(serde_json::json!({ "rate_limit_per_min": limit })))
}

#[derive(Deserialize)]
struct RateLimitUpdate {
    rate_limit_per_min: u32,
}

async fn put_rate_limit(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<RateLimitUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_claims(&headers, &state)?;
    crate::db::system_config::set(
        &state.db,
        "rate_limit_per_min",
        &req.rate_limit_per_min.to_string(),
    )
    .await
    .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    *state.rate_limit_config.write().await = req.rate_limit_per_min;
    Ok(Json(serde_json::json!({
        "message": "Rate limit updated",
        "rate_limit_per_min": req.rate_limit_per_min,
    })))
}

// -- LLM Config --

async fn get_llm_config(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_claims(&headers, &state)?;
    let config = state.llm_config.read().await;
    match config.as_ref() {
        Some(c) => {
            let masked_key = if c.api_key.len() > 8 {
                format!(
                    "{}...{}",
                    &c.api_key[..4],
                    &c.api_key[c.api_key.len() - 4..]
                )
            } else {
                "***".into()
            };
            Ok(Json(serde_json::json!({
                "api_base": c.api_base,
                "model": c.model,
                "api_key": masked_key,
                "context_window": c.context_window,
            })))
        }
        None => Ok(Json(serde_json::json!({
            "status": "not_configured",
            "message": "LLM config not set. Use PUT /api/llm-config to configure.",
        }))),
    }
}

#[derive(Deserialize)]
struct LlmConfigUpdate {
    api_base: String,
    model: Option<String>,
    api_key: Option<String>,
    context_window: Option<u32>,
}

async fn put_llm_config(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<LlmConfigUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_claims(&headers, &state)?;

    let current = state.llm_config.read().await.clone();
    let new_config = LlmConfig {
        api_base: req.api_base,
        model: req
            .model
            .or(current.as_ref().map(|c| c.model.clone()))
            .unwrap_or_else(|| "gpt-4o".into()),
        api_key: req
            .api_key
            .or(current.as_ref().map(|c| c.api_key.clone()))
            .unwrap_or_default(),
        context_window: req
            .context_window
            .or(current.as_ref().map(|c| c.context_window))
            .unwrap_or(204_800),
    };

    let json = serde_json::to_string(&new_config)
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    crate::db::system_config::set(&state.db, "llm_config", &json)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    *state.llm_config.write().await = Some(new_config);

    Ok(Json(serde_json::json!({ "message": "LLM config updated" })))
}

// -- Admin Skills (list all) --

async fn get_all_skills(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_claims(&headers, &state)?;
    let skills = crate::db::skills::list_all(&state.db)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(serde_json::json!({ "skills": skills })))
}

pub fn admin_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/admin/default-soul",
            get(get_default_soul).put(put_default_soul),
        )
        .route(
            "/api/admin/rate-limit",
            get(get_rate_limit).put(put_rate_limit),
        )
        .route("/api/llm-config", get(get_llm_config).put(put_llm_config))
        .route("/api/admin/skills", get(get_all_skills))
}
