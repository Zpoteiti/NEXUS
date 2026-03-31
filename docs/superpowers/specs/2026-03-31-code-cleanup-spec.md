# M2 Code Cleanup Spec

**Date:** 2026-03-31
**Goal:** Address all 23 issues found in the M2 branch code review to improve simplicity, remove dead code, and fix inconsistencies.

---

## C1. API Key Hardcoding

**Status:** DEFERRED. By design, LLM config will be managed via admin API (not env vars). No action now.

## C2. Session Creation Logic Duplicated

**Problem:** `GatewayChannel::handle_inbound` and `DiscordChannel::handle_message` both do:
```
get_or_create_session → register_session → spawn(agent_loop) → publish_inbound
```

**Fix:** Extract into a method on `SessionManager` or a free function in `bus.rs`:

```rust
// In session.rs or a new helper module
pub async fn ensure_session_and_publish(
    state: &Arc<AppState>,
    event: InboundEvent,
) {
    let session_id = &event.session_id;
    let (is_new, channels) = state.session_manager.get_or_create_session(session_id).await;
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
    state.bus.publish_inbound(event).await;
}
```

Then both channels call `ensure_session_and_publish(state, event).await;` — a one-liner.

**Files:** `nexus-server/src/session.rs`, `channels/gateway.rs`, `channels/discord/gateway_conn.rs`

---

## I1. Unused `once_cell` Dependency

**Problem:** `Cargo.toml` has `once_cell = "1"` but code uses `std::sync::LazyLock`.

**Fix:** Remove `once_cell = "1"` from `nexus-server/Cargo.toml`.

**Files:** `nexus-server/Cargo.toml`

---

## I2. Unused Import `PgPool` in `context.rs`

**Fix:** Remove `use sqlx::PgPool;` (line 10).

**Files:** `nexus-server/src/context.rs`

---

## I3. Unused Variables in `context.rs` Loops

**Fix:** Change `for (device_id, device_state)` to `for (_device_id, device_state)` at lines 90 and 122. Or better, just `for device_state in devices.values()` since the key isn't used.

**Files:** `nexus-server/src/context.rs`

---

## I4. Unused Functions in `db.rs`

| Function | Verdict |
|----------|---------|
| `update_device_name` | **DELETE** — device_name no longer updated by client after protocol refactor |
| `create_session` | **DELETE** — agent_loop uses raw SQL with `ON CONFLICT DO NOTHING` which this function doesn't support. Will be replaced by `ensure_session` (see I11) |
| `get_discord_config_by_user_id` | **KEEP** — will be needed by admin API soon |
| `upsert_discord_config` | Already used by auth.rs API. No change. |

**Files:** `nexus-server/src/db.rs`

---

## I5. Unused Methods in `session.rs`

| Method | Verdict |
|--------|---------|
| `remove_session` | **KEEP** — needed for session cleanup (see I6). Currently no one calls it, but it should be called when sessions end. |
| `list_sessions` | **DELETE** — no use case. Can be re-added when admin API needs it. |

**Files:** `nexus-server/src/session.rs`

---

## I6. Unused Methods in `bus.rs` + Session Memory Leak

**Problem:** `unregister_session` and `shutdown` exist but are never called. When an agent_loop session ends (inbox channel closed), the session's entry in `bus.inbound_routes` is never removed → slow memory leak.

**Fix:** In `agent_loop::run_session`, when the `while let Some(event) = inbox.recv().await` loop exits (inbox closed), call:
```rust
state.bus.unregister_session(&session_id);
state.session_manager.remove_session(&session_id).await;
```

Keep `shutdown()` — it will be useful for graceful server shutdown.

**Files:** `nexus-server/src/agent_loop.rs`, (bus.rs and session.rs already have the methods)

---

## I7. Confusing `mut state` + `Arc::clone` Pattern in `main.rs`

**Problem:** `state` is declared `mut`, cloned into `state_arc`, then `state.channel_manager_handle` is mutated. Works by accident because `channel_manager_handle` is `Arc<RwLock<...>>` (shared interior mutability). But confusing.

