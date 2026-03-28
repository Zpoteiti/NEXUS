/// 职责边界：
/// 1. 专门管理与 Server 的 WebSocket 长连接 (`tokio_tungstenite`)。
/// 2. 负责断线重连机制 (Exponential Backoff)。
/// 3. 负责维持心跳 (Heartbeat)，定期向 Server 报告 Client 的存活状态和当前工具 Hash。
/// 4. 将收到的 `ServerToClient` 消息推入内部的 MPSC Channel，供 executor 消费。
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【心跳与工具热拔插流程】
/// ─────────────────────────────────────────────────────────────────────────────
/// - Client 每次发送心跳前，调用 `discovery::discover_all()` 重新扫描：
///   - 内置工具 + MCP 工具 → tools_schema
///   - Skills → skill_summaries
/// - 计算 tools_hash 和 skills_hash
/// - 若任一 hash 与上次不同，说明工具集发生了变更，则发送 RegisterTools 更新 Server
/// - Server 收到新的 RegisterTools 后，更新 AppState 中该设备的工具快照。

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nexus_common::consts::{HEARTBEAT_INTERVAL_SEC, PROTOCOL_VERSION};
use nexus_common::protocol::{ClientToServer, ServerToClient};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

use crate::config::ClientConfig;

/// ClientSession 保存与 Server 的会话状态，供 main.rs 访问。
pub struct ClientSession {
    outbound_tx: mpsc::Sender<ClientToServer>,
    inbound_rx: mpsc::Receiver<ServerToClient>,
}

impl ClientSession {
    pub async fn send(&self, message: ClientToServer) -> Result<(), String> {
        self.outbound_tx
            .send(message)
            .await
            .map_err(|_| "failed to enqueue outbound message".to_string())
    }

    pub async fn recv(&mut self) -> Option<ServerToClient> {
        self.inbound_rx.recv().await
    }
}

pub async fn connect_and_loop(config: ClientConfig) -> ClientSession {
    let (outbound_tx, outbound_rx) = mpsc::channel::<ClientToServer>(256);
    let (inbound_tx, inbound_rx) = mpsc::channel::<ServerToClient>(256);

    tokio::spawn(connection_manager_loop(config, inbound_tx, outbound_rx));

    ClientSession {
        outbound_tx,
        inbound_rx,
    }
}

async fn connection_manager_loop(
    config: ClientConfig,
    inbound_tx: mpsc::Sender<ServerToClient>,
    mut outbound_rx: mpsc::Receiver<ClientToServer>,
) {
    let mut backoff_sec = 1u64;
    loop {
        match connect_async(&config.server_ws_url).await {
            Ok((mut ws_stream, _)) => {
                info!("connected to server: {}", config.server_ws_url);
                match run_single_connection(&mut ws_stream, &config, &inbound_tx, &mut outbound_rx)
                    .await
                {
                    Ok(()) => {
                        backoff_sec = 1;
                    }
                    Err(err) => {
                        warn!("websocket loop ended: {}", err);
                    }
                }
            }
            Err(err) => {
                warn!("failed to connect server: {}", err);
            }
        }

        sleep(Duration::from_secs(backoff_sec)).await;
        backoff_sec = (backoff_sec.saturating_mul(2)).min(30);
    }
}

