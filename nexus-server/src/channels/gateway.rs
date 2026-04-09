//! Gateway WebSocket client channel.
//! Connects to nexus-gateway, authenticates, forwards messages.
//! Stub for now — full implementation in M4 when gateway exists.

use crate::bus::{self, InboundEvent, OutboundEvent};
use crate::state::AppState;
use std::sync::Arc;
use tracing::{error, info, warn};

/// Spawn the gateway connection loop. Reconnects with exponential backoff.
pub fn spawn_gateway_client(state: Arc<AppState>) {
    let ws_url = state.config.gateway_ws_url.clone();
    let token = state.config.gateway_token.clone();

    tokio::spawn(async move {
        let mut backoff = 1u64;
        loop {
            info!("Gateway: connecting to {ws_url}...");
            match connect_and_run(&state, &ws_url, &token).await {
                Ok(()) => {
                    info!("Gateway: connection closed cleanly");
                    backoff = 1;
                }
                Err(e) => {
                    warn!("Gateway: connection error: {e}");
                }
            }
            info!("Gateway: reconnecting in {backoff}s...");
            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(30);
        }
    });
}

async fn connect_and_run(state: &Arc<AppState>, ws_url: &str, token: &str) -> Result<(), String> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    let (ws, _) = connect_async(ws_url)
        .await
        .map_err(|e| format!("connect: {e}"))?;
    let (mut sink, mut stream) = ws.split();

    // Authenticate
    let auth = serde_json::json!({ "type": "auth", "token": token });
    sink.send(Message::Text(serde_json::to_string(&auth).unwrap().into()))
        .await
        .map_err(|e| format!("send auth: {e}"))?;

    // Await auth response
    match stream.next().await {
        Some(Ok(Message::Text(text))) => {
            let msg: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| format!("parse: {e}"))?;
            match msg.get("type").and_then(|t| t.as_str()) {
                Some("auth_ok") => info!("Gateway: authenticated"),
                Some("auth_fail") => {
                    let reason = msg
                        .get("reason")
                        .and_then(|r| r.as_str())
                        .unwrap_or("unknown");
                    return Err(format!("auth failed: {reason}"));
                }
                _ => return Err(format!("unexpected auth response: {text}")),
            }
        }
        _ => return Err("no auth response".into()),
    }

    // Store sink for outbound delivery
    let sink = Arc::new(tokio::sync::Mutex::new(sink));
    // Register gateway sink in state for outbound delivery
    // (Using a simple approach: store in a dedicated field or use the existing outbound channel)
    // For now, store as a gateway-specific sink
    {
        let mut gw = state.gateway_sink.write().await;
        *gw = Some(sink.clone());
    }

    // Message loop: receive messages from gateway
    while let Some(Ok(msg)) = stream.next().await {
        let Message::Text(text) = msg else {
            continue;
        };
        let parsed: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let msg_type = parsed.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if msg_type != "message" {
            continue;
        }

        let content = parsed
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        let chat_id = parsed
            .get("chat_id")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());
        let sender_id = parsed
            .get("sender_id")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());
        let session_id = parsed
            .get("session_id")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();

        if content.is_empty() || session_id.is_empty() {
            continue;
        }

        // The sender_id from gateway is the user_id (JWT-authenticated)
        let user_id = sender_id.clone().unwrap_or_default();

        // Gateway users are always the account owner (JWT-authenticated)
        let event = InboundEvent {
            session_id,
            user_id: user_id.clone(),
            content,
            channel: nexus_common::consts::CHANNEL_GATEWAY.to_string(),
            chat_id,
            sender_id: Some(user_id.clone()),
            media: vec![],
            cron_job_id: None,
            identity: None, // Gateway = always owner, built from User in agent_loop
            metadata: Default::default(),
        };

        if let Err(e) = bus::publish_inbound(state, event).await {
            error!("Gateway inbound error: {e}");
        }
    }

    // Clear gateway sink on disconnect
    {
        let mut gw = state.gateway_sink.write().await;
        *gw = None;
    }

    Ok(())
}

/// Deliver an outbound event to the gateway.
pub async fn deliver(state: &AppState, event: &OutboundEvent) {
    let sink = state.gateway_sink.read().await;
    let Some(sink) = sink.as_ref() else {
        warn!("Gateway: not connected, dropping outbound message");
        return;
    };

    let mut msg = serde_json::json!({
        "type": "send",
        "chat_id": event.chat_id,
        "content": event.content,
    });

    let mut metadata = serde_json::Map::new();
    if event.is_progress {
        metadata.insert("_progress".into(), serde_json::json!(true));
    }
    if !event.media.is_empty() {
        metadata.insert("media".into(), serde_json::json!(event.media));
    }
    if let Some(sender_id) = event.metadata.get("sender_id") {
        metadata.insert("sender_id".into(), serde_json::json!(sender_id));
    }
    if !metadata.is_empty() {
        msg["metadata"] = serde_json::Value::Object(metadata);
    }

    let json = serde_json::to_string(&msg).unwrap();
    let mut s = sink.lock().await;
    if let Err(e) = futures_util::SinkExt::send(
        &mut *s,
        tokio_tungstenite::tungstenite::Message::Text(json.into()),
    )
    .await
    {
        warn!("Gateway send error: {e}");
    }
}
