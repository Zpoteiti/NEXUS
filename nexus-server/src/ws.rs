/// Client WebSocket connection handler.
///
/// Flow: RequireLogin → SubmitToken { token, protocol_version } → verify → LoginSuccess
/// Then: message loop (Heartbeat, RegisterTools, ToolExecutionResult)
/// On disconnect: cleanup device from routing tables and cancel pending requests.

use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use nexus_common::consts::{HEARTBEAT_INTERVAL_SEC, PROTOCOL_VERSION};
use nexus_common::protocol::{ClientToServer, ServerToClient};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{info, warn};

use crate::db;
use crate::state::{AppState, DeviceState, cancel_pending_requests_for_device};

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| socket_receive_loop(socket, state))
}

pub async fn socket_receive_loop(socket: WebSocket, state: AppState) {
    let (mut sink, mut stream) = socket.split();

    // Step 1: Send RequireLogin
    let require_login = ServerToClient::RequireLogin {
        message: "Please authenticate".to_string(),
    };
    let require_login_text = match serde_json::to_string(&require_login) {
        Ok(text) => text,
        Err(_) => return,
    };
    if sink.send(Message::Text(require_login_text.into())).await.is_err() {
        return;
    }

    // Step 2: Wait for SubmitToken
    let timeout_sec = state.config.heartbeat_timeout_sec;
    if timeout_sec < HEARTBEAT_INTERVAL_SEC {
        warn!(
            "heartbeat_timeout_sec({}) < HEARTBEAT_INTERVAL_SEC({})",
            timeout_sec, HEARTBEAT_INTERVAL_SEC
        );
    }
    let first_message = match timeout(Duration::from_secs(timeout_sec), stream.next()).await {
        Ok(Some(Ok(msg))) => msg,
        _ => return,
    };

    let login_text = match first_message {
        Message::Text(text) => text.to_string(),
        _ => return,
    };

    let (token, protocol_version) = match serde_json::from_str::<ClientToServer>(&login_text) {
        Ok(ClientToServer::SubmitToken {
            token,
            protocol_version,
        }) => (token, protocol_version),
        _ => return,
    };

    if protocol_version != PROTOCOL_VERSION {
        let failed = ServerToClient::LoginFailed {
            reason: "Protocol version mismatch".to_string(),
        };
        if let Ok(text) = serde_json::to_string(&failed) {
            let _ = sink.send(Message::Text(text.into())).await;
        }
        return;
    }

    // Step 3: Verify token → get user_id and device_name from DB
    let db::DeviceTokenVerification { user_id, device_name } = match db::verify_device_token(&state.db, &token).await {
        Ok(Some(v)) => v,
        _ => {
            let failed = ServerToClient::LoginFailed {
                reason: "Invalid token".to_string(),
            };
            if let Ok(text) = serde_json::to_string(&failed) {
                let _ = sink.send(Message::Text(text.into())).await;
            }
            return;
        }
    };

    // Use token as internal device key
    let device_key = token.clone();

    // Fetch current policy and MCP config before registering device
    let fs_policy = db::get_device_policy(&state.db, &user_id, &device_name)
        .await
        .unwrap_or_default();
    let mcp_servers = db::get_device_mcp_config(&state.db, &user_id, &device_name)
        .await
        .unwrap_or_default();

    // Step 4: Register device in routing tables
    let (ws_tx, mut ws_rx) = mpsc::channel::<Message>(1024);
    state.devices.insert(
        device_key.clone(),
        DeviceState {
            user_id: user_id.clone(),
            device_name: device_name.clone(),
            ws_tx: ws_tx.clone(),
            tools: Vec::new(),
            fs_policy: fs_policy.clone(),
            mcp_servers: mcp_servers.clone(),
            last_seen: Instant::now(),
        },
    );
    state.devices_by_user
        .entry(user_id.clone())
        .or_default()
        .insert(device_name.clone(), device_key.clone());
    state.config_dirty.insert(device_key.clone(), false);

    // Spawn writer task
    let writer = tokio::spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Step 5: Send LoginSuccess (tell client its device_name and current policy)
    let login_success = ServerToClient::LoginSuccess {
        user_id: user_id.clone(),
        device_name: device_name.clone(),
        fs_policy,
        mcp_servers,
    };
    let login_success_text = match serde_json::to_string(&login_success) {
        Ok(text) => text,
        Err(_) => {
            writer.abort();
            cleanup_device(&state, &device_key, &user_id).await;
            return;
        }
    };
    if ws_tx.send(Message::Text(login_success_text.into())).await.is_err() {
        writer.abort();
        cleanup_device(&state, &device_key, &user_id).await;
        return;
    }

    info!("device online: device_name={}, user_id={}", device_name, user_id);

    // Step 6: Message loop
    while let Some(frame) = stream.next().await {
        let message = match frame {
            Ok(msg) => msg,
            Err(_) => break,
        };

        let text = match message {
            Message::Text(text) => text.to_string(),
            Message::Close(_) => break,
            _ => continue,
        };

        let incoming = match serde_json::from_str::<ClientToServer>(&text) {
            Ok(v) => v,
            Err(_) => {
                warn!("invalid json from device={}", device_name);
                continue;
            }
        };

        match incoming {
            ClientToServer::Heartbeat { hash: _, status: _ } => {
                // Only re-query DB when config has been changed via the API
                let dirty = state.config_dirty.remove(&device_key).map(|(_, v)| v).unwrap_or(true);

                let (policy, mcp) = if dirty {
                    let (fresh_policy, fresh_mcp) = tokio::join!(
                        db::get_device_policy(&state.db, &user_id, &device_name),
                        db::get_device_mcp_config(&state.db, &user_id, &device_name),
                    );
                    let fresh_policy = fresh_policy.unwrap_or_default();
                    let fresh_mcp = fresh_mcp.unwrap_or_default();

                    if let Some(mut device) = state.devices.get_mut(&device_key) {
                        device.last_seen = Instant::now();
                        device.fs_policy = fresh_policy.clone();
                        device.mcp_servers = fresh_mcp.clone();
                    }

                    (fresh_policy, fresh_mcp)
                } else {
                    let cached = if let Some(mut device) = state.devices.get_mut(&device_key) {
                        device.last_seen = Instant::now();
                        (device.fs_policy.clone(), device.mcp_servers.clone())
                    } else {
                        (Default::default(), Vec::new())
                    };
                    cached
                };

                let ack = ServerToClient::HeartbeatAck { fs_policy: policy, mcp_servers: mcp };
                let ack_text = serde_json::to_string(&ack).unwrap_or_default();
                let _ = ws_tx.send(Message::Text(ack_text.into())).await;
            }
            ClientToServer::RegisterTools { schemas } => {
                if let Some(mut device) = state.devices.get_mut(&device_key) {
                    device.tools = schemas;
                }
                state.bump_tool_schema_generation();
            }
            ClientToServer::ToolExecutionResult(result) => {
                if let Some((_, tx)) = state.pending.remove(&result.request_id) {
                    let _ = tx.send(result);
                } else {
                    warn!("missing pending sender for request_id from device={}", device_name);
                }
            }
            ClientToServer::FileUploadResponse(response) => {
                if let Some((_, tx)) = state.file_upload_pending.remove(&response.request_id) {
                    let _ = tx.send(response);
                } else {
                    warn!("ws: no pending file upload for request_id={} from device={}", response.request_id, device_name);
                }
            }
            ClientToServer::FileDownloadResponse(response) => {
                if let Some((_, tx)) = state.file_download_pending.remove(&response.request_id) {
                    let _ = tx.send(response);
                } else {
                    warn!("ws: no pending file download for request_id={} from device={}", response.request_id, device_name);
                }
            }
            _ => {
                warn!("unsupported message from device={}", device_name);
            }
        }
    }

    // Step 7: Cleanup on disconnect
    writer.abort();
    cleanup_device(&state, &device_key, &user_id).await;
    info!("device offline: device_name={}, user_id={}", device_name, user_id);
}

