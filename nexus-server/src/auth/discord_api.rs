//! Discord config CRUD API endpoints.

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
struct CreateDiscordConfig {
    bot_token: String,
    partner_discord_id: String,
    #[serde(default)]
    allowed_users: Vec<String>,
}

async fn create_config(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateDiscordConfig>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    crate::db::discord::upsert_config(
        &state.db,
        &c.sub,
        &req.bot_token,
        &req.partner_discord_id,
        &req.allowed_users,
    )
    .await
    .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;

    // Start Discord bot
    crate::channels::discord::start_bot(Arc::clone(&state), c.sub.clone(), req.bot_token.clone())
        .await;

    Ok(Json(serde_json::json!({
        "user_id": c.sub,
        "enabled": true,
        "partner_discord_id": req.partner_discord_id,
        "allowed_users": req.allowed_users,
    })))
}

async fn get_config(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let config = crate::db::discord::get_config(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Discord not configured"))?;
    Ok(Json(serde_json::json!({
        "user_id": config.user_id,
        "bot_user_id": config.bot_user_id,
        "enabled": config.enabled,
        "partner_discord_id": config.partner_discord_id,
        "allowed_users": config.allowed_users,
    })))
}

async fn delete_config(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    crate::db::discord::delete_config(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    crate::channels::discord::stop_bot(&c.sub).await;
    Ok(Json(
        serde_json::json!({ "message": "Discord config deleted" }),
    ))
}

pub fn discord_api_routes() -> Router<Arc<AppState>> {
    Router::new().route(
        "/api/discord-config",
        post(create_config).get(get_config).delete(delete_config),
    )
}
