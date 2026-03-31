/// 职责边界：
/// 1. 仅负责处理 `/ws` 路由的 WebSocket 升级请求。
/// 2. 维护单个 WebSocket 连接的收发大循环 (Split Stream & Sink)。
/// 3. 负责在连接时调用 state.rs 将新设备注册到 AppState，断开时注销并清理挂起请求。
/// 4. 收到 Client 消息时，进行反序列化并分发。
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【Client 握手认证流程】
/// ─────────────────────────────────────────────────────────────────────────────
/// WebSocket 连接建立后，在进入正常消息循环之前，必须先完成以下握手序列：
///
/// 1. ws.rs 向 Client 发送 ServerToClient::RequireLogin { message: "Please authenticate" }
/// 2. 等待 Client 回复 ClientToServer::SubmitToken { token, device_id, device_name, protocol_version }
/// 3. 调用 db::verify_device_token(token) 验证 Device Token：
///    - 成功：取得 (user_id, device_id)，
///            向 Client 发送 ServerToClient::LoginSuccess { user_id, device_id }，
///            将设备注册到 AppState 在线设备路由表（含 ws_tx、last_seen 等字段），
///            随后进入正常消息收发循环。
///    - 失败：向 Client 发送 ServerToClient::LoginFailed { reason }，
///            立即关闭 WebSocket 连接，不注册到 AppState。
///
/// 握手超时处理：若在 HEARTBEAT_TIMEOUT_SEC 内未收到 SubmitToken，
/// 直接关闭连接（防止恶意空连接耗尽连接池）。
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【断线时清理挂起请求（盲区 6-A）】
/// ─────────────────────────────────────────────────────────────────────────────
/// 当 WebSocket 收发循环退出（无论正常断开还是网络异常），必须执行以下清理：
///
/// 1. 从 AppState 在线设备路由表中注销该 (user_id, device_id)。
/// 2. 调用 state::cancel_pending_requests_for_device(device_id, &pending_table)：
///    遍历工具调用挂起等待表，找出所有 request_id 归属该 device_id 的条目，
///    将对应的 oneshot::Sender 全部 drop 掉（Receiver 端会收到 Err(RecvError)）。
/// 3. agent_loop.rs 中 .await oneshot::Receiver 处理 Err 时，
///    将错误包装为 "Tool execution failed: device disconnected" 的 Tool Result，
///    以 tool_result 角色消息喂回 LLM，触发自我纠正机制（agent_loop.rs 已有描述）。
///
/// 若不执行步骤 2，agent_loop 将永久 .await 挂起，该 session 的 LLM 调用链永远无法继续。

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

    let require_login = ServerToClient::RequireLogin {
        message: "Please authenticate".to_string(),
    };
    let require_login_text = match serde_json::to_string(&require_login) {
        Ok(text) => text,
        Err(_) => return,
    };
    if sink
        .send(Message::Text(require_login_text.into()))
        .await
        .is_err()
    {
        return;
    }

    let timeout_sec = state.config.heartbeat_timeout_sec;
    if timeout_sec < HEARTBEAT_INTERVAL_SEC {
        warn!(
            "warn: heartbeat_timeout_sec({}) < HEARTBEAT_INTERVAL_SEC({})",
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

    let submit = match serde_json::from_str::<ClientToServer>(&login_text) {
        Ok(ClientToServer::SubmitToken {
            token,
            device_id,
            device_name,
            protocol_version,
        }) => (token, device_id, device_name, protocol_version),
        _ => return,
    };

    let (token, device_id, device_name, protocol_version) = submit;

    if protocol_version != PROTOCOL_VERSION {
        let failed = ServerToClient::LoginFailed {
            reason: "Protocol version mismatch".to_string(),
        };
        if let Ok(text) = serde_json::to_string(&failed) {
            let _ = sink.send(Message::Text(text.into())).await;
        }
        return;
    }

    let user_id = match db::verify_device_token(&state.db, &token).await {
        Ok(Some(user_id)) => user_id,
        _ => {
            let failed = ServerToClient::LoginFailed {
                reason: "Invalid or revoked token".to_string(),
            };
            if let Ok(text) = serde_json::to_string(&failed) {
                let _ = sink.send(Message::Text(text.into())).await;
            }
            return;
        }
    };

    // device_name is set at token creation time via WebUI, not overwritten by client
    let (ws_tx, mut ws_rx) = mpsc::channel::<Message>(256);
    {
        let mut devices = state.devices.write().await;
        devices.insert(
            device_id.clone(),
            DeviceState {
                user_id: user_id.clone(),
                device_name: device_name.clone(),
                ws_tx: ws_tx.clone(),
                tools: Vec::new(),
                last_seen: Instant::now(),
            },
        );
        // 维护 devices_by_user 索引：user_id → { device_name → device_id }
        let mut devices_by_user = state.devices_by_user.write().await;
        devices_by_user
            .entry(user_id.clone())
            .or_default()
            .insert(device_name.clone(), device_id.clone());
    }

    let writer = tokio::spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    let login_success = ServerToClient::LoginSuccess {
        user_id: user_id.clone(),
        device_id: device_id.clone(),
    };
    let login_success_text = match serde_json::to_string(&login_success) {
        Ok(text) => text,
        Err(_) => {
            writer.abort();
            {
                let mut devices = state.devices.write().await;
                devices.remove(&device_id);
            }
            cancel_pending_requests_for_device(&device_id, &state.pending).await;
            info!("device offline: device_id={device_id}");
            return;
        }
    };
    if ws_tx
        .send(Message::Text(login_success_text.into()))
        .await
        .is_err()
    {
        writer.abort();
        {
            let mut devices = state.devices.write().await;
            devices.remove(&device_id);
        }
        cancel_pending_requests_for_device(&device_id, &state.pending).await;
        info!("device offline: device_id={device_id}");
        return;
    }

    info!("device online: device_id={device_id}, user_id={user_id}");

    while let Some(frame) = stream.next().await {
        let message = match frame {
            Ok(msg) => msg,
            Err(_) => break,
        };

        let text = match message {
            Message::Text(text) => text.to_string(),
            Message::Close(_) => break,
            _ => {
                warn!("non-text frame ignored from device_id={device_id}");
                continue;
            }
        };

        let incoming = match serde_json::from_str::<ClientToServer>(&text) {
            Ok(v) => v,
            Err(_) => {
                warn!("invalid json from device_id={device_id}");
                continue;
            }
        };

        match incoming {
            ClientToServer::Heartbeat {
                device_id: incoming_device_id,
                device_name: incoming_device_name,
                hash: _,
                status: _,
            } => {
                if incoming_device_id != device_id {
                    warn!(
                        "heartbeat device_id mismatch: expected={}, got={}",
                        device_id, incoming_device_id
                    );
                    continue;
                }
                let mut devices = state.devices.write().await;
                let mut devices_by_user = state.devices_by_user.write().await;
                if let Some(device) = devices.get_mut(&device_id) {
                    device.last_seen = Instant::now();
                    // 若 device_name 变了（重连后用户改了配置），更新索引
                    if device.device_name != incoming_device_name {
                        if let Some(user_devices) = devices_by_user.get_mut(&user_id) {
                            user_devices.retain(|_, v| v != &device_id);
                            user_devices.insert(incoming_device_name.clone(), device_id.clone());
                        }
                        device.device_name = incoming_device_name;
                    }
                }
            }
            ClientToServer::RegisterTools {
                device_id: incoming_device_id,
                device_name: incoming_device_name,
                schemas,
                skills: _,
            } => {
                if incoming_device_id != device_id {
                    warn!(
                        "register_tools device_id mismatch: expected={}, got={}",
                        device_id, incoming_device_id
                    );
                    continue;
                }
                let mut devices = state.devices.write().await;
                if let Some(device) = devices.get_mut(&device_id) {
                    device.tools = schemas;
                    device.device_name = incoming_device_name.clone();
                    // 更新 devices_by_user 索引中的 device_name（可能重连后名称变了）
                    let mut devices_by_user = state.devices_by_user.write().await;
                    if let Some(user_devices) = devices_by_user.get_mut(&user_id) {
                        user_devices.retain(|_, v| v != &device_id);
                        user_devices.insert(incoming_device_name.clone(), device_id.clone());
                    }
                }
            }
            ClientToServer::ToolExecutionResult(result) => {
                let sender = {
                    let mut pending = state.pending.write().await;
                    pending.remove(&result.request_id)
                };
                if let Some(tx) = sender {
                    let _ = tx.send(result);
                } else {
                    warn!(
                        "missing pending sender for request_id from device_id={device_id}"
                    );
                }
            }
            _ => {
                warn!("unsupported message ignored from device_id={device_id}");
            }
        }
    }

    writer.abort();

    {
        let mut devices = state.devices.write().await;
        if let Some(device_state) = devices.remove(&device_id) {
            // 从 devices_by_user 中移除该设备
            let mut devices_by_user = state.devices_by_user.write().await;
            if let Some(user_devices) = devices_by_user.get_mut(&user_id) {
                user_devices.retain(|_, v| v != &device_id);
                if user_devices.is_empty() {
                    devices_by_user.remove(&user_id);
                }
            }
            info!("device offline: device_id={}, device_name={}", device_id, device_state.device_name);
        }
    }
    cancel_pending_requests_for_device(&device_id, &state.pending).await;
}
