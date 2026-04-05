# M3 Final Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix all 4 incomplete features and all code quality findings to reach a clean M3 close before M4.

**Architecture:** Fix-forward — no new features, only completing incomplete work and cleaning up.

**Tech Stack:** Rust, sqlx, serde_json, axum

---

## Task Overview

| # | Task | Type | Scope |
|---|------|------|-------|
| 1 | Centralized error handling | Quality | nexus-common, nexus-server |
| 2 | Cron REST API wiring | Incomplete | nexus-server |
| 3 | Checkpoint resume on restart | Incomplete | nexus-server |
| 4 | Tool schema merging across devices | Incomplete | nexus-server |
| 5 | MCP hot-reload hash fix | Incomplete | nexus-client |
| 6 | Dead code removal | Quality | all crates |
| 7 | DRY fixes (CronJob mapping, schema normalization) | Quality | nexus-server |
| 8 | Consistency fixes (error messages, logging) | Quality | nexus-server |

**Dependency order:** 1 → 8 → 2 → 3 → 4 → 5 → 6 → 7

Task 1 (error centralization) must come first since Task 8 depends on it.

---

## Task 1: Centralized Error Handling

**Files:**
- Modify: `nexus-common/src/error.rs` — expand with all error codes + API response type
- Modify: `nexus-common/src/lib.rs` — re-export error module
- Modify: `nexus-server/src/auth.rs` — use centralized errors in all API handlers
- Modify: `nexus-server/src/api.rs` — use centralized errors
- Modify: `nexus-server/src/agent_loop.rs` — use centralized errors for tool execution

### Design

```rust
// nexus-common/src/error.rs

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ErrorCode {
    // Auth
    AuthFailed,
    AuthTokenExpired,
    Unauthorized,
    Forbidden,

    // Resource
    NotFound,
    Conflict,

    // Validation
    ValidationFailed,
    InvalidParams,

    // Execution
    ExecutionFailed,
    ExecutionTimeout,
    ExecutionCancelled,
    DeviceNotFound,
    DeviceOffline,

    // Protocol
    ProtocolMismatch,

    // System
    InternalError,
    ServiceUnavailable,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str { /* match each variant */ }

    pub fn http_status(&self) -> u16 {
        match self {
            Self::AuthFailed | Self::AuthTokenExpired | Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NotFound | Self::DeviceNotFound => 404,
            Self::Conflict => 409,
            Self::ValidationFailed | Self::InvalidParams => 400,
            Self::ExecutionTimeout => 408,
            _ => 500,
        }
    }
}

/// Standard API error response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code: code.as_str().to_string(), message: message.into() }
    }
}
```

Usage in handlers:
```rust
// Before (inconsistent):
(StatusCode::FORBIDDEN, "admin only").into_response()
(StatusCode::FORBIDDEN, "Admin only").into_response()
(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {}", e)).into_response()

// After (consistent):
ApiError::new(ErrorCode::Forbidden, "admin access required").into_response()
ApiError::new(ErrorCode::InternalError, "operation failed").into_response()
```

- [ ] Step 1: Expand `nexus-common/src/error.rs` with all error codes, `http_status()`, and `ApiError`
- [ ] Step 2: Re-export in `nexus-common/src/lib.rs`
- [ ] Step 3: Add `impl IntoResponse for ApiError` in nexus-server (axum integration)
- [ ] Step 4: Update all auth.rs handlers to use `ApiError` (stop leaking raw DB errors)
- [ ] Step 5: Update api.rs handlers
- [ ] Step 6: Build and test
- [ ] Step 7: Commit

---

## Task 2: Cron REST API Wiring

**Files:**
- Modify: `nexus-server/src/main.rs` — verify cron routes are registered
- Modify: `nexus-server/src/auth.rs` — add cron REST handlers if missing

### Check
The plan already added cron handlers in auth.rs and routes in main.rs. Codex says they're missing — verify and fix if needed.

