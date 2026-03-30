use axum::{
    extract::{State, ws::{Message, WebSocket, WebSocketUpgrade}},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::protocol::{BrowserInbound, BrowserOutbound, NexusOutbound};
use crate::state::SharedState;

pub async fn browser_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| browser_connection(socket, state))
}

async fn browser_connection(socket: WebSocket, state: SharedState) {
    let chat_id = Uuid::new_v4().to_string();
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<String>(64);

    state.browser_conns.insert(chat_id.clone(), tx);
    info!("browser connected: chat_id={}", chat_id);

    // Writer task: push messages to browser
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sink.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // Reader loop: receive browser messages, forward to nexus
    while let Some(frame) = stream.next().await {
        let text = match frame {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Close(_)) | Err(_) => break,
            _ => continue,
        };

        let inbound: BrowserInbound = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => {
                warn!("browser invalid json: chat_id={}", chat_id);
                continue;
            }
        };

        let BrowserInbound::Message { content } = inbound;

        if let Err(e) = forward_browser_message(&state, &chat_id, &content).await {
            warn!("browser forward failed: {}", e);
            let err_json = serde_json::to_string(&BrowserOutbound::Error {
                reason: "Nexus server not connected".to_string(),
            })
            .unwrap();
            if let Some(tx) = state.browser_conns.get(&chat_id) {
                let _ = tx.send(err_json).await;
            }
        }
    }

    state.browser_conns.remove(&chat_id);
    writer.abort();
    info!("browser disconnected: chat_id={}", chat_id);
}

/// Wrap browser message as NexusOutbound::Message and send to nexus-server.
/// Returns Err if nexus is not connected.
pub async fn forward_browser_message(
    state: &crate::state::AppState,
    chat_id: &str,
    content: &str,
) -> Result<(), String> {
    let msg = NexusOutbound::Message {
        chat_id: chat_id.to_string(),
        sender_id: chat_id.to_string(),
        content: content.to_string(),
    };
    let json = serde_json::to_string(&msg).map_err(|e| e.to_string())?;

    let guard = state.nexus_tx.read().await;
    match guard.as_ref() {
        Some(tx) => tx.send(json).await.map_err(|e| e.to_string()),
        None => Err("nexus not connected".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;

    #[tokio::test]
    async fn forward_to_nexus_when_connected() {
        let state = AppState::new("token".to_string());
        let (nexus_tx, mut nexus_rx) = tokio::sync::mpsc::channel::<String>(8);
        *state.nexus_tx.write().await = Some(nexus_tx);

        let result = forward_browser_message(&state, "test-chat", "hello nexus").await;
        assert!(result.is_ok());

        let msg = nexus_rx.recv().await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "message");
        assert_eq!(parsed["chat_id"], "test-chat");
        assert_eq!(parsed["content"], "hello nexus");
    }

    #[tokio::test]
    async fn forward_to_nexus_when_disconnected_returns_err() {
        let state = AppState::new("token".to_string());
        // nexus_tx is None — no nexus connected
        let result = forward_browser_message(&state, "chat1", "hello").await;
        assert!(result.is_err());
    }
}
