//! Telegram config CRUD API endpoints.

use crate::auth::extract_claims;
use crate::state::AppState;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use nexus_common::error::{ApiError, ErrorCode};
use serde::Deserialize;
use std::sync::Arc;

fn claims(headers: &HeaderMap, state: &AppState) -> Result<crate::auth::Claims, ApiError> {
    extract_claims(headers, &state.config.jwt_secret)
}

#[derive(Deserialize)]
struct CreateTelegramConfig {
    bot_token: String,
    partner_telegram_id: String,
    #[serde(default)]
    allowed_users: Vec<String>,
    #[serde(default = "default_group_policy")]
    group_policy: String,
}

fn default_group_policy() -> String {
    "mention".into()
}

async fn create_config(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateTelegramConfig>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    crate::db::telegram::upsert_config(
        &state.db,
        &c.sub,
        &req.bot_token,
        &req.partner_telegram_id,
        &req.allowed_users,
        &req.group_policy,
    )
    .await
    .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;

    crate::channels::telegram::start_bot(Arc::clone(&state), c.sub.clone(), req.bot_token.clone())
        .await;

    Ok(Json(serde_json::json!({
        "user_id": c.sub,
        "enabled": true,
        "partner_telegram_id": req.partner_telegram_id,
        "allowed_users": req.allowed_users,
        "group_policy": req.group_policy,
    })))
}

async fn get_config(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let config = crate::db::telegram::get_config(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Telegram not configured"))?;
    Ok(Json(serde_json::json!({
        "user_id": config.user_id,
        "enabled": config.enabled,
        "partner_telegram_id": config.partner_telegram_id,
        "allowed_users": config.allowed_users,
        "group_policy": config.group_policy,
    })))
}

async fn delete_config(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    crate::db::telegram::delete_config(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    crate::channels::telegram::stop_bot(&c.sub).await;
    Ok(Json(
        serde_json::json!({ "message": "Telegram config deleted" }),
    ))
}

pub fn telegram_api_routes() -> Router<Arc<AppState>> {
    Router::new().route(
        "/api/telegram-config",
        post(create_config).get(get_config).delete(delete_config),
    )
}
