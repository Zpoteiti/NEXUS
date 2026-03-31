# Discord Channel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement Discord as a multi-bot channel in NEXUS, allowing each user to connect their own Discord bot with DM/Guild/Thread session support and whitelist-based access control.

**Architecture:** `DiscordChannel` implements the `Channel` trait and is registered once. Its `start()` spawns a `DiscordConnectionManager` that reads bot configs from the `discord_configs` DB table. For each enabled config, it spawns a `DiscordGatewayConn` tokio task that maintains a WebSocket connection to Discord's Gateway, handles the protocol (HELLO/IDENTIFY/Heartbeat/MESSAGE_CREATE), and publishes `InboundEvent`s to the MessageBus. Outbound messages are sent via Discord's REST API with message splitting and rate limit handling.

**Tech Stack:** tokio-tungstenite (Gateway WS), reqwest (REST API), sqlx (DB), dashmap (routing), tokio_util::sync::CancellationToken (lifecycle)

---

## File Structure

| Action | File | Responsibility |
|--------|------|----------------|
| Create | `channels/discord/mod.rs` | DiscordChannel (Channel trait impl), DiscordConnectionManager |
| Create | `channels/discord/gateway_conn.rs` | DiscordGatewayConn — single bot WS lifecycle |
| Create | `channels/discord/protocol.rs` | Discord Gateway opcodes, IDENTIFY payload, event parsing |
| Create | `channels/discord/rest.rs` | Discord REST API: send message, typing indicator, message splitting |
| Modify | `channels/mod.rs` | Change `pub mod discord;` to point at directory module |
| Modify | `db.rs` | Add discord_configs table + CRUD |
| Modify | `main.rs` | Register DiscordChannel in ChannelManager |
| Modify | `Cargo.toml` | Add tokio-util (CancellationToken) |

---

### Task 1: Add tokio-util dependency and discord_configs DB schema + CRUD

**Files:**
- Modify: `nexus-server/Cargo.toml`
- Modify: `nexus-server/src/db.rs`

- [ ] **Step 1: Add tokio-util to Cargo.toml**

Add after the `once_cell = "1"` line in `nexus-server/Cargo.toml`:

```toml
tokio-util = { version = "0.7", features = ["rt"] }
```

- [ ] **Step 2: Add DiscordConfig struct and discord_configs table to db.rs**

Add after the `User` struct definition in `db.rs`:

```rust
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DiscordConfig {
    pub user_id: String,
    pub bot_token: String,
    pub bot_user_id: Option<String>,
    pub enabled: bool,
    pub allowed_users: Vec<String>,
}
```

Add to the end of `init_db()`, before the final `Ok(())`:

```rust
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS discord_configs (
            user_id TEXT PRIMARY KEY REFERENCES users(user_id),
            bot_token TEXT NOT NULL,
            bot_user_id TEXT,
            enabled BOOLEAN NOT NULL DEFAULT TRUE,
            allowed_users TEXT[] NOT NULL DEFAULT '{}',
            created_at TIMESTAMPTZ DEFAULT NOW(),
            updated_at TIMESTAMPTZ DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;
```

- [ ] **Step 3: Add CRUD functions for discord_configs**

Add at the end of `db.rs`:

```rust
pub async fn get_all_discord_configs(
    db: &PgPool,
) -> Result<Vec<DiscordConfig>, sqlx::Error> {
    sqlx::query_as::<_, DiscordConfig>(
        r#"
        SELECT user_id, bot_token, bot_user_id, enabled, allowed_users
        FROM discord_configs
        WHERE enabled = TRUE
        "#,
    )
    .fetch_all(db)
    .await
}

pub async fn get_discord_config_by_user_id(
    db: &PgPool,
    user_id: &str,
) -> Result<Option<DiscordConfig>, sqlx::Error> {
    sqlx::query_as::<_, DiscordConfig>(
        r#"
        SELECT user_id, bot_token, bot_user_id, enabled, allowed_users
        FROM discord_configs
        WHERE user_id = $1
        "#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
}

pub async fn update_bot_user_id(
    db: &PgPool,
    user_id: &str,
    bot_user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE discord_configs
        SET bot_user_id = $1, updated_at = NOW()
        WHERE user_id = $2
        "#,
    )
    .bind(bot_user_id)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn upsert_discord_config(
    db: &PgPool,
    user_id: &str,
    bot_token: &str,
    allowed_users: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO discord_configs (user_id, bot_token, allowed_users)
        VALUES ($1, $2, $3)
        ON CONFLICT (user_id) DO UPDATE
        SET bot_token = $2, allowed_users = $3, updated_at = NOW()
        "#,
    )
    .bind(user_id)
    .bind(bot_token)
    .bind(allowed_users)
    .execute(db)
    .await?;
    Ok(())
}
```

