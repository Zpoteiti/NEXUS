use axum::{
    extract::{State, ws::{Message, WebSocket, WebSocketUpgrade}},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::protocol::{BrowserOutbound, NexusInbound, NexusOutbound};
use crate::state::SharedState;

pub async fn nexus_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| nexus_connection(socket, state))
}

async fn nexus_connection(socket: WebSocket, state: SharedState) {
    let (mut sink, mut stream) = socket.split();

    // 1. Wait for auth message
    let first = match stream.next().await {
        Some(Ok(Message::Text(t))) => t.to_string(),
        _ => return,
    };

    let token = match serde_json::from_str::<NexusInbound>(&first) {
        Ok(NexusInbound::Auth { token }) => token,
        _ => {
            let fail = serde_json::to_string(&NexusOutbound::AuthFail {
                reason: "expected auth message".to_string(),
            })
            .unwrap();
            let _ = sink.send(Message::Text(fail.into())).await;
            return;
        }
    };

    if !verify_token(&token, &state.gateway_token) {
        let fail = serde_json::to_string(&NexusOutbound::AuthFail {
            reason: "invalid token".to_string(),
        })
        .unwrap();
        let _ = sink.send(Message::Text(fail.into())).await;
        warn!("nexus gateway: auth rejected");
        return;
    }

    // Check if nexus is already connected before sending AuthOk
    {
        let guard = state.nexus_tx.read().await;
        if guard.is_some() {
            let fail = serde_json::to_string(&NexusOutbound::AuthFail {
                reason: "another nexus connection is already active".to_string(),
            })
            .unwrap();
            let _ = sink.send(Message::Text(fail.into())).await;
            warn!("nexus gateway: rejected duplicate connection");
            return;
        }
    }

    let ok = serde_json::to_string(&NexusOutbound::AuthOk).unwrap();
    if sink.send(Message::Text(ok.into())).await.is_err() {
        return;
    }
    info!("nexus gateway: nexus-server authenticated");

    // 2. Set up writer channel, store in state
    let (tx, mut rx) = mpsc::channel::<String>(256);
    {
        *state.nexus_tx.write().await = Some(tx);
    }

    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sink.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // 3. Receive nexus messages, route to browsers
    while let Some(frame) = stream.next().await {
        let text = match frame {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Close(_)) | Err(_) => break,
            _ => continue,
        };

        match serde_json::from_str::<NexusInbound>(&text) {
            Ok(NexusInbound::Send { chat_id, content, metadata }) => {
                route_nexus_send(&state, &chat_id, &content, metadata.as_ref()).await;
            }
            Ok(NexusInbound::Auth { .. }) => {
                warn!("nexus gateway: unexpected re-auth, ignoring");
            }
            Err(e) => {
                warn!("nexus gateway: invalid json: {}", e);
            }
        }
    }

    writer.abort();
    {
        *state.nexus_tx.write().await = None;
    }
    info!("nexus gateway: nexus-server disconnected");
}

/// Route a nexus agent reply to the corresponding browser connection.
pub async fn route_nexus_send(
    state: &crate::state::AppState,
    chat_id: &str,
    content: &str,
    metadata: Option<&serde_json::Value>,
) {
    let is_progress = metadata
        .and_then(|m| m.get("_progress"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if let Some(conn) = state.browser_conns.get(chat_id) {
        let session_id = conn.session_id.clone();

        // Extract media URLs from metadata if present
        let media = metadata
            .and_then(|m| m.get("media"))
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>());

        let msg = if is_progress {
            serde_json::to_string(&BrowserOutbound::Progress {
                content: content.to_string(),
                session_id,
            })
            .unwrap()
        } else {
            serde_json::to_string(&BrowserOutbound::Message {
                content: content.to_string(),
                session_id,
                media,
            })
            .unwrap()
        };

        if conn.tx.send(msg).await.is_err() {
            warn!("nexus gateway: browser {} disconnected before send", chat_id);
        }
    } else {
        warn!("nexus gateway: no browser found for chat_id={}", chat_id);
    }
}

pub fn verify_token(provided: &str, expected: &str) -> bool {
    let a = provided.as_bytes();
    let b = expected.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn route_send_to_browser() {
        let state = AppState::new("token".into(), "test-secret".into(), "http://localhost:8080".into());
        let (browser_tx, mut browser_rx) = mpsc::channel::<String>(8);
        state.browser_conns.insert("chat-abc".to_string(), crate::state::BrowserConnection {
            tx: browser_tx,
            user_id: "user1".to_string(),
            session_id: "gateway:user1:test-session".to_string(),
        });

        route_nexus_send(&state, "chat-abc", "hello browser", None).await;

        let msg = browser_rx.recv().await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "message");
        assert_eq!(parsed["content"], "hello browser");
        assert_eq!(parsed["session_id"], "gateway:user1:test-session");
        assert!(parsed.get("media").is_none() || parsed["media"].is_null());
    }

    #[tokio::test]
    async fn route_send_progress_to_browser() {
        let state = AppState::new("token".into(), "test-secret".into(), "http://localhost:8080".into());
        let (browser_tx, mut browser_rx) = mpsc::channel::<String>(8);
        state.browser_conns.insert("chat-abc".to_string(), crate::state::BrowserConnection {
            tx: browser_tx,
            user_id: "user1".to_string(),
            session_id: "gateway:user1:test-session".to_string(),
        });

        let metadata = serde_json::json!({"_progress": true});
        route_nexus_send(&state, "chat-abc", "thinking...", Some(&metadata)).await;

        let msg = browser_rx.recv().await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "progress");
        assert_eq!(parsed["content"], "thinking...");
    }

    #[tokio::test]
    async fn route_send_to_unknown_chat_id_is_noop() {
        let state = AppState::new("token".into(), "test-secret".into(), "http://localhost:8080".into());
        // chat_id not registered — should not panic
        route_nexus_send(&state, "unknown-chat", "hello", None).await;
    }

    #[test]
    fn verify_token_ok() {
        assert!(verify_token("secret", "secret"));
    }

    #[test]
    fn verify_token_fail() {
        assert!(!verify_token("secret", "wrong"));
    }
}
