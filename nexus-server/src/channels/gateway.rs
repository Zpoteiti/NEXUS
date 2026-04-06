//! GatewayChannel — WS client that connects to nexus-gateway's /ws/nexus endpoint,
//! authenticates, and bridges inbound/outbound messages.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

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
    Send { chat_id: String, content: String, #[serde(skip_serializing_if = "Option::is_none")] metadata: Option<serde_json::Value> },
}

/// Messages nexus-server receives FROM the gateway
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GatewayToNexus {
    AuthOk,
    AuthFail { reason: String },
    Message { chat_id: String, sender_id: String, content: String, #[serde(default)] session_id: Option<String>, #[serde(default)] media: Option<Vec<String>> },
}

// ============================================================================
// Helper
// ============================================================================

pub fn make_session_id(chat_id: &str) -> String {
    format!("gateway:{}", chat_id)
}

/// Search /tmp/nexus-uploads/ recursively for a file whose name starts with `file_id`.
async fn resolve_upload_file_id(file_id: &str) -> Option<String> {
    let base = "/tmp/nexus-uploads";
    let mut user_dirs = match tokio::fs::read_dir(base).await {
        Ok(d) => d,
        Err(_) => return None,
    };
    while let Ok(Some(user_entry)) = user_dirs.next_entry().await {
        let user_path = user_entry.path();
        if !user_path.is_dir() {
            continue;
        }
        let mut files = match tokio::fs::read_dir(&user_path).await {
            Ok(d) => d,
            Err(_) => continue,
        };
        while let Ok(Some(file_entry)) = files.next_entry().await {
            let name = file_entry.file_name().to_string_lossy().to_string();
            if name.starts_with(file_id) {
                return Some(file_entry.path().to_string_lossy().to_string());
            }
        }
    }
    None
}

/// Convert a `__FILE__:/tmp/nexus-media/uuid_filename` path to a `/api/files/uuid` download URL.
fn file_path_to_download_url(path: &str) -> Option<String> {
    let path = path.strip_prefix("__FILE__:")?;
    let filename = path.rsplit('/').next()?;
    // Filename format: {uuid}_{original_name}
    let uuid_part = filename.split('_').next()?;
    // Validate it looks like a UUID (36 chars with hyphens)
    if uuid_part.len() == 36 && uuid_part.chars().filter(|c| *c == '-').count() == 4 {
        Some(format!("/api/files/{}", uuid_part))
    } else {
        None
    }
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
    /// Cancellation token to break the reconnect loop on shutdown.
    shutdown: CancellationToken,
}

impl GatewayChannel {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            ws_url: state.config.gateway_ws_url.clone(),
            token: state.config.gateway_token.clone(),
            state,
            ws_out: Arc::new(RwLock::new(None)),
            shutdown: CancellationToken::new(),
        }
    }

    /// Handle one inbound message from the gateway.
    async fn handle_inbound(
        &self,
        chat_id: String,
        sender_id: String,
        content: String,
        forwarded_session_id: Option<String>,
        media: Option<Vec<String>>,
    ) {
        // Use the session_id forwarded by the gateway if available,
        // otherwise fall back to generating one from chat_id.
        let session_id = forwarded_session_id
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| make_session_id(&chat_id));

        // Resolve media file_ids to filesystem paths.
        // Frontend sends items as "file_id:file_name". We search /tmp/nexus-uploads/
        // for files whose name starts with the file_id.
        let resolved_media = if let Some(items) = media {
            let mut paths = Vec::new();
            for item in &items {
                let file_id = item.split(':').next().unwrap_or(item);
                if let Some(path) = resolve_upload_file_id(file_id).await {
                    paths.push(path);
                } else {
                    warn!("GatewayChannel: could not resolve media file_id: {}", file_id);
                }
            }
            paths
        } else {
            Vec::new()
        };

        let event = InboundEvent {
            channel: "gateway".to_string(),
            sender_id,
            chat_id,
            content,
            session_id,
            timestamp: Some(chrono::Utc::now()),
            media: resolved_media,
            metadata: HashMap::new(),
        };
        crate::session::ensure_session_and_publish(&self.state, event).await;
    }

    /// Single WS connection attempt. Returns Ok(()) if the connection closes
    /// gracefully (server closed), or Err if auth failed / IO error.
    async fn run_once(&self) -> Result<(), nexus_common::error::NexusError> {
        info!("GatewayChannel: connecting to {}", self.ws_url);

        use nexus_common::error::{ErrorCode, NexusError};

        let (ws_stream, _) = connect_async(&self.ws_url)
            .await
            .map_err(|e| NexusError::new(ErrorCode::ConnectionFailed, format!("connect failed: {}", e)))?;

        info!("GatewayChannel: connected, sending auth");

        let (mut ws_sink, mut ws_source) = ws_stream.split();

        // Send auth
        let auth_msg = serde_json::to_string(&NexusToGateway::Auth {
            token: self.token.clone(),
        })
        .map_err(|e| NexusError::new(ErrorCode::ChannelError, format!("serialize auth: {}", e)))?;
        ws_sink
            .send(Message::Text(auth_msg.into()))
            .await
            .map_err(|e| NexusError::new(ErrorCode::ChannelError, format!("send auth: {}", e)))?;

        // Wait for auth_ok / auth_fail
        let auth_resp = ws_source
            .next()
            .await
            .ok_or_else(|| NexusError::new(ErrorCode::ConnectionFailed, "WS closed before auth response"))?
            .map_err(|e| NexusError::new(ErrorCode::ConnectionFailed, format!("ws read error during auth: {}", e)))?;

        match auth_resp {
            Message::Text(text) => {
                match serde_json::from_str::<GatewayToNexus>(&text) {
                    Ok(GatewayToNexus::AuthOk) => {
                        info!("GatewayChannel: auth_ok");
                    }
                    Ok(GatewayToNexus::AuthFail { reason }) => {
                        return Err(NexusError::new(ErrorCode::AuthFailed, format!("auth_fail: {}", reason)));
                    }
                    Ok(_) => {
                        return Err(NexusError::new(ErrorCode::ChannelError, "unexpected message before auth_ok"));
                    }
                    Err(e) => {
                        return Err(NexusError::new(ErrorCode::ChannelError, format!("parse auth response: {}", e)));
                    }
                }
            }
            _ => return Err(NexusError::new(ErrorCode::ChannelError, "expected text frame for auth response")),
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
                            session_id,
                            media,
                        }) => {
                            self.handle_inbound(chat_id, sender_id, content, session_id, media).await;
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
                    return Err(NexusError::new(ErrorCode::ChannelError, format!("ws read error: {}", e)));
                }
            }
        }

        // Clean up sender on disconnect
        *self.ws_out.write().await = None;
        Ok(())
    }

    /// Send a message to the gateway with optional metadata.
    async fn send_with_metadata(
        &self,
        chat_id: &str,
        content: &str,
        metadata: Option<serde_json::Value>,
    ) -> Result<(), nexus_common::error::NexusError> {
        use nexus_common::error::{ErrorCode, NexusError};
        let guard = self.ws_out.read().await;
        match guard.as_ref() {
            Some(tx) => {
                let msg = serde_json::to_string(&NexusToGateway::Send {
                    chat_id: chat_id.to_string(),
                    content: content.to_string(),
                    metadata,
                })
                .map_err(|e| NexusError::new(ErrorCode::InternalError, format!("serialize send: {}", e)))?;
                tx.send(msg)
                    .await
                    .map_err(|e| NexusError::new(ErrorCode::ChannelError, format!("channel send error: {}", e)))
            }
            None => Err(NexusError::new(ErrorCode::ConnectionFailed, "GatewayChannel not connected")),
        }
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
            if self.shutdown.is_cancelled() {
                info!("GatewayChannel: shutdown requested, exiting reconnect loop");
                break;
            }

            match self.run_once().await {
                Ok(()) => {
                    backoff = Duration::from_secs(1);
                    info!("GatewayChannel: disconnected gracefully, reconnecting...");
                }
                Err(e) => {
                    error!("GatewayChannel: error: {}. Reconnecting in {:?}", e, backoff);
                }
            }

            tokio::select! {
                _ = self.shutdown.cancelled() => {
                    info!("GatewayChannel: shutdown during backoff, exiting");
                    break;
                }
                _ = tokio::time::sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }

    async fn stop(&self) {
        info!("GatewayChannel: stopping");
        self.shutdown.cancel();
        *self.ws_out.write().await = None;
    }

    async fn send(&self, chat_id: &str, content: &str) -> Result<(), nexus_common::error::NexusError> {
        self.send_with_metadata(chat_id, content, None).await
    }

    async fn send_progress(&self, chat_id: &str, content: &str) -> Result<(), nexus_common::error::NexusError> {
        self.send_with_metadata(chat_id, content, Some(serde_json::json!({"_progress": true}))).await
    }

    async fn send_with_media(
        &self,
        chat_id: &str,
        content: &str,
        media: &[String],
    ) -> Result<(), nexus_common::error::NexusError> {
        // Convert __FILE__:/tmp/nexus-media/... paths to /api/files/... download URLs
        let urls: Vec<String> = media
            .iter()
            .filter_map(|m| {
                if m.starts_with("__FILE__:") {
                    file_path_to_download_url(m)
                } else {
                    Some(m.clone())
                }
            })
            .collect();
        self.send_with_metadata(chat_id, content, Some(serde_json::json!({"media": urls}))).await
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
        let json = r#"{"type":"message","chat_id":"c1","sender_id":"u1","content":"hi","session_id":"gateway:u1:sess1"}"#;
        let msg: GatewayToNexus = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, GatewayToNexus::Message { session_id: Some(s), media: None, .. } if s == "gateway:u1:sess1"));
    }

    #[test]
    fn parse_gateway_message_without_session_id() {
        let json = r#"{"type":"message","chat_id":"c1","sender_id":"u1","content":"hi"}"#;
        let msg: GatewayToNexus = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, GatewayToNexus::Message { session_id: None, media: None, .. }));
    }

    #[test]
    fn parse_gateway_message_with_media() {
        let json = r#"{"type":"message","chat_id":"c1","sender_id":"u1","content":"hi","media":["abc-123:test.png"]}"#;
        let msg: GatewayToNexus = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, GatewayToNexus::Message { media: Some(m), .. } if m.len() == 1));
    }

    #[test]
    fn file_path_to_download_url_valid() {
        let url = file_path_to_download_url("__FILE__:/tmp/nexus-media/550e8400-e29b-41d4-a716-446655440000_photo.png");
        assert_eq!(url, Some("/api/files/550e8400-e29b-41d4-a716-446655440000".to_string()));
    }

    #[test]
    fn file_path_to_download_url_no_prefix() {
        let url = file_path_to_download_url("/tmp/nexus-media/550e8400-e29b-41d4-a716-446655440000_photo.png");
        assert_eq!(url, None);
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
            metadata: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"send""#));
        assert!(json.contains("c1"));
        // metadata should be omitted when None
        assert!(!json.contains("metadata"));
    }

    #[test]
    fn serialize_send_with_progress_metadata() {
        let msg = NexusToGateway::Send {
            chat_id: "c1".to_string(),
            content: "thinking".to_string(),
            metadata: Some(serde_json::json!({"_progress": true})),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""_progress":true"#));
    }
}