- [ ] **Step 4: Verify compilation**

```bash
cd D:/GitHub/NEXUS && cargo check --package nexus-server
```
Expected: compiles (warnings OK)

- [ ] **Step 5: Commit**

```bash
cd D:/GitHub/NEXUS && git add nexus-server/Cargo.toml nexus-server/src/db.rs && git commit -m "feat: add discord_configs table and CRUD functions"
```

---

### Task 2: Discord Gateway protocol types

**Files:**
- Create: `nexus-server/src/channels/discord/protocol.rs`

- [ ] **Step 1: Create the discord module directory**

```bash
mkdir -p D:/GitHub/NEXUS/nexus-server/src/channels/discord
```

- [ ] **Step 2: Create protocol.rs with Gateway types and event parsing**

Create `nexus-server/src/channels/discord/protocol.rs`:

```rust
//! Discord Gateway protocol types and event parsing.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Discord Gateway opcodes
pub const OP_DISPATCH: u8 = 0;
pub const OP_HEARTBEAT: u8 = 1;
pub const OP_IDENTIFY: u8 = 2;
pub const OP_RECONNECT: u8 = 7;
pub const OP_INVALID_SESSION: u8 = 9;
pub const OP_HELLO: u8 = 10;
pub const OP_HEARTBEAT_ACK: u8 = 11;

/// Discord Gateway intents bitmask:
/// GUILDS (1<<0) | GUILD_MESSAGES (1<<9) | DIRECT_MESSAGES (1<<12) | MESSAGE_CONTENT (1<<15)
pub const INTENTS: u64 = (1 << 0) | (1 << 9) | (1 << 12) | (1 << 15);

pub const GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";

/// A raw Gateway frame
#[derive(Debug, Deserialize)]
pub struct GatewayFrame {
    pub op: u8,
    pub d: Option<Value>,
    pub s: Option<u64>,
    pub t: Option<String>,
}

/// Build a Heartbeat frame (opcode 1)
pub fn heartbeat_frame(seq: Option<u64>) -> String {
    serde_json::json!({
        "op": OP_HEARTBEAT,
        "d": seq
    }).to_string()
}

/// Build an Identify frame (opcode 2)
pub fn identify_frame(token: &str) -> String {
    serde_json::json!({
        "op": OP_IDENTIFY,
        "d": {
            "token": token,
            "intents": INTENTS,
            "properties": {
                "os": "nexus",
                "browser": "nexus",
                "device": "nexus"
            }
        }
    }).to_string()
}

/// Parsed READY event data
pub struct ReadyData {
    pub bot_user_id: String,
}

/// Parse the READY event's `d` payload to extract bot user ID
pub fn parse_ready(d: &Value) -> Option<ReadyData> {
    let user_id = d.get("user")?.get("id")?.as_str()?;
    Some(ReadyData {
        bot_user_id: user_id.to_string(),
    })
}

/// Parsed MESSAGE_CREATE event data
#[derive(Debug)]
pub struct MessageCreateData {
    pub message_id: String,
    pub channel_id: String,
    pub guild_id: Option<String>,
    pub thread_id: Option<String>,
    pub sender_id: String,
    pub sender_is_bot: bool,
    pub content: String,
    pub mentions: Vec<String>,
}

/// Parse a MESSAGE_CREATE event's `d` payload
pub fn parse_message_create(d: &Value) -> Option<MessageCreateData> {
    let message_id = d.get("id")?.as_str()?.to_string();
    let channel_id = d.get("channel_id")?.as_str()?.to_string();
    let guild_id = d.get("guild_id").and_then(|v| v.as_str()).map(String::from);
    let content = d.get("content")?.as_str()?.to_string();

    let author = d.get("author")?;
    let sender_id = author.get("id")?.as_str()?.to_string();
    let sender_is_bot = author.get("bot").and_then(|v| v.as_bool()).unwrap_or(false);

    let mentions: Vec<String> = d
        .get("mentions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|id| id.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Thread detection: if the message has a thread or is in a thread channel
    // Discord sends thread messages with channel_id = thread_id
    // We detect threads by checking if the channel type in the payload indicates a thread
    // For simplicity, check if "thread" field exists in the message (thread starter messages)
    // or if the message's channel is a thread (type 11 or 12 in channel object)
    let thread_id = d.get("thread")
        .and_then(|t| t.get("id"))
        .and_then(|v| v.as_str())
        .map(String::from);

    Some(MessageCreateData {
        message_id,
        channel_id,
        guild_id,
        thread_id,
        sender_id,
        sender_is_bot,
        content,
        mentions,
    })
}

/// Strip bot mention strings from content: `<@BOT_ID>` and `<@!BOT_ID>`
pub fn strip_mention(content: &str, bot_user_id: &str) -> String {
    content
        .replace(&format!("<@{}>", bot_user_id), "")
        .replace(&format!("<@!{}>", bot_user_id), "")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heartbeat_frame() {
        let frame = heartbeat_frame(Some(42));
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["op"], 1);
        assert_eq!(v["d"], 42);
    }

    #[test]
    fn test_heartbeat_frame_null_seq() {
        let frame = heartbeat_frame(None);
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["op"], 1);
        assert!(v["d"].is_null());
    }

    #[test]
    fn test_identify_frame() {
        let frame = identify_frame("my-token");
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["op"], 2);
        assert_eq!(v["d"]["token"], "my-token");
        assert_eq!(v["d"]["intents"], INTENTS);
    }

    #[test]
    fn test_parse_ready() {
        let d = serde_json::json!({
            "user": {"id": "12345", "username": "nexus-bot"}
        });
        let ready = parse_ready(&d).unwrap();
        assert_eq!(ready.bot_user_id, "12345");
    }

    #[test]
    fn test_parse_message_create_dm() {
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "hello",
            "author": {"id": "user1", "username": "bob"},
            "mentions": []
        });
        let msg = parse_message_create(&d).unwrap();
        assert_eq!(msg.channel_id, "ch1");
        assert_eq!(msg.sender_id, "user1");
        assert!(!msg.sender_is_bot);
        assert!(msg.guild_id.is_none());
    }

    #[test]
    fn test_parse_message_create_guild_with_mention() {
        let d = serde_json::json!({
            "id": "msg2",
            "channel_id": "ch2",
            "guild_id": "guild1",
            "content": "<@botid> do something",
            "author": {"id": "user2", "username": "alice"},
            "mentions": [{"id": "botid", "username": "nexus-bot"}]
        });
        let msg = parse_message_create(&d).unwrap();
        assert_eq!(msg.guild_id, Some("guild1".to_string()));
        assert!(msg.mentions.contains(&"botid".to_string()));
    }

    #[test]
    fn test_strip_mention() {
        assert_eq!(strip_mention("<@123> hello", "123"), "hello");
        assert_eq!(strip_mention("<@!123> hello", "123"), "hello");
        assert_eq!(strip_mention("hello <@123> world", "123"), "hello  world");
        assert_eq!(strip_mention("no mention", "123"), "no mention");
    }
}
```

