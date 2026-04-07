/// Device token handlers (create, list, delete), device policy handlers, device MCP handlers.

use axum::{
    extract::{Json, Path, State},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use nexus_common::error::{ApiError, ErrorCode};

use super::Claims;
use crate::db;
use crate::state::AppState;

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

/// GET /api/device-tokens -- list all device tokens for current user
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

/// DELETE /api/device-tokens/:token -- delete a device token permanently
pub async fn delete_device_token(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(token): Path<String>,
) -> Response {
    match db::delete_device_token(&state.db, &token, &claims.sub).await {
        Ok(true) => Json(json!({"message": "Token deleted"})).into_response(),
        Ok(false) => ApiError::new(ErrorCode::NotFound, "token not found").into_response(),
        Err(e) => {
            tracing::error!("delete_device_token error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to delete token").into_response()
        }
    }
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

/// GET /api/devices/{device_name}/policy -- get the fs_policy for a device
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

/// PATCH /api/devices/{device_name}/policy -- update the fs_policy for a device
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
        Ok(false) => ApiError::new(ErrorCode::NotFound, "device not found").into_response(),
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

/// GET /api/devices/{device_name}/mcp -- get MCP config for a device
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

/// PUT /api/devices/{device_name}/mcp -- update MCP config for a device
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
        Ok(false) => ApiError::new(ErrorCode::NotFound, "device not found").into_response(),
        Err(e) => {
            tracing::error!("update_device_mcp error: {e}");
            ApiError::new(ErrorCode::InternalError, "operation failed").into_response()
        },
    }
}
