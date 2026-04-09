//! Device token CRUD + policy/MCP management endpoints.

use crate::auth::extract_claims;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use futures_util::SinkExt;
use nexus_common::consts::DEVICE_TOKEN_PREFIX;
use nexus_common::error::{ApiError, ErrorCode};
use nexus_common::protocol::{FsPolicy, McpServerEntry, ServerToClient};
use serde::Deserialize;
use std::sync::Arc;

fn claims(headers: &HeaderMap, state: &AppState) -> Result<crate::auth::Claims, ApiError> {
    extract_claims(headers, &state.config.jwt_secret)
}

// -- Token CRUD --

#[derive(Deserialize)]
struct CreateTokenRequest {
    device_name: String,
}

async fn create_token(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateTokenRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let token = format!("{}{}", DEVICE_TOKEN_PREFIX, uuid::Uuid::new_v4().simple());
    crate::db::devices::create_token(&state.db, &token, &c.sub, &req.device_name)
        .await
        .map_err(|e| {
            if e.to_string().contains("unique") || e.to_string().contains("duplicate") {
                ApiError::new(ErrorCode::Conflict, "Device name already exists")
            } else {
                ApiError::new(ErrorCode::InternalError, format!("{e}"))
            }
        })?;
    Ok(Json(serde_json::json!({
        "token": token,
        "device_name": req.device_name,
    })))
}

async fn list_tokens(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<crate::db::devices::DeviceToken>>, ApiError> {
    let c = claims(&headers, &state)?;
    let tokens = crate::db::devices::list_by_user(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(tokens))
}

async fn delete_token(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let dt = crate::db::devices::find_by_token(&state.db, &token)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Token not found"))?;
    if dt.user_id != c.sub {
        return Err(ApiError::new(ErrorCode::Forbidden, "Not your token"));
    }
    crate::db::devices::delete_token(&state.db, &token)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    let key = AppState::device_key(&c.sub, &dt.device_name);
    state.devices.remove(&key);
    if let Some(mut keys) = state.devices_by_user.get_mut(&c.sub) {
        keys.retain(|k| k != &key);
    }
    state.tool_schema_cache.remove(&c.sub);
    Ok(Json(serde_json::json!({ "message": "Token deleted" })))
}

// -- Device Status --

async fn list_devices(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let tokens = crate::db::devices::list_by_user(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    let mut devices = Vec::new();
    for dt in tokens {
        let key = AppState::device_key(&c.sub, &dt.device_name);
        let (status, tools_count) = if let Some(conn) = state.devices.get(&key) {
            ("online", conn.tools.len())
        } else {
            ("offline", 0)
        };
        devices.push(serde_json::json!({
            "device_name": dt.device_name,
            "status": status,
            "tools_count": tools_count,
            "fs_policy": dt.fs_policy,
        }));
    }
    Ok(Json(serde_json::json!(devices)))
}

// -- Policy --

async fn get_policy(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(device_name): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let dt = crate::db::devices::find_by_user_and_device(&state.db, &c.sub, &device_name)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Device not found"))?;
    Ok(Json(serde_json::json!({
        "device_name": dt.device_name,
        "fs_policy": dt.fs_policy,
    })))
}

#[derive(Deserialize)]
struct PolicyUpdate {
    fs_policy: serde_json::Value,
}

async fn patch_policy(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(device_name): Path<String>,
    Json(req): Json<PolicyUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let updated =
        crate::db::devices::update_fs_policy(&state.db, &c.sub, &device_name, &req.fs_policy)
            .await
            .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    if !updated {
        return Err(ApiError::new(ErrorCode::NotFound, "Device not found"));
    }
    push_config_update(&state, &c.sub, &device_name, |msg| {
        let fs_policy: FsPolicy = serde_json::from_value(req.fs_policy.clone()).unwrap_or_default();
        *msg = ServerToClient::ConfigUpdate {
            fs_policy: Some(fs_policy),
            mcp_servers: None,
            workspace_path: None,
            shell_timeout: None,
            ssrf_whitelist: None,
        };
    })
    .await;
    Ok(Json(serde_json::json!({
        "device_name": device_name,
        "fs_policy": req.fs_policy,
    })))
}

// -- MCP Config --

async fn get_mcp(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(device_name): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let dt = crate::db::devices::find_by_user_and_device(&state.db, &c.sub, &device_name)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Device not found"))?;
    Ok(Json(serde_json::json!({
        "device_name": dt.device_name,
        "mcp_servers": dt.mcp_config,
    })))
}

#[derive(Deserialize)]
struct McpUpdate {
    mcp_servers: serde_json::Value,
}

async fn put_mcp(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(device_name): Path<String>,
    Json(req): Json<McpUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let updated =
        crate::db::devices::update_mcp_config(&state.db, &c.sub, &device_name, &req.mcp_servers)
            .await
            .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    if !updated {
        return Err(ApiError::new(ErrorCode::NotFound, "Device not found"));
    }
    push_config_update(&state, &c.sub, &device_name, |msg| {
        let mcp_servers: Vec<McpServerEntry> =
            serde_json::from_value(req.mcp_servers.clone()).unwrap_or_default();
        *msg = ServerToClient::ConfigUpdate {
            fs_policy: None,
            mcp_servers: Some(mcp_servers),
            workspace_path: None,
            shell_timeout: None,
            ssrf_whitelist: None,
        };
    })
    .await;
    Ok(Json(serde_json::json!({
        "device_name": device_name,
        "mcp_servers": req.mcp_servers,
    })))
}

/// Push a ConfigUpdate to a connected client device.
async fn push_config_update(
    state: &AppState,
    user_id: &str,
    device_name: &str,
    build: impl FnOnce(&mut ServerToClient),
) {
    let key = AppState::device_key(user_id, device_name);
    let sink = state.devices.get(&key).map(|conn| Arc::clone(&conn.sink));
    if let Some(sink) = sink {
        let mut msg = ServerToClient::HeartbeatAck; // placeholder
        build(&mut msg);
        let json = serde_json::to_string(&msg).unwrap();
        let mut s = sink.lock().await;
        let _ = s.send(axum::extract::ws::Message::Text(json.into())).await;
    }
}

pub fn device_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/device-tokens", post(create_token).get(list_tokens))
        .route("/api/device-tokens/{token}", delete(delete_token))
        .route("/api/devices", get(list_devices))
        .route(
            "/api/devices/{device_name}/policy",
            get(get_policy).patch(patch_policy),
        )
        .route("/api/devices/{device_name}/mcp", get(get_mcp).put(put_mcp))
}
