/// Session management: WebSocket connection to server, handshake, heartbeat, tool registration.
///
/// Client only sends its token. Server resolves user_id, device_name from DB.
/// LoginSuccess returns the device_name assigned by the user at token creation time.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nexus_common::consts::{HEARTBEAT_INTERVAL_SEC, PROTOCOL_VERSION};
use nexus_common::protocol::{ClientToServer, FsPolicy, ServerToClient};
use tokio::sync::{RwLock, mpsc};
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

use crate::config::ClientConfig;

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

pub async fn connect_and_loop(config: ClientConfig, policy_lock: Arc<RwLock<FsPolicy>>) -> ClientSession {
    let (outbound_tx, outbound_rx) = mpsc::channel::<ClientToServer>(256);
    let (inbound_tx, inbound_rx) = mpsc::channel::<ServerToClient>(256);

    tokio::spawn(connection_manager_loop(config, inbound_tx, outbound_rx, policy_lock));

    ClientSession {
        outbound_tx,
        inbound_rx,
    }
}

async fn connection_manager_loop(
    config: ClientConfig,
    inbound_tx: mpsc::Sender<ServerToClient>,
    mut outbound_rx: mpsc::Receiver<ClientToServer>,
    policy_lock: Arc<RwLock<FsPolicy>>,
) {
    let mut backoff_sec = 1u64;
    loop {
        match connect_async(&config.server_ws_url).await {
            Ok((mut ws_stream, _)) => {
                info!("connected to server: {}", config.server_ws_url);
                match run_single_connection(&mut ws_stream, &config, &inbound_tx, &mut outbound_rx, policy_lock.clone())
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
    policy_lock: Arc<RwLock<FsPolicy>>,
) -> Result<(), String> {
    let (device_name, initial_policy) = perform_handshake(ws_stream, &config.auth_token).await?;
    *policy_lock.write().await = initial_policy;

    // Discover and register tools
    let (schemas, skills, hash) =
        crate::discovery::discover_all(&config.mcp_servers, &config.skills_dir).await;

    let register = ClientToServer::RegisterTools { schemas, skills };
    send_client_message(ws_stream, &register).await?;

    let mut heartbeat = tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SEC));
    let mut last_hash = hash;

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                let (current_schemas, current_skills, current_hash) =
                    crate::discovery::discover_all(&config.mcp_servers, &config.skills_dir).await;

                let heartbeat_event = ClientToServer::Heartbeat {
                    hash: current_hash.clone(),
                    status: "online".to_string(),
                };
                send_client_message(ws_stream, &heartbeat_event).await?;

                if current_hash != last_hash {
                    let register = ClientToServer::RegisterTools {
                        schemas: current_schemas,
                        skills: current_skills,
                    };
                    send_client_message(ws_stream, &register).await?;
                    last_hash = current_hash;
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

                // Handle HeartbeatAck locally — update policy if changed
                if let ServerToClient::HeartbeatAck { fs_policy } = &parsed {
                    let current = policy_lock.read().await;
                    if *current != *fs_policy {
                        drop(current);
                        info!("FsPolicy updated via heartbeat: {:?}", fs_policy);
                        *policy_lock.write().await = fs_policy.clone();
                    }
                    continue;
                }

                if inbound_tx.send(parsed).await.is_err() {
                    return Err("inbound channel closed".to_string());
                }
            }
        }
    }
}

/// Handshake: wait for RequireLogin, send SubmitToken, receive LoginSuccess.
/// Returns (device_name, fs_policy) assigned by server.
async fn perform_handshake(
    ws_stream: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    auth_token: &str,
) -> Result<(String, FsPolicy), String> {
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
        token: auth_token.to_string(),
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
        ServerToClient::LoginSuccess { device_name, fs_policy, .. } => {
            info!("device login success: device_name={}, fs_policy={:?}", device_name, fs_policy);
            Ok((device_name, fs_policy))
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
