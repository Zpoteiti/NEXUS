//! DiscordGatewayConn — maintains a single Discord bot's Gateway WS connection.
//! Handles HELLO, IDENTIFY, Heartbeat, MESSAGE_CREATE, and reconnection.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::bus::InboundEvent;
use crate::db::{self, DiscordConfig};
use crate::state::AppState;

use super::protocol::*;
use super::rest;

/// Shared state for outbound routing: channel_id → bot_token
pub type ChannelTokenMap = Arc<DashMap<String, String>>;

use super::TypingTokenMap;

/// Run a single Discord bot connection with auto-reconnect.
pub async fn run(
    config: DiscordConfig,
    state: Arc<AppState>,
    channel_tokens: ChannelTokenMap,
    typing_tokens: TypingTokenMap,
    cancel: CancellationToken,
) {
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(60);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("DiscordGatewayConn [{}]: cancelled, shutting down", config.user_id);
                break;
            }
            result = run_once(&config, &state, &channel_tokens, &typing_tokens) => {
                match result {
                    Ok(()) => {
                        backoff = Duration::from_secs(1);
                        info!("DiscordGatewayConn [{}]: disconnected gracefully, reconnecting...", config.user_id);
                    }
                    Err(e) => {
                        error!("DiscordGatewayConn [{}]: error: {}. Reconnecting in {:?}", config.user_id, e, backoff);
                        // Only increase backoff on errors, not graceful disconnects
                        let wait = backoff;
                        backoff = (backoff * 2).min(max_backoff);
                        tokio::select! {
                            _ = cancel.cancelled() => break,
                            _ = tokio::time::sleep(wait) => {}
                        }
                        continue;
                    }
                }
            }
        }

        // Graceful disconnect: reconnect after 1s with no backoff increase
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(backoff) => {}
        }
    }
}