**Fix:** Remove `mut`, create `state_arc` after construction, write to `channel_manager_handle` via the Arc:
```rust
let state = state::AppState::new(pool, config.clone(), bus.clone(), session_manager);
let state_arc = Arc::new(state);
// ... register channels with state_arc.clone() ...
*state_arc.channel_manager_handle.write().await = Some(channel_manager_handle);
// Pass state_arc.as_ref().clone() to axum (AppState is Clone via Arc internals)
```

Actually simpler: `AppState` is already `Clone` (all fields are `Arc`). Just use `state_arc` everywhere:
```rust
let state = state::AppState::new(pool, config.clone(), bus.clone(), session_manager);
let state_arc = Arc::new(state);

let mut channel_manager = ChannelManager::new(bus);
channel_manager.register(GatewayChannel::new(state_arc.clone()));
channel_manager.register(DiscordChannel::new(state_arc.clone()));
let channel_manager_handle = channel_manager.start();

*state_arc.channel_manager_handle.write().await = Some(channel_manager_handle);

let app = Router::new()
    // ...
    .with_state((*state_arc).clone());  // AppState: Clone
```

**Files:** `nexus-server/src/main.rs`

---

## I8. Unused Config Fields (`gateway_ws_url`, `gateway_token`)

**Problem:** `ServerConfig` stores `gateway_ws_url` and `gateway_token`, but `GatewayChannel::new()` reads env vars directly.

**Fix:** Make `GatewayChannel::new()` read from `state.config` instead of `std::env::var`:
```rust
pub fn new(state: Arc<AppState>) -> Self {
    Self {
        ws_url: state.config.gateway_ws_url.clone(),
        token: state.config.gateway_token.clone(),
        state,
        ws_out: Arc::new(RwLock::new(None)),
    }
}
```

**Files:** `nexus-server/src/channels/gateway.rs`

---

## I9. `BrowserOutbound::Connected` Never Constructed (nexus-gateway)

**Verdict:** SKIP — this is in `nexus-gateway` crate which is a separate concern. Not blocking.

---

## I10. Commented-Out Module Declarations

**Problem:** `main.rs` has `// mod api;` and `// mod memory;`.

**Fix:** Remove both commented-out lines. The actual stub files (`api.rs`, `memory.rs`) should also be deleted if they exist and only contain TODOs.

**Files:** `nexus-server/src/main.rs`, potentially delete `api.rs` and `memory.rs`

---

## I11. Raw SQL in `agent_loop.rs` Bypassing `db.rs`

**Problem:** `agent_loop.rs` lines 40-48 use `sqlx::query()` directly to create sessions, violating the "all DB access through db.rs" pattern.

**Fix:** Add an `ensure_session` function to `db.rs`:
```rust
pub async fn ensure_session(
    db: &PgPool,
    session_id: &str,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sessions (session_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"
    )
    .bind(session_id)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(())
}
```

Then in `agent_loop.rs` replace the raw SQL with `db::ensure_session(&state.db, &session_id, &event.sender_id).await`.

Also delete the old `create_session` function since `ensure_session` replaces it.

**Files:** `nexus-server/src/db.rs`, `nexus-server/src/agent_loop.rs`

---

## I12. Empty `tools` on Follow-Up LLM Calls

**Problem:** After tool execution, the follow-up LLM call passes `tools: vec![]`. This prevents the LLM from chaining multiple tool calls across turns.

**Fix:** Pass the full tools schema to follow-up calls. In `execute_tool_calls_loop`, capture the tools at the start and pass them through:

In `run_single_turn`, pass `tools.clone()` to `execute_tool_calls_loop`.
In `execute_tool_calls_loop`, use the passed tools in all subsequent LLM requests instead of `vec![]`.

**Files:** `nexus-server/src/agent_loop.rs`

---

## M1. Stale `device_id` References in Comments

**Fix:** Find-replace in comments across `state.rs` and `tools_registry.rs`:
- `device_id` → `device_key` (when referring to the internal routing key, which is now the token)
- Update the docstring at top of `state.rs` to reflect new key semantics
- Update `cancel_pending_requests_for_device` parameter name from `device_id` to `device_key`

