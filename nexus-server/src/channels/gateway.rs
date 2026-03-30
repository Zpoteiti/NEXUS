//! GatewayChannel — WS client that connects to nexus-gateway's /ws/nexus endpoint,
//! authenticates, and bridges inbound/outbound messages.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, warn};

use crate::agent_loop;
use crate::bus::InboundEvent;
use crate::channels::Channel;
use crate::state::AppState;

// ============================================================================
// Protocol types
// ============================================================================

/// Messages nexus-server sends TO the gateway
#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NexusToGateway {
    Auth { token: String },
    Send { chat_id: String, content: String },
}

/// Messages nexus-server receives FROM the gateway
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GatewayToNexus {
    AuthOk,
    AuthFail { reason: String },
    Message { chat_id: String, sender_id: String, content: String },
}

// ============================================================================
// Helper
// ============================================================================

pub fn make_session_id(chat_id: &str) -> String {
    format!("gateway:{}", chat_id)
}

// ============================================================================
// GatewayChannel
// ============================================================================

pub struct GatewayChannel {
    ws_url: String,
    token: String,
    state: Arc<AppState>,
    /// Sender half used by `send()` to push text frames to the WS write task.
    ws_out: Arc<RwLock<Option<mpsc::Sender<String>>>>,
}

impl GatewayChannel {
    pub fn new(state: Arc<AppState>) -> Self {
        let ws_url = std::env::var("NEXUS_GATEWAY_WS_URL")
            .unwrap_or_else(|_| "ws://localhost:9090/ws/nexus".to_string());
        let token = std::env::var("NEXUS_GATEWAY_TOKEN").unwrap_or_default();

        Self {
            ws_url,
            token,
            state,
            ws_out: Arc::new(RwLock::new(None)),
        }
    }

    /// Handle one inbound message from the gateway:
    /// create a session if new, route the event into it.
    async fn handle_inbound(
        &self,
        chat_id: String,
        sender_id: String,
        content: String,
    ) {
        let session_id = make_session_id(&chat_id);

        let (is_new, channels) = self
            .state
            .session_manager
            .get_or_create_session(&session_id)
            .await;

        if is_new {
            if let Some((inbox_tx, inbox_rx)) = channels {
                // Register the inbox sender in the bus routing table
                self.state
                    .bus
                    .register_session(session_id.clone(), inbox_tx.clone());

                // Spawn per-session agent loop
                let state_clone = self.state.clone();
                let sid = session_id.clone();
                tokio::spawn(async move {
                    agent_loop::run_session(sid, inbox_rx, state_clone).await;
                });
            }
        }

        // Publish the inbound event to the session's inbox via the bus
        let event = InboundEvent {
            channel: "gateway".to_string(),
            sender_id,
            chat_id,
            content,
            session_id,
            timestamp: Some(chrono::Utc::now()),
            media: Vec::new(),
            metadata: HashMap::new(),
        };
        self.state.bus.publish_inbound(event).await;
    }

