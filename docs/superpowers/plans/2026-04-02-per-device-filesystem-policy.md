# Per-Device Filesystem Access Policy + Skills Registration Fix

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** (1) Allow users to configure per-device filesystem access policies (sandbox, whitelist, unrestricted) through the REST API, with hot-reload via heartbeat. (2) Fix the server discarding skills from `RegisterTools` — store them in `DeviceState` and inject `always=true` skills into the system prompt.

**Architecture:** Add a `fs_policy` JSONB column to `device_tokens` table. Server sends the policy to the client during handshake (via `LoginSuccess`) and on every heartbeat response (via new `HeartbeatAck`). Client stores the active policy in an `Arc<RwLock<FsPolicy>>` and the `sanitize_path` function checks it on every filesystem operation. Whitelisted paths are read-only; writes are always restricted to the workspace. Additionally, `DeviceState` gains a `skills` field so the server can inject `always=true` skill content into the agent's system prompt.

**Tech Stack:** Rust (serde, sqlx, tokio, axum), PostgreSQL (JSONB)

---

## File Structure

| Action | File | Responsibility |
|--------|------|---------------|
| Modify | `nexus-common/src/protocol.rs` | Add `FsPolicy` struct, extend `LoginSuccess` with policy field, add `HeartbeatAck` variant to `ServerToClient` |
| Modify | `nexus-server/src/state.rs` | Add `skills: Vec<SkillFull>` field to `DeviceState` |
| Modify | `nexus-server/src/db.rs` | Add `fs_policy` column to `device_tokens`, add unique constraint on `(user_id, device_name)`, add `get_device_policy` / `update_device_policy` queries |
| Modify | `nexus-server/src/ws.rs` | Include policy in `LoginSuccess`, send `HeartbeatAck` with policy on heartbeat, store skills from `RegisterTools` |
| Modify | `nexus-server/src/context.rs` | Inject `always=true` skill content into system prompt |
| Modify | `nexus-server/src/auth.rs` | Add `get_device_policy` / `update_device_policy` REST handlers |
| Modify | `nexus-server/src/main.rs` | Register new policy API route |
| Modify | `nexus-client/src/session.rs` | Parse policy from `LoginSuccess`, update policy on `HeartbeatAck`, store in shared state |
| Modify | `nexus-client/src/env.rs` | Replace hardcoded `restrict` bool with `FsPolicy`-aware path validation (workspace=read+write, whitelist=read-only, unrestricted=all access) |
| Modify | `nexus-client/src/tools/fs.rs` | Pass operation type (read vs write) to path resolution so whitelist can be enforced as read-only |
| Modify | `nexus-client/src/main.rs` | Thread `Arc<RwLock<FsPolicy>>` through to tools |

---

### Task 1: Add `FsPolicy` to Protocol

**Files:**
- Modify: `nexus-common/src/protocol.rs`

- [ ] **Step 1: Add the `FsPolicy` struct and `HeartbeatAck` variant**

In `nexus-common/src/protocol.rs`, add:

```rust
/// Per-device filesystem access policy.
/// - Sandbox: only workspace (default)
/// - Whitelist: workspace (read+write) + listed paths (read-only)
/// - Unrestricted: full filesystem access
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode")]
pub enum FsPolicy {
    #[serde(rename = "sandbox")]
    Sandbox,
    #[serde(rename = "whitelist")]
    Whitelist { allowed_paths: Vec<String> },
    #[serde(rename = "unrestricted")]
    Unrestricted,
}

impl Default for FsPolicy {
    fn default() -> Self {
        FsPolicy::Sandbox
    }
}
```

Extend `LoginSuccess` to include the policy:

```rust
LoginSuccess {
    user_id: String,
    device_name: String,
    fs_policy: FsPolicy,
},
```

Add `HeartbeatAck` to `ServerToClient`:

```rust
HeartbeatAck {
    fs_policy: FsPolicy,
},
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build --package nexus-common 2>&1 | grep "^error"`
Expected: No errors (but downstream crates will break — that's expected, we fix them in later tasks)

- [ ] **Step 3: Commit**

```bash
git add nexus-common/src/protocol.rs
git commit -m "feat(protocol): add FsPolicy enum, extend LoginSuccess, add HeartbeatAck"
```

---

### Task 2: Database Schema and Queries

**Files:**
- Modify: `nexus-server/src/db.rs`

- [ ] **Step 1: Add `fs_policy` column and unique constraint to `device_tokens`**

In the `init_db` function, after the existing `device_tokens` CREATE TABLE, add two ALTER statements:

```rust
sqlx::query("ALTER TABLE device_tokens ADD COLUMN IF NOT EXISTS fs_policy JSONB NOT NULL DEFAULT '{\"mode\":\"sandbox\"}'")
    .execute(pool)
    .await?;

sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS idx_device_tokens_user_device ON device_tokens (user_id, device_name)")
    .execute(pool)
    .await?;
```

- [ ] **Step 2: Add `get_device_policy` query**

```rust
pub async fn get_device_policy(
    db: &PgPool,
    user_id: &str,
    device_name: &str,
) -> Result<nexus_common::protocol::FsPolicy, sqlx::Error> {
    let row: (serde_json::Value,) = sqlx::query_as(
        "SELECT COALESCE(fs_policy, '{\"mode\":\"sandbox\"}'::jsonb) FROM device_tokens WHERE user_id = $1 AND device_name = $2 AND revoked = FALSE"
    )
    .bind(user_id)
    .bind(device_name)
    .fetch_one(db)
    .await?;

    serde_json::from_value(row.0)
        .map_err(|e| sqlx::Error::Protocol(format!("invalid fs_policy JSON: {e}")))
}
```

- [ ] **Step 3: Add `update_device_policy` query**

```rust
pub async fn update_device_policy(
    db: &PgPool,
    user_id: &str,
    device_name: &str,
    policy: &nexus_common::protocol::FsPolicy,
) -> Result<bool, sqlx::Error> {
    let json = serde_json::to_value(policy)
        .map_err(|e| sqlx::Error::Protocol(format!("failed to serialize policy: {e}")))?;

    let result = sqlx::query(
        "UPDATE device_tokens SET fs_policy = $1 WHERE user_id = $2 AND device_name = $3 AND revoked = FALSE"
    )
    .bind(json)
    .bind(user_id)
    .bind(device_name)
    .execute(db)
    .await?;

    Ok(result.rows_affected() > 0)
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build --package nexus-server 2>&1 | grep "^error"`
Expected: Errors in `ws.rs` (LoginSuccess now requires `fs_policy`) — expected, fixed in Task 3.

- [ ] **Step 5: Commit**

```bash
git add nexus-server/src/db.rs
git commit -m "feat(db): add fs_policy column, unique device constraint, policy queries"
```

---

### Task 3: Server WebSocket — Send Policy in Handshake and Heartbeat

**Files:**
- Modify: `nexus-server/src/ws.rs`

- [ ] **Step 1: Include policy in `LoginSuccess`**

After `verify_device_token` succeeds (around line 80-92), fetch the policy:

```rust
let fs_policy = db::get_device_policy(&state.db, &user_id, &device_name)
    .await
    .unwrap_or_default();
```

Update the `LoginSuccess` construction to include it:

```rust
let login_success = ServerToClient::LoginSuccess {
    user_id: user_id.clone(),
    device_name: device_name.clone(),
    fs_policy,
};
```

- [ ] **Step 2: Send `HeartbeatAck` on heartbeat**

In the heartbeat match arm (around line 170), after updating `last_seen`, fetch the current policy and send it back:

```rust
ClientToServer::Heartbeat { hash: _, status: _ } => {
    let mut devices = state.devices.write().await;
    if let Some(device) = devices.get_mut(&device_key) {
        device.last_seen = Instant::now();
    }
    drop(devices);

    let fs_policy = db::get_device_policy(&state.db, &user_id, &device_name)
        .await
        .unwrap_or_default();

    let ack = ServerToClient::HeartbeatAck { fs_policy };
    let ack_text = serde_json::to_string(&ack).unwrap_or_default();
    let _ = ws_tx.send(Message::Text(ack_text.into())).await;
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build --package nexus-server 2>&1 | grep "^error"`
Expected: No server errors. Client will have errors (LoginSuccess changed) — fixed in Task 5.

- [ ] **Step 4: Commit**

```bash
git add nexus-server/src/ws.rs
git commit -m "feat(ws): send FsPolicy in LoginSuccess and HeartbeatAck"
```

---

### Task 4: REST API for Device Policy

**Files:**
- Modify: `nexus-server/src/auth.rs`
- Modify: `nexus-server/src/main.rs`

- [ ] **Step 1: Add request/response structs and handlers in `auth.rs`**

```rust
#[derive(Debug, Deserialize)]
pub struct UpdateDevicePolicyRequest {
    pub fs_policy: nexus_common::protocol::FsPolicy,
}

#[derive(Debug, Serialize)]
pub struct DevicePolicyResponse {
    pub device_name: String,
    pub fs_policy: nexus_common::protocol::FsPolicy,
}

pub async fn get_device_policy(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(device_name): Path<String>,
) -> Response {
    match db::get_device_policy(&state.db, &claims.sub, &device_name).await {
        Ok(policy) => Json(DevicePolicyResponse {
            device_name,
            fs_policy: policy,
        }).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "Device not found").into_response(),
    }
}

pub async fn update_device_policy(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(device_name): Path<String>,
    Json(payload): Json<UpdateDevicePolicyRequest>,
) -> Response {
    match db::update_device_policy(&state.db, &claims.sub, &device_name, &payload.fs_policy).await {
        Ok(true) => Json(DevicePolicyResponse {
            device_name,
            fs_policy: payload.fs_policy,
        }).into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "Device not found or revoked").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {e}")).into_response(),
    }
}
```

- [ ] **Step 2: Register route in `main.rs`**

Add inside the `protected` router:

```rust
.route("/api/devices/{device_name}/policy", axum::routing::get(auth::get_device_policy).patch(auth::update_device_policy))
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build --package nexus-server 2>&1 | grep "^error"`
Expected: No server errors.

- [ ] **Step 4: Commit**

```bash
git add nexus-server/src/auth.rs nexus-server/src/main.rs
git commit -m "feat(api): add GET/PATCH /api/devices/{device_name}/policy endpoint"
```

---

### Task 5: Client — Receive and Store Policy

**Files:**
- Modify: `nexus-client/src/session.rs`
- Modify: `nexus-client/src/main.rs`

- [ ] **Step 1: Update handshake to extract `FsPolicy` from `LoginSuccess`**

In `perform_handshake` (session.rs), change the return type to include the policy:

```rust
async fn perform_handshake(
    ws_stream: &mut ...,
    auth_token: &str,
) -> Result<(String, FsPolicy), String> {
```

Update the `LoginSuccess` match:

```rust
ServerToClient::LoginSuccess { device_name, fs_policy, .. } => {
    info!("device login success: device_name={}, fs_policy={:?}", device_name, fs_policy);
    Ok((device_name, fs_policy))
}
```

Update the caller of `perform_handshake` to store the policy in an `Arc<RwLock<FsPolicy>>`:

```rust
let (device_name, initial_policy) = perform_handshake(ws_stream, &config.auth_token).await?;
*policy_lock.write().await = initial_policy;
```

- [ ] **Step 2: Handle `HeartbeatAck` in the message loop**

In the `session.rs` message loop, add a match arm for `HeartbeatAck`:

```rust
ServerToClient::HeartbeatAck { fs_policy } => {
    let current = policy_lock.read().await;
    if *current != fs_policy {
        drop(current);
        info!("FsPolicy updated via heartbeat: {:?}", fs_policy);
        *policy_lock.write().await = fs_policy;
    }
}
```

- [ ] **Step 3: Create the shared policy lock in `main.rs`**

In `nexus-client/src/main.rs`, create the shared policy and pass it through:

```rust
use nexus_common::protocol::FsPolicy;
use std::sync::Arc;
use tokio::sync::RwLock;

let fs_policy = Arc::new(RwLock::new(FsPolicy::default()));
```

Pass `fs_policy.clone()` to `run_session` (or wherever the session is started) and to the tool executor so filesystem tools can read it.

- [ ] **Step 4: Verify it compiles**

Run: `cargo build --package nexus-client 2>&1 | grep "^error"`
Expected: Errors in `env.rs` / `fs.rs` — fixed in Task 6.

- [ ] **Step 5: Commit**

```bash
git add nexus-client/src/session.rs nexus-client/src/main.rs
git commit -m "feat(client): receive FsPolicy from handshake and heartbeat, store in Arc<RwLock>"
```

---

### Task 6: Client — Policy-Aware Path Validation

**Files:**
- Modify: `nexus-client/src/env.rs`
- Modify: `nexus-client/src/tools/fs.rs`

- [ ] **Step 1: Replace `sanitize_path` with policy-aware version in `env.rs`**

Add a new function alongside the existing one:

```rust
use nexus_common::protocol::FsPolicy;

/// Operation type for policy enforcement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FsOp {
    Read,
    Write,
}

pub fn sanitize_path_with_policy(
    path: &str,
    op: FsOp,
    policy: &FsPolicy,
) -> Result<PathBuf, String> {
    let p = Path::new(path);

    let resolved = if p.is_relative() {
        get_workspace_root().join(p)
    } else {
        PathBuf::from(p)
    };

    let resolved = resolved.canonicalize().unwrap_or_else(|_| resolved.clone());

    match policy {
        FsPolicy::Unrestricted => Ok(resolved),
        FsPolicy::Sandbox => {
            let workspace = get_workspace_root();
            let workspace = workspace.canonicalize().unwrap_or(workspace);
            if !is_subpath(&resolved, &workspace) {
                return Err(format!(
                    "Path {} is outside workspace {}",
                    resolved.display(),
                    workspace.display()
                ));
            }
            Ok(resolved)
        }
        FsPolicy::Whitelist { allowed_paths } => {
            let workspace = get_workspace_root();
            let workspace = workspace.canonicalize().unwrap_or(workspace);

            // Workspace: always allowed (read+write)
            if is_subpath(&resolved, &workspace) {
                return Ok(resolved);
            }

            // Whitelisted paths: read-only
            if op == FsOp::Write {
                return Err(format!(
                    "Path {} is outside workspace — writes only allowed in workspace",
                    resolved.display()
                ));
            }

            for allowed in allowed_paths {
                let allowed_path = PathBuf::from(allowed);
                let allowed_path = allowed_path.canonicalize().unwrap_or(allowed_path);
                if is_subpath(&resolved, &allowed_path) {
                    return Ok(resolved);
                }
            }

            Err(format!(
                "Path {} is outside workspace and not in whitelist",
                resolved.display()
            ))
        }
    }
}

pub async fn sanitize_path_with_policy_async(
    raw: &str,
    op: FsOp,
    policy: &FsPolicy,
) -> Result<PathBuf, String> {
    let raw = raw.to_string();
    let policy = policy.clone();
    tokio::task::spawn_blocking(move || sanitize_path_with_policy(&raw, op, &policy))
        .await
        .unwrap_or_else(|_| Err("path resolution task panicked".to_string()))
}
```

- [ ] **Step 2: Update `fs.rs` to use policy-aware path resolution**

Change the `resolve_path_async` helper to accept `FsOp` and a policy reference:

```rust
use nexus_common::protocol::FsPolicy;
use crate::env::FsOp;
use std::sync::Arc;
use tokio::sync::RwLock;

async fn resolve_path_for_read(path: &str, policy: &FsPolicy) -> Result<PathBuf, ToolError> {
    env::sanitize_path_with_policy_async(path, FsOp::Read, policy)
        .await
        .map_err(|e| ToolError::InvalidParams(format!("path access denied: {}", e)))
}

async fn resolve_path_for_write(path: &str, policy: &FsPolicy) -> Result<PathBuf, ToolError> {
    env::sanitize_path_with_policy_async(path, FsOp::Write, policy)
        .await
        .map_err(|e| ToolError::InvalidParams(format!("path access denied: {}", e)))
}
```

Update each tool function to accept `&FsPolicy` and use the appropriate resolver:
- `read_file` → `resolve_path_for_read`
- `list_dir` → `resolve_path_for_read`
- `stat_path` → `resolve_path_for_read`
- `write_file` → `resolve_path_for_write`
- `edit_file` → `resolve_path_for_write`
- `mkdir` → `resolve_path_for_write`
- `move_file` → source: `resolve_path_for_read`, destination: `resolve_path_for_write`

The `&FsPolicy` should be obtained by reading from the `Arc<RwLock<FsPolicy>>` at the call site (in `executor.rs` or wherever tools are dispatched), then passed as a reference into each tool function.

- [ ] **Step 3: Verify it compiles**

Run: `cargo build --package nexus-client 2>&1 | grep "^error"`
Expected: No errors.

- [ ] **Step 4: Commit**

```bash
git add nexus-client/src/env.rs nexus-client/src/tools/fs.rs
git commit -m "feat(client): policy-aware path validation with read-only whitelist enforcement"
```

---

### Task 7: Fix Skills Registration on Server

**Files:**
- Modify: `nexus-server/src/state.rs`
- Modify: `nexus-server/src/ws.rs`

- [ ] **Step 1: Add `skills` field to `DeviceState`**

In `nexus-server/src/state.rs`, add the import and field:

```rust
use nexus_common::protocol::SkillFull;
```

Add to `DeviceState`:

```rust
pub struct DeviceState {
    pub user_id: String,
    pub device_name: String,
    pub ws_tx: mpsc::Sender<Message>,
    pub tools: Vec<serde_json::Value>,
    pub skills: Vec<SkillFull>,
    pub last_seen: Instant,
}
```

- [ ] **Step 2: Initialize `skills` in device registration and update on `RegisterTools`**

In `nexus-server/src/ws.rs`, where `DeviceState` is constructed (around line 100-115), add:

```rust
skills: Vec::new(),
```

In the `RegisterTools` match arm, store skills instead of discarding them:

```rust
ClientToServer::RegisterTools { schemas, skills } => {
    let mut devices = state.devices.write().await;
    if let Some(device) = devices.get_mut(&device_key) {
        device.tools = schemas;
        device.skills = skills;
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build --package nexus-server 2>&1 | grep "^error"`
Expected: No errors.

- [ ] **Step 4: Commit**

```bash
git add nexus-server/src/state.rs nexus-server/src/ws.rs
git commit -m "fix(ws): store skills from RegisterTools in DeviceState instead of discarding"
```

---

### Task 8: Inject Always-On Skills into System Prompt

**Files:**
- Modify: `nexus-server/src/context.rs`

- [ ] **Step 1: Add section 5 — always-on skills injection**

In `build_system_prompt`, after section 4 (RAG injection) and before `sections.join(SECTION_SEPARATOR)`, add:

```rust
    // 段 5 — 常驻 Skill 内容注入（always=true 的 skill 全文注入）
    let always_skills = collect_always_skills(state, user_id).await;
    if !always_skills.is_empty() {
        let mut skill_section = String::from("## Active Skills\n");
        for (name, content) in &always_skills {
            skill_section.push_str(&format!("### {}\n{}\n\n", name, content));
        }
        sections.push(skill_section);
    }
```

- [ ] **Step 2: Add the `collect_always_skills` helper**

```rust
/// Collect all always=true skills from the user's online devices.
async fn collect_always_skills(state: &AppState, user_id: &str) -> Vec<(String, String)> {
    let devices = state.devices.read().await;
    let mut skills = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for device_state in devices.values() {
        if device_state.user_id != user_id {
            continue;
        }
        for skill in &device_state.skills {
            if skill.always {
                if let Some(ref content) = skill.content {
                    if seen.insert(skill.name.clone()) {
                        skills.push((skill.name.clone(), content.clone()));
                    }
                }
            }
        }
    }

    skills
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build --package nexus-server 2>&1 | grep "^error"`
Expected: No errors.

- [ ] **Step 4: Commit**

```bash
git add nexus-server/src/context.rs
git commit -m "feat(context): inject always-on skill content into system prompt (section 5)"
```

---

### Task 9: Integration Smoke Test

- [ ] **Step 1: Start postgres and server**

```bash
# Terminal 1
cd NEXUS/nexus-server && cargo run
```

- [ ] **Step 2: Register user and configure LLM**

```bash
curl -X POST http://localhost:8080/api/auth/register \
  -H "Content-Type: application/json" \
  -d '{"email":"test@test.com","password":"testpass","admin_token":"nexus_dev_admin_token"}'
# Save the JWT token
```

- [ ] **Step 3: Create device token and start client**

```bash
curl -X POST http://localhost:8080/api/device-tokens \
  -H "Authorization: Bearer <jwt>" \
  -H "Content-Type: application/json" \
  -d '{"device_name":"test-device"}'
# Save device token, start client with it
```

- [ ] **Step 4: Verify default sandbox policy**

```bash
curl http://localhost:8080/api/devices/test-device/policy \
  -H "Authorization: Bearer <jwt>"
```
Expected: `{"device_name":"test-device","fs_policy":{"mode":"sandbox"}}`

- [ ] **Step 5: Set whitelist policy and verify hot-reload**

```bash
curl -X PATCH http://localhost:8080/api/devices/test-device/policy \
  -H "Authorization: Bearer <jwt>" \
  -H "Content-Type: application/json" \
  -d '{"fs_policy":{"mode":"whitelist","allowed_paths":["/var/log"]}}'
```
Expected: Next heartbeat cycle, client logs `FsPolicy updated via heartbeat`.
Agent should be able to read files in `/var/log` but not write to it.

- [ ] **Step 6: Set unrestricted and verify**

```bash
curl -X PATCH http://localhost:8080/api/devices/test-device/policy \
  -H "Authorization: Bearer <jwt>" \
  -H "Content-Type: application/json" \
  -d '{"fs_policy":{"mode":"unrestricted"}}'
```
Expected: Agent can read and write anywhere via absolute paths.

- [ ] **Step 7: Commit (if any test-driven fixes were needed)**

```bash
git add -u
git commit -m "fix: address issues found during integration testing"
```