Expected routes:
```
GET  /api/cron-jobs         → list user's cron jobs
POST /api/cron-jobs         → create cron job
DELETE /api/cron-jobs/{id}  → delete cron job
PATCH /api/cron-jobs/{id}   → update cron job (enable/disable)
```

- [ ] Step 1: Check if routes exist in main.rs; add if missing
- [ ] Step 2: Check if handlers exist in auth.rs; add if missing
- [ ] Step 3: Add `update_cron_job` handler for PATCH (enable/disable, change schedule)
- [ ] Step 4: Add `db::update_cron_job()` function
- [ ] Step 5: Build and test
- [ ] Step 6: Commit

---

## Task 3: Checkpoint Resume on Restart

**Files:**
- Modify: `nexus-server/src/db.rs` — add `list_all_checkpoints()`
- Modify: `nexus-server/src/main.rs` — resume checkpoints on startup
- Modify: `nexus-server/src/db.rs` — remove unused `pending_tool_calls` column

### Design
On server startup, after initializing AppState and bus:
1. Query `agent_checkpoints` for any orphaned checkpoints
2. For each, inject a resume message into the bus
3. The agent loop picks it up and continues

```rust
// In main.rs after bus/session init:
if let Ok(checkpoints) = db::list_all_checkpoints(&pool).await {
    for cp in checkpoints {
        info!("Resuming agent checkpoint for session {}", cp.session_id);
        let event = InboundEvent {
            channel: cp.channel,
            sender_id: cp.user_id,
            chat_id: cp.chat_id,
            content: "[System] Resuming interrupted task...".into(),
            session_id: cp.session_id,
            ..
        };
        state.bus.publish_inbound(event).await;
    }
}
```

- [ ] Step 1: Add `list_all_checkpoints()` to db.rs
- [ ] Step 2: Add `Checkpoint` struct
- [ ] Step 3: Add resume logic in main.rs
- [ ] Step 4: Remove `pending_tool_calls` column from schema (unused)
- [ ] Step 5: Build and test
- [ ] Step 6: Commit

---

## Task 4: Tool Schema Merging Across Devices

**Files:**
- Modify: `nexus-server/src/context.rs` — merge same-named device tools
- Modify: `nexus-server/src/tools_registry.rs` — add merge helper

### Design
Current: each device's tools get `device_name` injected separately → duplicates.
Target: group tools by function name, merge device names into one enum.

```rust
fn merge_device_tool_schemas(
    devices: &HashMap<String, DeviceState>,
    user_id: &str,
) -> Vec<Value> {
    // tool_function_name → (base_schema, vec![device_names])
    let mut tool_map: IndexMap<String, (Value, Vec<String>)> = IndexMap::new();

    for device in devices.values().filter(|d| d.user_id == user_id) {
        for schema in &device.tools {
            let name = schema.pointer("/function/name")
                .and_then(|v| v.as_str()).unwrap_or("").to_string();
            tool_map.entry(name)
                .and_modify(|(_, devices)| devices.push(device.device_name.clone()))
                .or_insert((schema.clone(), vec![device.device_name.clone()]));
        }
    }

    tool_map.into_values()
        .map(|(schema, devices)| inject_device_name_param(schema, &devices))
        .collect()
}
```

- [ ] Step 1: Add `merge_device_tool_schemas()` to tools_registry.rs
- [ ] Step 2: Update `get_all_tools_schema()` in context.rs to use it
- [ ] Step 3: Build and test
- [ ] Step 4: Commit

---

## Task 5: MCP Hot-Reload Hash Fix

**Files:**
- Modify: `nexus-client/src/discovery.rs` — hash full config, not just names

### Fix
Change the hash from just server names to full config (name + command + args + env keys):