- [ ] **Step 3: Verify compilation**

```bash
cd D:/GitHub/NEXUS && cargo check --package nexus-server
```
Expected: will error because `channels/discord` module structure not yet set up. That's fine — commit protocol.rs alone.

- [ ] **Step 4: Commit**

```bash
cd D:/GitHub/NEXUS && git add nexus-server/src/channels/discord/protocol.rs && git commit -m "feat: add Discord Gateway protocol types and event parsing"
```

---

### Task 3: Discord REST API helpers

**Files:**
- Create: `nexus-server/src/channels/discord/rest.rs`

- [ ] **Step 1: Create rest.rs with send_message, start/stop typing, and message splitting**

Create `nexus-server/src/channels/discord/rest.rs`:

```rust
//! Discord REST API helpers: send messages, typing indicator, message splitting.

use reqwest::Client;
use std::sync::LazyLock;
use tracing::{debug, warn};

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";
const MAX_MESSAGE_LENGTH: usize = 2000;
const MAX_RETRIES: usize = 3;

static HTTP_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("failed to create Discord HTTP client")
});

/// Send a message to a Discord channel. Handles message splitting and 429 rate limits.
pub async fn send_message(
    bot_token: &str,
    channel_id: &str,
    content: &str,
) -> Result<(), String> {
    let chunks = split_message(content, MAX_MESSAGE_LENGTH);

    for chunk in &chunks {
        send_single_message(bot_token, channel_id, chunk).await?;
    }

    Ok(())
}

/// Send a single message (must be <= 2000 chars). Retries on 429.
async fn send_single_message(
    bot_token: &str,
    channel_id: &str,
    content: &str,
) -> Result<(), String> {
    let url = format!("{}/channels/{}/messages", DISCORD_API_BASE, channel_id);

    for attempt in 0..MAX_RETRIES {
        let response = HTTP_CLIENT
            .post(&url)
            .header("Authorization", format!("Bot {}", bot_token))
            .json(&serde_json::json!({ "content": content }))
            .send()
            .await
            .map_err(|e| format!("Discord send error: {}", e))?;

        let status = response.status().as_u16();

        if status == 200 || status == 201 {
            return Ok(());
        }

        if status == 429 {
            let body: serde_json::Value = response
                .json()
                .await
                .unwrap_or_else(|_| serde_json::json!({"retry_after": 1.0}));
            let retry_after = body
                .get("retry_after")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0);
            warn!(
                "Discord 429 rate limited (attempt {}/{}), retrying after {:.1}s",
                attempt + 1,
                MAX_RETRIES,
                retry_after
            );
            tokio::time::sleep(std::time::Duration::from_secs_f64(retry_after)).await;
            continue;
        }

        let body = response.text().await.unwrap_or_default();
        return Err(format!("Discord API error {}: {}", status, body));
    }

    Err("Discord send: max retries exceeded on 429".to_string())
}

/// Start typing indicator in a channel. Returns a JoinHandle that can be aborted to stop.
pub fn start_typing(
    bot_token: String,
    channel_id: String,
    cancel: tokio_util::sync::CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let url = format!("{}/channels/{}/typing", DISCORD_API_BASE, channel_id);
        loop {
            let _ = HTTP_CLIENT
                .post(&url)
                .header("Authorization", format!("Bot {}", bot_token))
                .send()
                .await;

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(8)) => {}
                _ = cancel.cancelled() => break,
            }
        }
    })
}

/// Split a message into chunks of at most `max_len` characters.
/// Prefers splitting at newline boundaries.
pub fn split_message(content: &str, max_len: usize) -> Vec<String> {
    if content.len() <= max_len {
        return vec![content.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = content;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        // Try to find a newline within the limit to split at
        let search_region = &remaining[..max_len];
        let split_at = search_region
            .rfind('\n')
            .map(|pos| pos + 1) // include the newline in the current chunk
            .unwrap_or(max_len); // no newline found, hard split

        chunks.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..];
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_message_short() {
        let chunks = split_message("hello", 2000);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn test_split_message_exact() {
        let msg = "a".repeat(2000);
        let chunks = split_message(&msg, 2000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 2000);
    }

    #[test]
    fn test_split_message_at_newline() {
        let msg = format!("{}\n{}", "a".repeat(1500), "b".repeat(1000));
        let chunks = split_message(&msg, 2000);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], format!("{}\n", "a".repeat(1500)));
        assert_eq!(chunks[1], "b".repeat(1000));
    }

    #[test]
    fn test_split_message_hard_split() {
        let msg = "a".repeat(5000);
        let chunks = split_message(&msg, 2000);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 2000);
        assert_eq!(chunks[1].len(), 2000);
        assert_eq!(chunks[2].len(), 1000);
    }
}
```