**Files:** `nexus-server/src/state.rs`, `nexus-server/src/tools_registry.rs`

---

## M2. `ConnHandle.handle` Never Read

**Fix:** Prefix with underscore: `_handle: JoinHandle<()>`. The handle is kept alive to prevent the task from being detached, which is correct behavior — just mark it intentionally unused.

**Files:** `nexus-server/src/channels/discord/mod.rs`

---

## M3. Unused Fields on `InboundEvent` and `OutboundEvent`

**Verdict:** KEEP — these are needed for media/attachment support (near-term Discord feature). No change.

---

## M4. `embed_text` and `RAG_TOP_K` Unused in `context.rs`

**Fix:** Delete both. They are stubs with no implementation. Re-add when RAG is actually implemented.

Also remove the commented-out RAG injection code block in `build_system_prompt`.

**Files:** `nexus-server/src/context.rs`

---

## M5. Unused `HashMap` Import in `context.rs`

**Fix:** Remove `use std::collections::HashMap;` (line 9). Already covered by I2 cleanup.

**Files:** `nexus-server/src/context.rs`

---

## M6. Error Type Inconsistency

**Verdict:** DEFER. Creating a unified `AgentError` type is a good idea but is a larger refactor. Not blocking for M3.

---

## M7. `DeviceOffline` Variant Never Constructed

**Fix:** Delete `DeviceOffline(String)` from `RouteError`. The current code only distinguishes "not found" and "send failed". Add offline detection later when heartbeat-based health checks are implemented.

**Files:** `nexus-server/src/tools_registry.rs`, `nexus-server/src/agent_loop.rs` (remove the match arm)

---

## M8. Duplicate JWT in nexus-gateway

**Verdict:** DEFER. Moving shared auth types to `nexus-common` is correct but is a separate refactor. Gateway is a standalone binary and the duplication is small.

---

## M9. Typing Timeout Task Not Cancelled on Disconnect

**Verdict:** Already mitigated — typing tokens are cancelled on `send()`. The 120s fallback is acceptable as a safety net. No change needed.

---

## Summary: What To Do Now

| # | Issue | Action | Files |
|---|-------|--------|-------|
| C2 | Session creation duplication | Extract `ensure_session_and_publish` | session.rs, gateway.rs, discord/gateway_conn.rs |
| I1 | Unused once_cell | Remove from Cargo.toml | Cargo.toml |
| I2+I3+M5 | Unused imports/vars in context.rs | Clean up imports, use `_` or `.values()` | context.rs |
| I4 | Dead functions in db.rs | Delete `update_device_name`, `create_session` | db.rs |
| I5 | Dead method in session.rs | Delete `list_sessions` | session.rs |
| I6 | Session memory leak | Call `unregister_session` + `remove_session` on loop exit | agent_loop.rs |
| I7 | Confusing mut state | Simplify main.rs initialization | main.rs |
| I8 | Config fields bypassed | GatewayChannel reads from config struct | gateway.rs |
| I10 | Commented-out modules | Remove `// mod api`, `// mod memory`, delete stub files | main.rs |
| I11 | Raw SQL in agent_loop | Add `db::ensure_session`, replace raw SQL | db.rs, agent_loop.rs |
| I12 | Empty tools on follow-up | Pass tools to follow-up LLM calls | agent_loop.rs |
| M1 | Stale device_id comments | Update comments | state.rs, tools_registry.rs |
| M2 | ConnHandle.handle unused | Prefix with `_` | discord/mod.rs |
| M4 | Dead RAG stubs | Delete `embed_text`, `RAG_TOP_K`, commented-out RAG code | context.rs |
| M7 | DeviceOffline never used | Delete variant + match arm | tools_registry.rs, agent_loop.rs |

**Deferred:** C1 (API key → admin API), I9 (gateway), M3 (keep fields), M6 (error types), M8 (JWT duplication), M9 (typing timeout)