```rust
// Before:
let current_hash = compute_hash(&mcp_servers.iter().map(|s| &s.name).collect::<Vec<_>>());

// After:
let current_hash = compute_hash(&mcp_servers.iter().map(|s| {
    (&s.name, &s.command, &s.args, s.env.as_ref().map(|e| {
        let mut keys: Vec<_> = e.keys().collect();
        keys.sort();
        keys
    }))
}).collect::<Vec<_>>());
```

- [ ] Step 1: Update hash computation in `discover_mcp_tools_internal()`
- [ ] Step 2: Update hash computation in `init_mcp()`
- [ ] Step 3: Build and test
- [ ] Step 4: Commit

---

## Task 6: Dead Code Removal

**Files:**
- Delete: `nexus-client/src/process.rs`
- Delete: `nexus-common/src/error.rs` (old version — replaced by Task 1's new version)
- Modify: `nexus-gateway/src/protocol.rs` — remove `BrowserOutbound::Connected`
- Modify: `nexus-server/src/bus.rs` — remove `shutdown()`
- Modify: `nexus-server/src/server_tools/mod.rs` — remove `has()`
- Modify: `nexus-server/src/server_mcp.rs` — remove unused `get_session_mut()`, `server_names()`
- Modify: `nexus-client/src/discovery.rs` — remove `#[allow(dead_code)]` on `discover_mcp_tools`
- Modify: `nexus-client/src/executor.rs` — remove `#[allow(dead_code)]` on `validate_required_params`
- Modify: `nexus-server/src/channels/discord/protocol.rs` — remove unused `message_id` field

- [ ] Step 1: Delete `nexus-client/src/process.rs`
- [ ] Step 2: Remove all listed dead functions/fields
- [ ] Step 3: Build and fix any cascading issues
- [ ] Step 4: Commit

---

## Task 7: DRY Fixes

**Files:**
- Modify: `nexus-server/src/db.rs` — deduplicate CronJob tuple mapping
- Modify: `nexus-server/src/server_mcp.rs` — add schema normalization (match client)

### CronJob mapping
Extract shared helper:
```rust
fn row_to_cron_job(r: (String, String, ...)) -> CronJob { ... }
```
Use in both `list_cron_jobs` and `get_due_cron_jobs`.

### Server MCP schema normalization
Copy `normalize_schema_for_openai()` from nexus-client into a shared location (nexus-common), or duplicate in server_mcp.rs. The server MCP must normalize schemas the same way client MCP does.

- [ ] Step 1: Extract CronJob row mapping helper
- [ ] Step 2: Add schema normalization to server_mcp.rs
- [ ] Step 3: Build and test
- [ ] Step 4: Commit

---

## Task 8: Consistency Fixes

**Files:**
- Modify: `nexus-server/src/auth.rs` — standardize forbidden messages
- Modify: `nexus-server/src/api.rs` — standardize forbidden messages
- Modify: `nexus-server/src/agent_loop.rs` — reduce info-level logging of sensitive data
- Modify: `nexus-server/src/ws.rs` — fix "warn:" inside warn! log

### Changes
- All forbidden responses: use `ApiError::new(ErrorCode::Forbidden, "admin access required")`
- agent_loop.rs: change `info!("tool_name={}, arguments={}")` to `debug!`
- ws.rs: fix `warn!("warn: ...")` → `warn!("...")`

- [ ] Step 1: Apply all consistency fixes (covered by Task 1's ApiError migration)
- [ ] Step 2: Reduce log verbosity for sensitive data
- [ ] Step 3: Build and test
- [ ] Step 4: Commit

---

## Execution Order

```
Task 1 (error centralization)     ← foundation
Task 8 (consistency fixes)        ← uses Task 1's ApiError
Task 2 (cron REST API)            ← uses Task 1's ApiError
Task 3 (checkpoint resume)        ← independent
Task 4 (tool schema merging)      ← independent
Task 5 (MCP hash fix)             ← independent
Task 6 (dead code removal)        ← do after all features complete
Task 7 (DRY fixes)                ← do last
```

Tasks 3, 4, 5 can run in parallel after Task 1+8 are done.