- [ ] **Step 2: Commit**

```bash
cd D:/GitHub/NEXUS && git add nexus-server/src/channels/discord/rest.rs && git commit -m "feat: add Discord REST API helpers (send, typing, split)"
```

---

### Task 4: DiscordGatewayConn — single bot connection lifecycle

**Files:**
- Create: `nexus-server/src/channels/discord/gateway_conn.rs`

- [ ] **Step 1: Create gateway_conn.rs**

Create `nexus-server/src/channels/discord/gateway_conn.rs`:

```rust
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

use crate::agent_loop;
use crate::bus::InboundEvent;
use crate::db::{self, DiscordConfig};
use crate::state::AppState;

use super::protocol::*;
use super::rest;

/// Shared state for outbound routing: channel_id → bot_token
pub type ChannelTokenMap = Arc<DashMap<String, String>>;

/// Run a single Discord bot connection with auto-reconnect.
/// This is the top-level entry point spawned as a tokio task.
pub async fn run(
    config: DiscordConfig,
    state: Arc<AppState>,
    channel_tokens: ChannelTokenMap,
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
            result = run_once(&config, &state, &channel_tokens) => {
                match result {
                    Ok(()) => {
                        backoff = Duration::from_secs(1);
                        info!("DiscordGatewayConn [{}]: disconnected gracefully, reconnecting...", config.user_id);
                    }
                    Err(e) => {
                        error!("DiscordGatewayConn [{}]: error: {}. Reconnecting in {:?}", config.user_id, e, backoff);
                    }
                }
            }
        }

        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(max_backoff);
    }
}

/// Single connection attempt. Returns Ok(()) on graceful close, Err on failure.
async fn run_once(
    config: &DiscordConfig,
    state: &Arc<AppState>,
    channel_tokens: &ChannelTokenMap,
) -> Result<(), String> {
    info!("DiscordGatewayConn [{}]: connecting to Gateway", config.user_id);

    let (ws_stream, _) = connect_async(GATEWAY_URL)
        .await
        .map_err(|e| format!("connect failed: {}", e))?;

    let (mut ws_sink, mut ws_source) = ws_stream.split();

    // 1. Wait for HELLO
    let heartbeat_interval = match ws_source.next().await {
        Some(Ok(Message::Text(text))) => {
            let frame: GatewayFrame = serde_json::from_str(&text)
                .map_err(|e| format!("parse HELLO: {}", e))?;
            if frame.op != OP_HELLO {
                return Err(format!("expected HELLO (op 10), got op {}", frame.op));
            }
            let interval_ms = frame.d
                .as_ref()
                .and_then(|d| d.get("heartbeat_interval"))
                .and_then(|v| v.as_u64())
                .ok_or("missing heartbeat_interval in HELLO")?;
            info!("DiscordGatewayConn [{}]: HELLO, heartbeat_interval={}ms", config.user_id, interval_ms);
            interval_ms
        }
        Some(Ok(_)) => return Err("expected text frame for HELLO".to_string()),
        Some(Err(e)) => return Err(format!("ws error waiting for HELLO: {}", e)),
        None => return Err("ws closed before HELLO".to_string()),
    };

    // 2. Send IDENTIFY
    ws_sink
        .send(Message::Text(identify_frame(&config.bot_token).into()))
        .await
        .map_err(|e| format!("send IDENTIFY: {}", e))?;

    // 3. Start heartbeat task
    let seq: Arc<std::sync::atomic::AtomicU64> = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let seq_valid = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let heartbeat_cancel = CancellationToken::new();
    let hb_seq = seq.clone();
    let hb_seq_valid = seq_valid.clone();
    let hb_cancel = heartbeat_cancel.clone();

    // We need a channel to send heartbeats through the ws_sink
    let (hb_tx, mut hb_rx) = mpsc::channel::<String>(4);

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

    // Track bot_user_id (filled after READY)
    let mut bot_user_id: Option<String> = config.bot_user_id.clone();

    // 4. Read loop
    loop {
        tokio::select! {
            // Forward heartbeat frames to ws_sink
            Some(hb_frame) = hb_rx.recv() => {
                if let Err(e) = ws_sink.send(Message::Text(hb_frame.into())).await {
                    heartbeat_cancel.cancel();
                    return Err(format!("send heartbeat: {}", e));
                }
            }
            // Read from Gateway
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

                        // Update sequence number
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
                                                // Backfill bot_user_id to DB
                                                let _ = db::update_bot_user_id(&state.db, &config.user_id, &ready.bot_user_id).await;
                                            }
                                        }
                                    }
                                    "MESSAGE_CREATE" => {
                                        if let Some(d) = &frame.d {
                                            if let Some(msg_data) = parse_message_create(d) {
                                                handle_message(
                                                    config,
                                                    &state,
                                                    channel_tokens,
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
                            OP_HEARTBEAT_ACK => {} // expected, ignore
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
                    Some(Ok(_)) => {} // ping/pong/binary, ignore
                    Some(Err(e)) => {
                        heartbeat_cancel.cancel();
                        return Err(format!("ws read error: {}", e));
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

/// Handle a MESSAGE_CREATE event: permission check, session creation, publish to bus.
async fn handle_message(
    config: &DiscordConfig,
    state: &Arc<AppState>,
    channel_tokens: &ChannelTokenMap,
    bot_user_id: Option<&str>,
    msg: MessageCreateData,
) {
    // Ignore bot messages
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

    // Determine session type and check permissions
    let (session_id, user_id, content) = if msg.guild_id.is_some() {
        // Guild or Thread message — require @mention
        if !msg.mentions.contains(&bot_uid.to_string()) {
            return; // Not mentioned, ignore
        }

        // Check whitelist: empty = owner only (bot owner's discord sender_id not checked,
        // but we check if sender is in allowed_users, with owner always allowed)
        if !config.allowed_users.is_empty()
            && !config.allowed_users.contains(&msg.sender_id)
        {
            debug!("DiscordGatewayConn [{}]: sender {} not in whitelist, ignoring", config.user_id, msg.sender_id);
            return;
        }

        // Empty whitelist = only the bot owner. We need to check if the sender IS the owner.
        // We don't have the owner's discord_id directly, so empty whitelist = deny all non-listed.
        // The owner should add their own discord_id to allowed_users.
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
        (sid, config.user_id.clone(), clean_content)
    } else {
        // DM — check if sender is the bot owner
        // In our 1:1 model, the bot_token belongs to config.user_id
        // For DMs, we only allow the owner to interact
        // (Future: could look up sender_discord_id in a binding table)
        let sid = format!("discord:dm:{}", msg.sender_id);
        (sid, config.user_id.clone(), msg.content.clone())
    };

    if content.trim().is_empty() {
        return; // Skip empty messages (e.g., mention-only with no text)
    }

    // Register channel_id → bot_token for outbound routing
    channel_tokens.insert(msg.channel_id.clone(), config.bot_token.clone());

    // Start typing indicator
    let typing_cancel = CancellationToken::new();
    let _typing_handle = rest::start_typing(
        config.bot_token.clone(),
        msg.channel_id.clone(),
        typing_cancel.clone(),
    );

    // Create session if new, publish to bus
    let (is_new, channels) = state
        .session_manager
        .get_or_create_session(&session_id)
        .await;

    if is_new {
        if let Some((inbox_tx, inbox_rx)) = channels {
            state.bus.register_session(session_id.clone(), inbox_tx);
            let state_clone = state.clone();
            let sid = session_id.clone();
            tokio::spawn(async move {
                agent_loop::run_session(sid, inbox_rx, state_clone).await;
            });
        }
    }

    let event = InboundEvent {
        channel: "discord".to_string(),
        sender_id: user_id,
        chat_id: msg.channel_id,
        content,
        session_id,
        timestamp: Some(chrono::Utc::now()),
        media: Vec::new(),
        metadata: {
            let mut m = HashMap::new();
            m.insert("typing_cancel_token".to_string(), serde_json::json!("active"));
            m
        },
    };
    state.bus.publish_inbound(event).await;

    // Note: typing indicator will be cancelled when the response is sent.
    // For now, set a timeout to auto-cancel after 120s to prevent leaks.
    let cancel = typing_cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(120)).await;
        cancel.cancel();
    });
}
```

