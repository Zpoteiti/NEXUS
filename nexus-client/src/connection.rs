//! WebSocket connection, auth handshake, and message loop.

use crate::config::ClientConfig;
use futures_util::{SinkExt, StreamExt};
use nexus_common::consts::PROTOCOL_VERSION;
use nexus_common::protocol::{ClientToServer, ServerToClient};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info};

pub type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    Message,
>;
pub type WsStream = futures_util::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
>;

/// Send a ClientToServer message as JSON over WebSocket.
pub async fn send_message(sink: &mut WsSink, msg: &ClientToServer) -> Result<(), String> {
    let json = serde_json::to_string(msg).map_err(|e| format!("serialize: {e}"))?;
    sink.send(Message::Text(json.into()))
        .await
        .map_err(|e| format!("send: {e}"))
}

/// Read next ServerToClient message from WebSocket.
pub async fn recv_message(stream: &mut WsStream) -> Result<ServerToClient, String> {
    loop {
        match stream.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str::<ServerToClient>(&text)
                    .map_err(|e| format!("deserialize: {e}"));
            }
            Some(Ok(Message::Close(_))) => return Err("connection closed".into()),
            Some(Err(e)) => return Err(format!("ws error: {e}")),
            None => return Err("stream ended".into()),
            _ => continue,
        }
    }
}

/// Perform WebSocket connection + auth handshake. Returns (sink, stream, config) on success.
pub async fn connect_and_auth(
    ws_url: &str,
    token: &str,
) -> Result<(WsSink, WsStream, ClientConfig), String> {
    info!("Connecting to {ws_url}...");
    let (ws, _) = connect_async(ws_url)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    let (mut sink, mut stream) = ws.split();

    // Receive RequireLogin
    match recv_message(&mut stream).await? {
        ServerToClient::RequireLogin { message } => info!("Server: {message}"),
        other => return Err(format!("Expected RequireLogin, got: {other:?}")),
    }

    // Send SubmitToken
    send_message(
        &mut sink,
        &ClientToServer::SubmitToken {
            token: token.to_string(),
            protocol_version: PROTOCOL_VERSION.to_string(),
        },
    )
    .await?;
    debug!("Sent SubmitToken");

    // Receive LoginSuccess or LoginFailed
    match recv_message(&mut stream).await? {
        ServerToClient::LoginSuccess {
            user_id,
            device_name,
            fs_policy,
            mcp_servers,
            workspace_path,
            shell_timeout,
            ssrf_whitelist,
        } => {
            info!("Login success: user={user_id}, device={device_name}");
            Ok((
                sink,
                stream,
                ClientConfig::from_login(
                    workspace_path,
                    fs_policy,
                    shell_timeout,
                    ssrf_whitelist,
                    mcp_servers,
                ),
            ))
        }
        ServerToClient::LoginFailed { reason } => Err(format!("Login failed: {reason}")),
        other => Err(format!("Expected LoginSuccess/Failed, got: {other:?}")),
    }
}