    /// Single WS connection attempt. Returns Ok(()) if the connection closes
    /// gracefully (server closed), or Err if auth failed / IO error.
    async fn run_once(&self) -> Result<(), String> {
        info!("GatewayChannel: connecting to {}", self.ws_url);

        let (ws_stream, _) = connect_async(&self.ws_url)
            .await
            .map_err(|e| format!("connect failed: {}", e))?;

        info!("GatewayChannel: connected, sending auth");

        let (mut ws_sink, mut ws_source) = ws_stream.split();

        // Send auth
        let auth_msg = serde_json::to_string(&NexusToGateway::Auth {
            token: self.token.clone(),
        })
        .map_err(|e| format!("serialize auth: {}", e))?;
        ws_sink
            .send(Message::Text(auth_msg.into()))
            .await
            .map_err(|e| format!("send auth: {}", e))?;

        // Wait for auth_ok / auth_fail
        let auth_resp = ws_source
            .next()
            .await
            .ok_or_else(|| "WS closed before auth response".to_string())?
            .map_err(|e| format!("ws read error during auth: {}", e))?;

        match auth_resp {
            Message::Text(text) => {
                match serde_json::from_str::<GatewayToNexus>(&text) {
                    Ok(GatewayToNexus::AuthOk) => {
                        info!("GatewayChannel: auth_ok");
                    }
                    Ok(GatewayToNexus::AuthFail { reason }) => {
                        return Err(format!("auth_fail: {}", reason));
                    }
                    Ok(_) => {
                        return Err("unexpected message before auth_ok".to_string());
                    }
                    Err(e) => {
                        return Err(format!("parse auth response: {}", e));
                    }
                }
            }
            _ => return Err("expected text frame for auth response".to_string()),
        }

        // Set up mpsc channel for outbound send() calls → ws write task
        let (out_tx, mut out_rx) = mpsc::channel::<String>(64);
        *self.ws_out.write().await = Some(out_tx);

        // Spawn write task
        tokio::spawn(async move {
            while let Some(text) = out_rx.recv().await {
                if let Err(e) = ws_sink.send(Message::Text(text.into())).await {
                    warn!("GatewayChannel write task: send error: {}", e);
                    break;
                }
            }
        });

        // Read loop
        while let Some(msg) = ws_source.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    match serde_json::from_str::<GatewayToNexus>(&text) {
                        Ok(GatewayToNexus::Message {
                            chat_id,
                            sender_id,
                            content,
                        }) => {
                            self.handle_inbound(chat_id, sender_id, content).await;
                        }
                        Ok(GatewayToNexus::AuthOk | GatewayToNexus::AuthFail { .. }) => {
                            warn!("GatewayChannel: unexpected auth message in read loop");
                        }
                        Err(e) => {
                            warn!("GatewayChannel: failed to parse message: {}", e);
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    info!("GatewayChannel: server closed WS connection");
                    break;
                }
                Ok(_) => {} // ignore ping/pong/binary
                Err(e) => {
                    return Err(format!("ws read error: {}", e));
                }
            }
        }

        // Clean up sender on disconnect
        *self.ws_out.write().await = None;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Channel for GatewayChannel {
    fn name(&self) -> &str {
        "gateway"
    }

    async fn start(&self) {
        let mut backoff = Duration::from_secs(1);
        const MAX_BACKOFF: Duration = Duration::from_secs(60);

        loop {
            match self.run_once().await {
                Ok(()) => {
                    backoff = Duration::from_secs(1);
                    info!("GatewayChannel: disconnected gracefully, reconnecting...");
                }
                Err(e) => {
                    error!("GatewayChannel: error: {}. Reconnecting in {:?}", e, backoff);
                }
            }

            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }

    async fn stop(&self) {
        *self.ws_out.write().await = None;
    }

    async fn send(&self, chat_id: &str, content: &str) -> Result<(), String> {
        let guard = self.ws_out.read().await;
        match guard.as_ref() {
            Some(tx) => {
                let msg = serde_json::to_string(&NexusToGateway::Send {
                    chat_id: chat_id.to_string(),
                    content: content.to_string(),
                })
                .map_err(|e| format!("serialize send: {}", e))?;
                tx.send(msg)
                    .await
                    .map_err(|e| format!("channel send error: {}", e))
            }
            None => Err("GatewayChannel not connected".to_string()),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_from_chat_id() {
        assert_eq!(make_session_id("abc-123"), "gateway:abc-123");
    }

    #[test]
    fn parse_gateway_message_ok() {
        let json = r#"{"type":"message","chat_id":"c1","sender_id":"u1","content":"hi"}"#;
        let msg: GatewayToNexus = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, GatewayToNexus::Message { .. }));
    }

    #[test]
    fn parse_auth_ok() {
        let json = r#"{"type":"auth_ok"}"#;
        let msg: GatewayToNexus = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, GatewayToNexus::AuthOk));
    }

    #[test]
    fn serialize_auth_message() {
        let msg = NexusToGateway::Auth { token: "tok".to_string() };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"auth""#));
        assert!(json.contains("tok"));
    }

    #[test]
    fn serialize_send_message() {
        let msg = NexusToGateway::Send {
            chat_id: "c1".to_string(),
            content: "hello".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"send""#));
        assert!(json.contains("c1"));
    }
}