- [ ] **Step 2: Commit**

```bash
cd D:/GitHub/NEXUS && git add nexus-server/src/channels/discord/gateway_conn.rs && git commit -m "feat: implement DiscordGatewayConn single-bot WS lifecycle"
```

---

### Task 5: DiscordChannel and DiscordConnectionManager

**Files:**
- Create: `nexus-server/src/channels/discord/mod.rs`
- Modify: `nexus-server/src/channels/mod.rs`

- [ ] **Step 1: Create discord/mod.rs**

Create `nexus-server/src/channels/discord/mod.rs`:

```rust
//! Discord Channel — multi-bot architecture.
//! DiscordChannel implements the Channel trait. Its start() spawns a DiscordConnectionManager
//! that reads bot configs from DB and manages one DiscordGatewayConn per enabled bot.

pub mod protocol;
pub mod rest;
pub mod gateway_conn;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::channels::Channel;
use crate::db;
use crate::state::AppState;

use gateway_conn::ChannelTokenMap;

/// Per-connection tracking info
struct ConnHandle {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}

/// DiscordChannel — registered once in ChannelManager.
pub struct DiscordChannel {
    state: Arc<AppState>,
    /// channel_id → bot_token mapping for outbound routing
    channel_tokens: ChannelTokenMap,
    /// Shared cancellation token for the entire Discord subsystem
    shutdown: CancellationToken,
}

impl DiscordChannel {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            channel_tokens: Arc::new(DashMap::new()),
            shutdown: CancellationToken::new(),
        }
    }
}

#[async_trait::async_trait]
impl Channel for DiscordChannel {
    fn name(&self) -> &str {
        "discord"
    }

    async fn start(&self) {
        info!("DiscordChannel: starting connection manager");
        run_connection_manager(
            self.state.clone(),
            self.channel_tokens.clone(),
            self.shutdown.clone(),
        )
        .await;
    }

    async fn stop(&self) {
        info!("DiscordChannel: stopping all connections");
        self.shutdown.cancel();
    }

    async fn send(&self, chat_id: &str, content: &str) -> Result<(), String> {
        // Look up which bot token owns this channel_id
        let bot_token = self
            .channel_tokens
            .get(chat_id)
            .map(|v| v.value().clone())
            .ok_or_else(|| format!("no bot token mapped for channel_id {}", chat_id))?;

        rest::send_message(&bot_token, chat_id, content).await
    }
}

/// Connection manager: polls DB for discord_configs, spawns/cancels connections dynamically.
async fn run_connection_manager(
    state: Arc<AppState>,
    channel_tokens: ChannelTokenMap,
    shutdown: CancellationToken,
) {
    let mut connections: HashMap<String, ConnHandle> = HashMap::new();
    let poll_interval = Duration::from_secs(30);

    // Shared semaphore to rate-limit IDENTIFY (1 per 5 seconds)
    let identify_semaphore = Arc::new(tokio::sync::Semaphore::new(1));

    loop {
        // Load configs from DB
        let configs = match db::get_all_discord_configs(&state.db).await {
            Ok(c) => c,
            Err(e) => {
                error!("DiscordConnectionManager: failed to load configs: {}", e);
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = tokio::time::sleep(poll_interval) => continue,
                }
            }
        };

        let active_user_ids: std::collections::HashSet<String> =
            configs.iter().map(|c| c.user_id.clone()).collect();

        // Cancel connections for removed/disabled configs
        connections.retain(|user_id, conn| {
            if !active_user_ids.contains(user_id) {
                info!("DiscordConnectionManager: stopping connection for user {}", user_id);
                conn.cancel.cancel();
                false
            } else {
                true
            }
        });

        // Spawn new connections for configs without an active connection
        for config in configs {
            if connections.contains_key(&config.user_id) {
                continue;
            }

            info!("DiscordConnectionManager: spawning connection for user {}", config.user_id);

            let cancel = CancellationToken::new();
            let state_clone = state.clone();
            let ct_clone = channel_tokens.clone();
            let cancel_clone = cancel.clone();
            let sem_clone = identify_semaphore.clone();

            let handle = tokio::spawn(async move {
                // Acquire semaphore to rate-limit IDENTIFY calls across bots
                // Hold it for 5 seconds after acquiring to enforce the rate limit
                let _permit = sem_clone.acquire().await;
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    drop(_permit);
                });

                gateway_conn::run(config, state_clone, ct_clone, cancel_clone).await;
            });

            connections.insert(
                config.user_id.clone(),
                ConnHandle {
                    cancel,
                    handle,
                },
            );
        }

        // Wait before next poll, or shutdown
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = tokio::time::sleep(poll_interval) => {}
        }
    }

    // Clean up all connections
    for (user_id, conn) in &connections {
        info!("DiscordConnectionManager: shutting down connection for user {}", user_id);
        conn.cancel.cancel();
    }
}
```

