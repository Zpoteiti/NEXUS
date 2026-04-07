use axum::{
    extract::{State, ws::{Message, WebSocket, WebSocketUpgrade}},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::protocol::{BrowserInbound, BrowserOutbound, NexusOutbound};
use crate::state::{BrowserConnection, SharedState};

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct Claims {
    sub: String,
    is_admin: bool,
    exp: usize,
}

pub fn verify_jwt(token: &str, secret: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    let key = jsonwebtoken::DecodingKey::from_secret(secret.as_bytes());
    let data = jsonwebtoken::decode::<Claims>(token, &key, &jsonwebtoken::Validation::default())?;
    Ok(data.claims)
}

pub async fn browser_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    headers: axum::http::HeaderMap,
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    // Try Authorization header first, then fall back to ?token= query param
    // (browsers cannot send custom headers on WebSocket upgrade requests)
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .or_else(|| query.get("token").map(|s| s.as_str()));

    let user_id = match token {
        Some(t) => match verify_jwt(t, &state.jwt_secret) {
            Ok(claims) => claims.sub,
            Err(_) => return (axum::http::StatusCode::UNAUTHORIZED, "Invalid token").into_response(),
        },
        None => return (axum::http::StatusCode::UNAUTHORIZED, "Missing token").into_response(),
    };

    ws.on_upgrade(move |socket| browser_connection(socket, state, user_id))
        .into_response()
}

async fn browser_connection(socket: WebSocket, state: SharedState, user_id: String) {
    let chat_id = Uuid::new_v4().to_string();
    let session_id = format!("gateway:{}:{}", user_id, Uuid::new_v4());
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<String>(64);

    state.browser_conns.insert(chat_id.clone(), BrowserConnection {
        tx: tx.clone(),
        user_id: user_id.clone(),
        session_id: session_id.clone(),
    });
    info!("browser connected: chat_id={} user_id={} session_id={}", chat_id, user_id, session_id);

    // Send initial session_created to browser
    let init_msg = serde_json::to_string(&BrowserOutbound::SessionCreated {
        session_id: session_id.clone(),
    }).unwrap();
    let _ = tx.send(init_msg).await;

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

        match inbound {
            BrowserInbound::Message { content, media } => {
                let current_session = state.browser_conns.get(&chat_id)
                    .map(|c| c.session_id.clone())
                    .unwrap_or_default();
                if let Err(e) = forward_browser_message(&state, &chat_id, &user_id, &content, &current_session, media.as_deref()).await {
                    warn!("browser forward failed: {}", e);
                    let err_json = serde_json::to_string(&BrowserOutbound::Error {
                        reason: "Nexus server not connected".to_string(),
                    })
                    .unwrap();
                    let tx = state.browser_conns.get(&chat_id).map(|c| c.tx.clone());
                    if let Some(tx) = tx {
                        let _ = tx.send(err_json).await;
                    }
                }
            }
            BrowserInbound::NewSession => {
                let new_session_id = format!("gateway:{}:{}", user_id, Uuid::new_v4());
                if let Some(mut conn) = state.browser_conns.get_mut(&chat_id) {
                    conn.session_id = new_session_id.clone();
                }
                let msg = serde_json::to_string(&BrowserOutbound::SessionCreated {
                    session_id: new_session_id,
                }).unwrap();
                let tx = state.browser_conns.get(&chat_id).map(|c| c.tx.clone());
                if let Some(tx) = tx {
                    let _ = tx.send(msg).await;
                }
            }
            BrowserInbound::SwitchSession { session_id } => {
                if let Some(mut conn) = state.browser_conns.get_mut(&chat_id) {
                    conn.session_id = session_id.clone();
                }
                let msg = serde_json::to_string(&BrowserOutbound::SessionSwitched {
                    session_id,
                }).unwrap();
                let tx = state.browser_conns.get(&chat_id).map(|c| c.tx.clone());
                if let Some(tx) = tx {
                    let _ = tx.send(msg).await;
                }
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
    user_id: &str,
    content: &str,
    session_id: &str,
    media: Option<&[String]>,
) -> Result<(), String> {
    let msg = NexusOutbound::Message {
        chat_id: chat_id.to_string(),
        sender_id: user_id.to_string(),
        content: content.to_string(),
        session_id: session_id.to_string(),
        media: media.map(|m| m.to_vec()),
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
        let state = AppState::new("token".into(), "test-secret".into(), "http://localhost:8080".into());
        let (nexus_tx, mut nexus_rx) = tokio::sync::mpsc::channel::<String>(8);
        *state.nexus_tx.write().await = Some(nexus_tx);

        let result = forward_browser_message(&state, "test-chat", "test-user-id", "hello nexus", "gateway:test-user-id:sess1", None).await;
        assert!(result.is_ok());

        let msg = nexus_rx.recv().await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "message");
        assert_eq!(parsed["chat_id"], "test-chat");
        assert_eq!(parsed["sender_id"], "test-user-id");
        assert_eq!(parsed["content"], "hello nexus");
        assert_eq!(parsed["session_id"], "gateway:test-user-id:sess1");
    }

    #[tokio::test]
    async fn forward_to_nexus_when_disconnected_returns_err() {
        let state = AppState::new("token".into(), "test-secret".into(), "http://localhost:8080".into());
        let result = forward_browser_message(&state, "chat1", "test-user-id", "hello", "gateway:test-user-id:sess1", None).await;
        assert!(result.is_err());
    }

    #[test]
    fn verify_jwt_rejects_invalid_token() {
        let result = verify_jwt("not-a-jwt", "secret");
        assert!(result.is_err());
    }

    #[test]
    fn verify_jwt_rejects_wrong_secret() {
        // A valid-looking JWT signed with a different secret should be rejected
        // (we just verify the error path works)
        let result = verify_jwt("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ1c2VyMSIsImlzX2FkbWluIjpmYWxzZSwiZXhwIjo5OTk5OTk5OTk5fQ.wrong_sig", "different-secret");
        assert!(result.is_err());
    }
}