/// Single connection attempt. Returns Ok(()) on graceful close, Err on failure.
async fn run_once(
    config: &DiscordConfig,
    state: &Arc<AppState>,
    channel_tokens: &ChannelTokenMap,
    typing_tokens: &TypingTokenMap,
) -> Result<(), nexus_common::error::NexusError> {
    info!("DiscordGatewayConn [{}]: connecting to Gateway", config.user_id);

    use nexus_common::error::{ErrorCode, NexusError};

    let (ws_stream, _) = connect_async(GATEWAY_URL)
        .await
        .map_err(|e| NexusError::new(ErrorCode::ConnectionFailed, format!("connect failed: {}", e)))?;

    let (mut ws_sink, mut ws_source) = ws_stream.split();

    // 1. Wait for HELLO
    let heartbeat_interval = match ws_source.next().await {
        Some(Ok(Message::Text(text))) => {
            let frame: GatewayFrame = serde_json::from_str(&text)
                .map_err(|e| NexusError::new(ErrorCode::ChannelError, format!("parse HELLO: {}", e)))?;
            if frame.op != OP_HELLO {
                return Err(NexusError::new(ErrorCode::ChannelError, format!("expected HELLO (op 10), got op {}", frame.op)));
            }
            let interval_ms = frame.d
                .as_ref()
                .and_then(|d| d.get("heartbeat_interval"))
                .and_then(|v| v.as_u64())
                .ok_or_else(|| NexusError::new(ErrorCode::ChannelError, "missing heartbeat_interval in HELLO"))?;
            info!("DiscordGatewayConn [{}]: HELLO, heartbeat_interval={}ms", config.user_id, interval_ms);
            interval_ms
        }
        Some(Ok(_)) => return Err(NexusError::new(ErrorCode::ChannelError, "expected text frame for HELLO")),
        Some(Err(e)) => return Err(NexusError::new(ErrorCode::ConnectionFailed, format!("ws error waiting for HELLO: {}", e))),
        None => return Err(NexusError::new(ErrorCode::ConnectionFailed, "ws closed before HELLO")),
    };

    // 2. Send IDENTIFY
    ws_sink
        .send(Message::Text(identify_frame(&config.bot_token).into()))
        .await
        .map_err(|e| NexusError::new(ErrorCode::ChannelError, format!("send IDENTIFY: {}", e)))?;

    // 3. Start heartbeat task
    let seq = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let seq_valid = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let heartbeat_cancel = CancellationToken::new();

    let (hb_tx, mut hb_rx) = mpsc::channel::<String>(4);
    {
        let hb_seq = seq.clone();
        let hb_seq_valid = seq_valid.clone();
        let hb_cancel = heartbeat_cancel.clone();
        tokio::spawn(async move {
            let interval = Duration::from_millis(heartbeat_interval);
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {
                        let s = if hb_seq_valid.load(std::sync::atomic::Ordering::Relaxed) {
                            Some(hb_seq.load(std::sync::atomic::Ordering::Relaxed))
                        } else {
                            None
                        };
                        if hb_tx.send(heartbeat_frame(s)).await.is_err() {
                            break;
                        }
                    }
                    _ = hb_cancel.cancelled() => break,
                }
            }
        });
    }

    let mut bot_user_id: Option<String> = config.bot_user_id.clone();

    // 4. Read loop
    loop {
        tokio::select! {
            Some(hb_frame) = hb_rx.recv() => {
                if let Err(e) = ws_sink.send(Message::Text(hb_frame.into())).await {
                    heartbeat_cancel.cancel();
                    return Err(NexusError::new(ErrorCode::ChannelError, format!("send heartbeat: {}", e)));
                }
            }
            msg = ws_source.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let frame: GatewayFrame = match serde_json::from_str(&text) {
                            Ok(f) => f,
                            Err(e) => {
                                warn!("DiscordGatewayConn [{}]: failed to parse frame: {}", config.user_id, e);
                                continue;
                            }
                        };

                        if let Some(s) = frame.s {
                            seq.store(s, std::sync::atomic::Ordering::Relaxed);
                            seq_valid.store(true, std::sync::atomic::Ordering::Relaxed);
                        }

                        match frame.op {
                            OP_DISPATCH => {
                                let event_name = frame.t.as_deref().unwrap_or("");
                                match event_name {
                                    "READY" => {
                                        if let Some(d) = &frame.d {
                                            if let Some(ready) = parse_ready(d) {
                                                info!("DiscordGatewayConn [{}]: READY, bot_user_id={}", config.user_id, ready.bot_user_id);
                                                bot_user_id = Some(ready.bot_user_id.clone());
                                                let _ = db::update_bot_user_id(&state.db, &config.user_id, &ready.bot_user_id).await;
                                            }
                                        }
                                    }
                                    "MESSAGE_CREATE" => {
                                        if let Some(d) = &frame.d {
                                            if let Some(msg_data) = parse_message_create(d) {
                                                handle_message(
                                                    config,
                                                    state,
                                                    channel_tokens,
                                                    typing_tokens,
                                                    bot_user_id.as_deref(),
                                                    msg_data,
                                                ).await;
                                            }
                                        }
                                    }
                                    _ => {
                                        debug!("DiscordGatewayConn [{}]: ignoring event: {}", config.user_id, event_name);
                                    }
                                }
                            }
                            OP_HEARTBEAT_ACK => {}
                            OP_RECONNECT => {
                                info!("DiscordGatewayConn [{}]: RECONNECT requested", config.user_id);
                                heartbeat_cancel.cancel();
                                return Ok(());
                            }
                            OP_INVALID_SESSION => {
                                warn!("DiscordGatewayConn [{}]: INVALID_SESSION", config.user_id);
                                heartbeat_cancel.cancel();
                                return Ok(());
                            }
                            _ => {
                                debug!("DiscordGatewayConn [{}]: unhandled opcode {}", config.user_id, frame.op);
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("DiscordGatewayConn [{}]: server closed connection", config.user_id);
                        heartbeat_cancel.cancel();
                        return Ok(());
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        heartbeat_cancel.cancel();
                        return Err(NexusError::new(ErrorCode::ChannelError, format!("ws read error: {}", e)));
                    }
                    None => {
                        heartbeat_cancel.cancel();
                        return Ok(());
                    }
                }
            }
        }
    }
}