- [ ] **Step 2: Update channels/mod.rs**

The existing `channels/mod.rs` already has `pub mod discord;`. Since we're changing from a single file to a directory module, we just need to delete the old `discord.rs` stub file:

```bash
rm D:/GitHub/NEXUS/nexus-server/src/channels/discord.rs
```

The `pub mod discord;` declaration in `channels/mod.rs` will now resolve to `channels/discord/mod.rs`.

- [ ] **Step 3: Verify compilation**

```bash
cd D:/GitHub/NEXUS && cargo check --package nexus-server
```
Expected: compiles (warnings OK). The DiscordChannel is defined but not yet wired into main.rs.

- [ ] **Step 4: Run tests**

```bash
cd D:/GitHub/NEXUS && cargo test --package nexus-server
```
Expected: all existing tests pass + new protocol and rest tests pass.

- [ ] **Step 5: Commit**

```bash
cd D:/GitHub/NEXUS && git add -A nexus-server/src/channels/discord/ && git add -u nexus-server/src/channels/discord.rs && git commit -m "feat: implement DiscordChannel with multi-bot ConnectionManager"
```

---

### Task 6: Wire DiscordChannel into main.rs

**Files:**
- Modify: `nexus-server/src/main.rs`

- [ ] **Step 1: Register DiscordChannel in ChannelManager**

