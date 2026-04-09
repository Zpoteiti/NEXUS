# M2c+M2d: Server MCP + Channels Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Complete M2 — server MCP, gateway stub channel, Discord channel, Telegram channel, outbound dispatch routing, discord/telegram API endpoints.

**Architecture:** Channel trait pattern — each channel converts incoming messages to InboundEvents and delivers OutboundEvents. Per-user bots for Discord and Telegram. Server MCP via rmcp.

**Tech Stack:** rmcp (MCP), serenity (Discord), teloxide (Telegram), tokio-tungstenite (gateway WS client)

**Depends on:** M2a + M2b (all server infrastructure)

---

## File Map

| File | Responsibility |
|---|---|
| `nexus-server/src/server_mcp.rs` | Server-side MCP manager (admin-configured, rmcp) |
| `nexus-server/src/channels/mod.rs` | Channel trait, outbound dispatch loop |
| `nexus-server/src/channels/gateway.rs` | Gateway WebSocket client (stub for M4) |
| `nexus-server/src/channels/discord.rs` | Discord per-user bots (serenity) |
| `nexus-server/src/channels/telegram.rs` | Telegram per-user bots (teloxide) |
| `nexus-server/src/auth/discord_api.rs` | Discord config CRUD endpoints |
| `nexus-server/src/auth/telegram_api.rs` | Telegram config CRUD endpoints |
| `nexus-server/src/db/telegram.rs` | Telegram configs DB CRUD |

---

### Task 1: Server MCP Manager

**Files:** Create `server_mcp.rs`, modify `state.rs`, `main.rs`, `auth/admin.rs`

Server MCP: admin configures MCP servers via `PUT /api/server-mcp`. Uses rmcp (same as nexus-client). Tools appear with `device_name="server"` in schema. Lifecycle: init on startup from DB, reinit on admin update.

### Task 2: Channel Trait + Outbound Dispatch

**Files:** Create `channels/mod.rs`, modify `main.rs`

Replace the drain task with real outbound routing. Channel trait with `deliver(event)`. Dispatch loop matches `event.channel` to the right handler.

### Task 3: Gateway Channel (Stub)

**Files:** Create `channels/gateway.rs`

WebSocket client connecting to `NEXUS_GATEWAY_WS_URL`. Auth with `NEXUS_GATEWAY_TOKEN`. Inbound: receive user messages, publish to bus. Outbound: send agent responses. Stub: connect + auth, log messages, reconnect with backoff. Full implementation in M4 when gateway exists.

### Task 4: Discord Channel + API

**Files:** Create `channels/discord.rs`, `auth/discord_api.rs`, modify `auth/mod.rs`, `db/mod.rs`

Per-user Discord bots via serenity. On config create: spawn bot. On delete: stop bot. Security: owner vs non-owner via ChannelIdentity. Group support with mention/reply detection.

API: POST/GET/DELETE `/api/discord-config`

### Task 5: Telegram Channel + API + DB

**Files:** Create `channels/telegram.rs`, `auth/telegram_api.rs`, `db/telegram.rs`

Per-user Telegram bots via teloxide. Long polling. Group policy: respond only when @mentioned. DMs always allowed. Access control: owner + allowed_users list.

DB table: `telegram_configs` (user_id, bot_token, owner_telegram_id, allowed_users, enabled, group_policy, created_at, updated_at)

API: POST/GET/DELETE `/api/telegram-config`

### Task 6: Wire Everything + Integration Test

Merge all routes, spawn all channels, test outbound routing.