async fn handle_message(
    config: &DiscordConfig,
    state: &Arc<AppState>,
    channel_tokens: &ChannelTokenMap,
    typing_tokens: &TypingTokenMap,
    bot_user_id: Option<&str>,
    msg: MessageCreateData,
) {
    if msg.sender_is_bot {
        return;
    }

    let bot_uid = match bot_user_id {
        Some(id) => id,
        None => {
            warn!("DiscordGatewayConn [{}]: received message before READY, ignoring", config.user_id);
            return;
        }
    };

    let (session_id, user_id, content) = if msg.guild_id.is_some() {
        if !msg.mentions.contains(&bot_uid.to_string()) {
            return;
        }

        if !config.allowed_users.is_empty()
            && !config.allowed_users.contains(&msg.sender_id)
        {
            debug!("DiscordGatewayConn [{}]: sender {} not in whitelist, ignoring", config.user_id, msg.sender_id);
            return;
        }

        if config.allowed_users.is_empty() {
            debug!("DiscordGatewayConn [{}]: empty whitelist, ignoring guild message from {}", config.user_id, msg.sender_id);
            return;
        }

        let sid = if let Some(ref thread_id) = msg.thread_id {
            format!("discord:thread:{}", thread_id)
        } else {
            format!("discord:guild:{}", msg.channel_id)
        };
        let clean_content = strip_mention(&msg.content, bot_uid);
        let prefixed = format!("{}: {}", msg.sender_name, clean_content);
        (sid, config.user_id.clone(), prefixed)
    } else {
        // DM: sender must be in allowed_users. Empty list = nobody allowed.
        if !config.allowed_users.contains(&msg.sender_id) {
            debug!("DiscordGatewayConn [{}]: DM sender {} not in allowed_users, ignoring", config.user_id, msg.sender_id);
            return;
        }
        let sid = format!("discord:dm:{}", msg.sender_id);
        let prefixed = format!("{}: {}", msg.sender_name, msg.content);
        (sid, config.user_id.clone(), prefixed)
    };

    // Parse and download attachments
    let mut media_paths = Vec::new();
    let mut content = content; // make mutable for attachment annotations

    for att in &msg.attachments {
        let filename = att.get("filename").and_then(|v| v.as_str()).unwrap_or("unknown");
        let attachment_id = att.get("id").and_then(|v| v.as_str()).unwrap_or("0");
        let url = att.get("url").and_then(|v| v.as_str());
        let size = att.get("size").and_then(|v| v.as_u64()).unwrap_or(0);

        if size > 25 * 1024 * 1024 {
            warn!("Discord attachment {} is too large ({} bytes), skipping", filename, size);
            content.push_str(&format!("\n[attachment: {} - too large]", filename));
            continue;
        }

        if let Some(url) = url {
            match rest::download_attachment(url, filename, &config.user_id, attachment_id).await {
                Ok(path) => media_paths.push(path),
                Err(e) => {
                    warn!("Failed to download Discord attachment {}: {}", filename, e);
                    content.push_str(&format!("\n[attachment: {} - download failed]", filename));
                }
            }
        }
    }

    if content.trim().is_empty() && media_paths.is_empty() {
        return;
    }

    channel_tokens.insert(msg.channel_id.clone(), config.bot_token.clone());

    let typing_cancel = CancellationToken::new();
    typing_tokens.insert(msg.channel_id.clone(), typing_cancel.clone());
    let _typing_handle = rest::start_typing(
        config.bot_token.clone(),
        msg.channel_id.clone(),
        typing_cancel.clone(),
    );

    let is_owner = config
        .owner_discord_id
        .as_ref()
        .is_some_and(|oid| oid == &msg.sender_id);

    let mut metadata = HashMap::new();
    metadata.insert("sender_discord_name".to_string(), serde_json::json!(msg.sender_name));
    metadata.insert("is_owner".to_string(), serde_json::json!(is_owner));

    let event = InboundEvent {
        channel: "discord".to_string(),
        sender_id: user_id,
        chat_id: msg.channel_id,
        content,
        session_id,
        timestamp: Some(chrono::Utc::now()),
        media: media_paths,
        metadata,
    };
    crate::session::ensure_session_and_publish(state, event).await;

    let cancel = typing_cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(120)).await;
        cancel.cancel();
    });
}