In `main.rs`, add the import at the top (after the existing `use channels::gateway::GatewayChannel;`):

```rust
use channels::discord::DiscordChannel;
```

Then after the line `channel_manager.register(GatewayChannel::new(state_arc));` (around line 61), add:

```rust
    channel_manager.register(DiscordChannel::new(state_arc.clone()));
```

Note: `state_arc` is used by both GatewayChannel and DiscordChannel, so we need `.clone()` on the second usage. Check if the existing code already clones — if `GatewayChannel::new(state_arc)` consumes the Arc, you need `state_arc.clone()` for it too. Since `Arc::clone` is just a reference count bump, this is fine.

Actually, looking at the existing code:
```rust
let state_arc = Arc::new(state.clone());
let mut channel_manager = ChannelManager::new(bus);
channel_manager.register(GatewayChannel::new(state_arc));
```

`state_arc` is moved into `GatewayChannel::new()`. We need to clone it first:

```rust
    channel_manager.register(GatewayChannel::new(state_arc.clone()));
    channel_manager.register(DiscordChannel::new(state_arc));
```

- [ ] **Step 2: Verify compilation**

```bash
cd D:/GitHub/NEXUS && cargo check --package nexus-server
```
Expected: compiles

- [ ] **Step 3: Verify all tests pass**