/// Background task: periodically check for stale device connections and clean them up.
pub async fn heartbeat_reaper(state: AppState) {
    let check_interval = std::time::Duration::from_secs(30);
    let timeout_dur = std::time::Duration::from_secs(state.config.heartbeat_timeout_sec);

    loop {
        tokio::time::sleep(check_interval).await;

        // Collect stale device keys
        let stale: Vec<(String, String)> = state.devices
            .iter()
            .filter(|entry| entry.value().last_seen.elapsed() > timeout_dur)
            .map(|entry| (entry.key().clone(), entry.value().user_id.clone()))
            .collect();

        for (device_key, user_id) in &stale {
            warn!("heartbeat reaper: device timed out, cleaning up (user_id={})", user_id);
            cleanup_device(&state, device_key, user_id).await;
        }

        if !stale.is_empty() {
            info!("heartbeat reaper: cleaned up {} stale device(s)", stale.len());
        }
    }
}

async fn cleanup_device(state: &AppState, device_key: &str, user_id: &str) {
    if state.devices.remove(device_key).is_some() {
        if let Some(mut user_devices) = state.devices_by_user.get_mut(user_id) {
            user_devices.retain(|_, v| v != device_key);
            if user_devices.is_empty() {
                drop(user_devices);
                state.devices_by_user.remove(user_id);
            }
        }
    }
    cancel_pending_requests_for_device(device_key, &state.pending, &state.file_upload_pending, &state.file_download_pending);
    // Clean up schema cache for the disconnecting user
    state.tool_schema_cache.remove(user_id);
    state.bump_tool_schema_generation();
}
