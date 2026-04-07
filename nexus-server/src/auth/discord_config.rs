/// Discord config handlers (upsert, get, delete).

use axum::{
    extract::{Json, State},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use nexus_common::error::{ApiError, ErrorCode};

use super::Claims;
use crate::db;
use crate::state::AppState;

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

/// GET /api/discord-config -- get current user's Discord config
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

/// DELETE /api/discord-config -- delete current user's Discord config
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