```bash
cd D:/GitHub/NEXUS && cargo test --package nexus-server
```
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
cd D:/GitHub/NEXUS && git add nexus-server/src/main.rs && git commit -m "feat: register DiscordChannel in ChannelManager startup"
```

---

### Task 7: Insert test discord_config and manual E2E verification

**Files:** None (manual verification)

- [ ] **Step 1: Ensure PostgreSQL is running with NEXUS database**

- [ ] **Step 2: Insert a test discord_config into the database**

You need a NEXUS user_id (from the users table) and a Discord bot token. Run:

```sql
INSERT INTO discord_configs (user_id, bot_token, allowed_users)
VALUES ('YOUR_NEXUS_USER_ID', 'YOUR_DISCORD_BOT_TOKEN', '{}');
```

- [ ] **Step 3: Start nexus-server**

```bash
cd D:/GitHub/NEXUS && cargo run --package nexus-server
```

Expected logs:
- `DiscordChannel: starting connection manager`
- `DiscordConnectionManager: spawning connection for user YOUR_USER_ID`
- `DiscordGatewayConn [user_id]: connecting to Gateway`
- `DiscordGatewayConn [user_id]: HELLO, heartbeat_interval=...`
- `DiscordGatewayConn [user_id]: READY, bot_user_id=...`

- [ ] **Step 4: Start nexus-client (for tool execution)**

```bash
cd D:/GitHub/NEXUS && cargo run --package nexus-client
```

- [ ] **Step 5: Send a DM to the Discord bot**

Send a direct message to the bot. Expected:
1. Server log: `handle_message` processing the DM
2. Server log: LLM call to MiniMax
3. Bot replies in Discord with the LLM response

- [ ] **Step 6: Verify typing indicator**

While the bot is processing, the Discord UI should show "Bot is typing..."

- [ ] **Step 7: Verify message splitting**

Send a prompt that produces a long response (>2000 chars). The bot should split it into multiple messages.
