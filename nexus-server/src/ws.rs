//! Client WebSocket handler: device login, heartbeat, tool registration, tool results.

use crate::state::{AppState, DeviceConnection};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use nexus_common::consts::{HEARTBEAT_REAPER_INTERVAL_SEC, PROTOCOL_VERSION};
use nexus_common::protocol::{
    ClientToServer, FsPolicy, McpServerEntry, ServerToClient, ToolExecutionResult,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::sync::Mutex;
use tracing::{info, warn};

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_connection(socket, state))
}

async fn handle_connection(socket: WebSocket, state: Arc<AppState>) {
    let (mut sink, mut stream) = socket.split();

    // Send RequireLogin
    let msg = ServerToClient::RequireLogin {
        message: "NEXUS Server v1.0".into(),
    };
    if send_msg(&mut sink, &msg).await.is_err() {
        return;
    }

    // Await SubmitToken
    let (user_id, device_name, device_token) = match stream.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientToServer>(&text) {
            Ok(ClientToServer::SubmitToken {
                token,
                protocol_version,
            }) => {
                if protocol_version != PROTOCOL_VERSION {
                    let _ = send_msg(
                        &mut sink,
                        &ServerToClient::LoginFailed {
                            reason: format!(
                                "Protocol mismatch: expected {PROTOCOL_VERSION}, got {protocol_version}"
                            ),
                        },
                    )
                    .await;
                    return;
                }
                match crate::db::devices::find_by_token(&state.db, &token).await {
                    Ok(Some(dt)) => (dt.user_id.clone(), dt.device_name.clone(), dt),
                    _ => {
                        let _ = send_msg(
                            &mut sink,
                            &ServerToClient::LoginFailed {
                                reason: "Invalid token".into(),
                            },
                        )
                        .await;
                        return;
                    }
                }
            }
            _ => return,
        },
        _ => return,
    };

    // Send LoginSuccess
    let fs_policy: FsPolicy =
        serde_json::from_value(device_token.fs_policy.clone()).unwrap_or_default();
    let mcp_servers: Vec<McpServerEntry> =
        serde_json::from_value(device_token.mcp_config.clone()).unwrap_or_default();
    let ssrf_whitelist: Vec<String> =
        serde_json::from_value(device_token.ssrf_whitelist.clone()).unwrap_or_default();

    if send_msg(
        &mut sink,
        &ServerToClient::LoginSuccess {
            user_id: user_id.clone(),
            device_name: device_name.clone(),
            fs_policy,
            mcp_servers,
            workspace_path: device_token.workspace_path.clone(),
            shell_timeout: device_token.shell_timeout as u64,
            ssrf_whitelist,
        },
    )
    .await
    .is_err()
    {
        return;
    }

    info!("Device connected: {user_id}:{device_name}");

    // Register device in state
    let device_key = AppState::device_key(&user_id, &device_name);
    let sink = Arc::new(Mutex::new(sink));
    let last_seen = Arc::new(AtomicI64::new(chrono::Utc::now().timestamp()));

    state.devices.insert(
        device_key.clone(),
        DeviceConnection {
            user_id: user_id.clone(),
            device_name: device_name.clone(),
            sink: Arc::clone(&sink),
            last_seen: Arc::clone(&last_seen),
            tools: Vec::new(),
        },
    );
    state
        .devices_by_user
        .entry(user_id.clone())
        .or_default()
        .push(device_key.clone());
    state.pending.entry(device_key.clone()).or_default();

    // Message loop
    while let Some(Ok(msg)) = stream.next().await {
        let Message::Text(text) = msg else {
            continue;
        };
        let Ok(client_msg) = serde_json::from_str::<ClientToServer>(&text) else {
            warn!("Bad message from {device_key}");
            continue;
        };
        last_seen.store(chrono::Utc::now().timestamp(), Ordering::SeqCst);

        match client_msg {
            ClientToServer::Heartbeat { .. } => {
                let _ = send_msg_arc(&sink, &ServerToClient::HeartbeatAck).await;
            }
            ClientToServer::RegisterTools { schemas } => {
                if let Some(mut conn) = state.devices.get_mut(&device_key) {
                    conn.tools = schemas;
                }
                state.tool_schema_cache.remove(&user_id);
                info!("Tools registered for {device_key}");
            }
            ClientToServer::ToolExecutionResult(result) => {
                resolve_pending(&state, &device_key, result);
            }
            ClientToServer::FileResponse {
                request_id,
                content_base64,
                mime_type,
                error,
            } => {
                let result = ToolExecutionResult {
                    request_id: request_id.clone(),
                    exit_code: if error.is_some() { 1 } else { 0 },
                    output: if let Some(e) = error {
                        e
                    } else {
                        serde_json::json!({
                            "content_base64": content_base64,
                            "mime_type": mime_type,
                        })
                        .to_string()
                    },
                };
                resolve_pending(&state, &device_key, result);
            }
            ClientToServer::FileSendAck { request_id, error } => {
                let result = ToolExecutionResult {
                    request_id: request_id.clone(),
                    exit_code: if error.is_some() { 1 } else { 0 },
                    output: error.unwrap_or_else(|| "ok".into()),
                };
                resolve_pending(&state, &device_key, result);
            }
            _ => {}
        }
    }

    // Disconnect cleanup
    info!("Device disconnected: {device_key}");
    state.devices.remove(&device_key);
    if let Some(mut keys) = state.devices_by_user.get_mut(&user_id) {
        keys.retain(|k| k != &device_key);
        if keys.is_empty() {
            drop(keys);
            state.devices_by_user.remove(&user_id);
        }
    }
    state.pending.remove(&device_key);
    state.tool_schema_cache.remove(&user_id);
}

fn resolve_pending(state: &AppState, device_key: &str, result: ToolExecutionResult) {
    if let Some(device_pending) = state.pending.get(device_key) {
        if let Some((_, sender)) = device_pending.remove(&result.request_id) {
            let _ = sender.send(result);
        }
    }
}

async fn send_msg(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    msg: &ServerToClient,
) -> Result<(), ()> {
    let json = serde_json::to_string(msg).unwrap();
    sink.send(Message::Text(json.into())).await.map_err(|_| ())
}

async fn send_msg_arc(
    sink: &Arc<Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    msg: &ServerToClient,
) -> Result<(), ()> {
    let json = serde_json::to_string(msg).unwrap();
    sink.lock()
        .await
        .send(Message::Text(json.into()))
        .await
        .map_err(|_| ())
}

/// Spawn heartbeat reaper: checks every 30s for stale devices.
pub fn spawn_heartbeat_reaper(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(
            HEARTBEAT_REAPER_INTERVAL_SEC,
        ));
        loop {
            interval.tick().await;
            let now = chrono::Utc::now().timestamp();
            let timeout = nexus_common::consts::HEARTBEAT_INTERVAL_SEC as i64 * 4;
            let mut stale = Vec::new();
            for entry in state.devices.iter() {
                let last = entry.value().last_seen.load(Ordering::SeqCst);
                if now - last > timeout {
                    stale.push(entry.key().clone());
                }
            }
            for key in stale {
                warn!("Reaping stale device: {key}");
                if let Some((_, conn)) = state.devices.remove(&key) {
                    if let Some(mut keys) = state.devices_by_user.get_mut(&conn.user_id) {
                        keys.retain(|k| k != &key);
                        if keys.is_empty() {
                            drop(keys);
                            state.devices_by_user.remove(&conn.user_id);
                        }
                    }
                    state.pending.remove(&key);
                    state.tool_schema_cache.remove(&conn.user_id);
                }
            }
        }
    });
}
