# Discord Channel — Multi-Bot Design Spec

**Date:** 2026-03-31
**Status:** Approved
**Goal:** Implement Discord as a real channel in NEXUS, supporting multiple users each with their own Discord bot, DM/Guild/Thread sessions, whitelist-based access control, and @mention-only guild responses.

## Architecture

```
DiscordChannel (implements Channel trait, registered once)
  └─ DiscordConnectionManager
       ├─ User A's bot → DiscordGatewayConn (tokio task)
       │    ├─ WS: Discord Gateway (receive events)
       │    └─ HTTP: Discord REST API (send messages)
       ├─ User B's bot → DiscordGatewayConn (tokio task)
       └─ ...
       Lifecycle: DB-driven, poll every 30s for changes
```

One NEXUS server manages N Discord bots simultaneously. Each bot belongs to one NEXUS user. Connections are spawned/cancelled dynamically based on `discord_configs` table state.

## DB Schema

### New table: `discord_configs`

```sql
CREATE TABLE IF NOT EXISTS discord_configs (
    user_id TEXT PRIMARY KEY REFERENCES users(user_id),
    bot_token TEXT NOT NULL,
    bot_user_id TEXT,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    allowed_users TEXT[] NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);
```

| Column | Purpose |
|--------|---------|
| `user_id` | FK to users table. One bot per user (1:1). |
| `bot_token` | Discord bot token. |
| `bot_user_id` | Auto-filled on READY event. Used for @mention detection. |
| `enabled` | Toggle without deleting config. |
| `allowed_users` | Discord user IDs allowed to trigger bot in guilds. Empty = bot owner only. |

### New DB functions

- `get_all_discord_configs(pool) -> Vec<DiscordConfig>` — all enabled configs
- `get_discord_config_by_bot_user_id(pool, bot_user_id) -> Option<DiscordConfig>` — lookup by bot identity
- `update_bot_user_id(pool, user_id, bot_user_id)` — backfill after READY
- `upsert_discord_config(pool, user_id, bot_token, allowed_users)` — for future admin API
- `delete_discord_config(pool, user_id)` — for future admin API

## Session Mapping

| Scenario | session_id | user_id (owner) | Response trigger |
|----------|-----------|-----------------|-----------------|
| DM | `discord:dm:{sender_discord_id}` | Lookup sender's discord_id in discord_configs → user_id | All messages (if sender is a known NEXUS user) |
| Guild Channel | `discord:guild:{channel_id}` | Bot owner's user_id | @mention + sender in allowed_users |
| Thread | `discord:thread:{thread_id}` | Bot owner's user_id | @mention + sender in allowed_users |

## Permission Check Flow

```
MESSAGE_CREATE received
  │
  ├─ author.bot == true → IGNORE
  │
  ├─ guild_id is None (DM)
  │    └─ sender discord_id has matching discord_configs entry?
  │         ├─ Yes → PROCESS (user_id = that config's user_id)
  │         └─ No → IGNORE
  │
  └─ guild_id is Some (Guild/Thread)
       └─ message mentions this bot's bot_user_id?
            ├─ No → IGNORE
            └─ Yes → sender in allowed_users? (empty = owner-only)
                 ├─ Yes → PROCESS (user_id = bot owner)
                 └─ No → IGNORE
```

For guild messages, "bot owner" is the NEXUS user whose discord_configs row contains the bot_token that this connection is using.

## Discord Gateway Protocol

### Connection lifecycle (per bot)

1. **Connect** to `wss://gateway.discord.gg/?v=10&encoding=json` via tokio-tungstenite
2. **HELLO** (opcode 10): Extract `heartbeat_interval`, start heartbeat task
3. **IDENTIFY** (opcode 2): Send bot token + intents (37377 = GUILDS | GUILD_MESSAGES | MESSAGE_CONTENT | DIRECT_MESSAGES)
4. **READY** (opcode 0, type "READY"): Extract `user.id` as `bot_user_id`, backfill to DB
5. **MESSAGE_CREATE** (opcode 0): Route through permission check → InboundEvent → MessageBus
6. **RECONNECT** (opcode 7) / **INVALID_SESSION** (opcode 9): Break inner loop, reconnect
7. **Heartbeat** (opcode 1): Send `{"op": 1, "d": last_sequence}` at server-specified interval

### Reconnection

Exponential backoff: 1s → 2s → 4s → ... → 60s max. Reset to 1s on successful connection.

### IDENTIFY rate limit

Discord limits IDENTIFY to 1 per 5 seconds globally. DiscordConnectionManager must serialize IDENTIFY calls across all bots with a shared semaphore/timer. On startup with N bots, connections are staggered ~5s apart.

## Sending Messages (Discord REST API)

### Endpoint

```
POST https://discord.com/api/v10/channels/{channel_id}/messages
Authorization: Bot {bot_token}
Content-Type: application/json

{"content": "message text"}
```

### Message splitting

Discord has a 2000-character limit. Messages exceeding this are split:
- Prefer splitting at newline boundaries
- Each chunk sent as a separate POST
- First chunk carries `message_reference` for reply threading (if applicable)

### Rate limit handling

On HTTP 429:
1. Read `retry_after` from response JSON body
2. Sleep for that duration
3. Retry (max 3 attempts)

### Typing indicator

- On receiving a message, start POSTing to `POST /channels/{channel_id}/typing` every 8 seconds
- Cancel when reply is fully sent
- Per-channel: one typing task per active channel, tracked in a HashMap

## Outbound Routing

DiscordChannel maintains a shared `DashMap<String, usize>` mapping `channel_id → connection_index`. Each DiscordGatewayConn registers channel_ids it has seen messages from. When `send(chat_id, content)` is called, it looks up which bot connection owns that channel_id and uses that bot's token for the HTTP request.

Shared reqwest Client across all connections (connection pooling).

## DiscordConnectionManager

### Startup

1. Query `discord_configs` for all `enabled = true` rows
2. For each config, spawn a `DiscordGatewayConn` task (staggered by 5s for IDENTIFY rate limit)
3. Store `JoinHandle` + `CancellationToken` per connection

### Polling for changes

Every 30 seconds, re-query `discord_configs`:
- New enabled config found → spawn new connection
- Existing config disabled/deleted → cancel task, drop connection
- Token changed → cancel old, spawn new

### Shutdown

Cancel all connection tasks, close all WS connections.

## File Structure

| Action | File | Responsibility |
|--------|------|----------------|
| Rewrite | `channels/discord.rs` | DiscordChannel, DiscordConnectionManager, DiscordGatewayConn |
| Modify | `channels/mod.rs` | Ensure `pub mod discord;` is declared |
| Modify | `db.rs` | Add discord_configs table + CRUD functions |
| Modify | `main.rs` | Register DiscordChannel in ChannelManager |
| No change | `Cargo.toml` | tokio-tungstenite and reqwest already available |

## Content stripping for @mentions

When the bot is @mentioned in a guild, the message content contains `<@BOT_USER_ID>` or `<@!BOT_USER_ID>`. Before passing to the agent loop, strip these mention strings from the content so the LLM sees clean text.

## What is NOT in scope

- Attachment/media handling (future)
- Discord slash commands (future)
- RESUME on reconnect (fresh IDENTIFY each time, like nanobot)
- Streaming/delta responses (Discord doesn't support editing messages as a stream well)
- Admin API for managing discord_configs (future, currently hardcoded or direct DB insert)