async fn run_single_connection(
    ws_stream: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    config: &ClientConfig,
    inbound_tx: &mpsc::Sender<ServerToClient>,
    outbound_rx: &mut mpsc::Receiver<ClientToServer>,
) -> Result<(), String> {
    let device_id = config.device_id.clone();
    let device_name = config.device_name.clone();
    perform_handshake(ws_stream, &device_id, &device_name).await?;

    // 步骤 2 — 登录成功后，立即发现并注册工具（重连时也会执行）
    let (schemas, skill_summaries, tools_hash, skills_hash) =
        crate::discovery::discover_all(&config.mcp_servers, &config.skills_dir).await;

    let register = ClientToServer::RegisterTools {
        device_id: device_id.clone(),
        device_name: device_name.clone(),
        schemas,
        skill_summaries,
    };
    send_client_message(ws_stream, &register).await?;

    let mut heartbeat = tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SEC));
    let mut last_tools_hash = tools_hash;
    let mut last_skills_hash = skills_hash;

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                // 每次心跳重新计算 hash，检测工具或 skills 是否变更
                let (current_schemas, current_skill_summaries, current_tools_hash, current_skills_hash) =
                    crate::discovery::discover_all(&config.mcp_servers, &config.skills_dir).await;

                let heartbeat_event = ClientToServer::Heartbeat {
                    device_id: device_id.clone(),
                    device_name: device_name.clone(),
                    tools_hash: current_tools_hash.clone(),
                    skills_hash: current_skills_hash.clone(),
                    status: "online".to_string(),
                };
                send_client_message(ws_stream, &heartbeat_event).await?;

                // 若 tools_hash 或 skills_hash 变了，说明工具集发生变更，立即重新注册
                if current_tools_hash != last_tools_hash || current_skills_hash != last_skills_hash {
                    let register = ClientToServer::RegisterTools {
                        device_id: device_id.clone(),
                        device_name: device_name.clone(),
                        schemas: current_schemas,
                        skill_summaries: current_skill_summaries,
                    };
                    send_client_message(ws_stream, &register).await?;
                    last_tools_hash = current_tools_hash;
                    last_skills_hash = current_skills_hash;
                }
            }
            outbound = outbound_rx.recv() => {
                match outbound {
                    Some(message) => {
                        send_client_message(ws_stream, &message).await?;
                    }
                    None => return Err("outbound channel closed".to_string()),
                }
            }
            incoming = ws_stream.next() => {
                let message = match incoming {
                    Some(Ok(msg)) => msg,
                    Some(Err(err)) => return Err(format!("websocket read error: {err}")),
                    None => return Err("websocket closed".to_string()),
                };

                if let Message::Close(_) = message {
                    return Err("server closed websocket".to_string());
                }

                let text = match message {
                    Message::Text(text) => text.to_string(),
                    _ => continue,
                };

                let parsed = serde_json::from_str::<ServerToClient>(&text)
                    .map_err(|err| format!("invalid server message json: {err}"))?;
                if inbound_tx.send(parsed).await.is_err() {
                    return Err("inbound channel closed".to_string());
                }
            }
        }
    }
}

async fn perform_handshake(
    ws_stream: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    device_id: &str,
    device_name: &str,
) -> Result<(), String> {
    let require_login = tokio::time::timeout(Duration::from_secs(30), ws_stream.next())
        .await
        .map_err(|_| "wait require-login timeout".to_string())?
        .ok_or_else(|| "websocket closed before require-login".to_string())
        .and_then(|msg| msg.map_err(|err| format!("websocket read error: {err}")))?;

    let require_login_text = match require_login {
        Message::Text(text) => text.to_string(),
        _ => return Err("expected text frame for require-login".to_string()),
    };

    let require_login_msg = serde_json::from_str::<ServerToClient>(&require_login_text)
        .map_err(|err| format!("invalid require-login json: {err}"))?;
    if !matches!(require_login_msg, ServerToClient::RequireLogin { .. }) {
        return Err("server did not send RequireLogin".to_string());
    }

    let submit = ClientToServer::SubmitToken {
        token: crate::config::load_config().auth_token.clone(),
        device_id: device_id.to_string(),
        device_name: device_name.to_string(),
        protocol_version: PROTOCOL_VERSION.to_string(),
    };
    send_client_message(ws_stream, &submit).await?;

    let login_result = tokio::time::timeout(Duration::from_secs(30), ws_stream.next())
        .await
        .map_err(|_| "wait login result timeout".to_string())?
        .ok_or_else(|| "websocket closed before login result".to_string())
        .and_then(|msg| msg.map_err(|err| format!("websocket read error: {err}")))?;

    let login_result_text = match login_result {
        Message::Text(text) => text.to_string(),
        _ => return Err("expected text frame for login result".to_string()),
    };

    let login_result_msg = serde_json::from_str::<ServerToClient>(&login_result_text)
        .map_err(|err| format!("invalid login-result json: {err}"))?;
    match login_result_msg {
        ServerToClient::LoginSuccess { .. } => {
            info!(
                "device login success: device_id={}",
                device_id,
            );
            Ok(())
        }
        ServerToClient::LoginFailed { reason } => Err(format!("login failed: {}", reason)),
        _ => Err("unexpected message during login".to_string()),
    }
}

async fn send_client_message(
    ws_stream: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    message: &ClientToServer,
) -> Result<(), String> {
    let text = serde_json::to_string(message).map_err(|err| format!("serialize error: {err}"))?;
    ws_stream
        .send(Message::Text(text.into()))
        .await
        .map_err(|err| format!("websocket send error: {err}"))
}
