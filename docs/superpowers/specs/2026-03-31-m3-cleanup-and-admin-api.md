# M3 Final Cleanup + Admin API Spec

**Date:** 2026-03-31
**Goal:** 收一个干净的 M3。修复所有代码质量问题 + 实现 Admin API，使 M4 可以专注 Gateway。

---

## 优先级分层

| 级别 | 含义 |
|------|------|
| **P0-CRITICAL** | 功能性 bug，不修则系统不可用 |
| **P1-HIGH** | 内存泄漏、架构违规、DRY 违反 |
| **P2-MEDIUM** | 死代码、命名不一致、未使用依赖 |
| **P3-LOW** | 注释、下划线前缀等小修 |

---

## P0-CRITICAL

### FIX-01: Empty Tools on Follow-Up LLM Calls

**问题：** `execute_tool_calls_loop` 中，所有后续 LLM 调用都传 `tools: vec![]`。LLM 在第二轮及之后无法再调用工具，多步推理完全失效。

**位置：** `agent_loop.rs:230-234`（soft error 后）和 `agent_loop.rs:270-274`（正常 follow-up）

**修复：**
1. `run_single_turn` 将 `tools` 传入 `execute_tool_calls_loop`
2. `execute_tool_calls_loop` 签名增加 `tools: Vec<Value>` 参数
3. 所有后续 `ChatCompletionRequest` 使用 `tools.clone()` 而非 `vec![]`
4. soft error 分支：仍然传完整 tools（让 LLM 可以选择不同工具）

```rust
// run_single_turn 调用处改为：
execute_tool_calls_loop(state, user_id, session_id, messages, tool_calls, tools).await

// execute_tool_calls_loop 签名改为：
async fn execute_tool_calls_loop(
    state: &Arc<AppState>,
    user_id: &str,
    session_id: &str,
    messages: Vec<Value>,
    initial_tool_calls: Vec<ToolCallParsed>,
    tools: Vec<Value>,  // 新增
) -> Result<String, String>

// 两处 ChatCompletionRequest 都改为：
tools: tools.clone(),
```

**Files:** `nexus-server/src/agent_loop.rs`

---

### FIX-02: Raw SQL in agent_loop.rs (违反 db.rs 单一职责)

**问题：** `agent_loop.rs:40-48` 直接用 `sqlx::query()` 创建 session，绕过 `db.rs`。且 SQL 语法不完整：`ON CONFLICT DO NOTHING` 缺少 `(session_id)` 列标识。

**修复：**
1. 在 `db.rs` 新增 `ensure_session` 函数：

```rust
pub async fn ensure_session(
    db: &PgPool,
    session_id: &str,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sessions (session_id, user_id) VALUES ($1, $2) ON CONFLICT (session_id) DO NOTHING"
    )
    .bind(session_id)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(())
}
```

2. `agent_loop.rs` 中替换为 `db::ensure_session(&state.db, &session_id, &event.sender_id).await`
3. 删除 `db.rs` 中旧的 `create_session` 函数（被 `ensure_session` 取代）

**Files:** `nexus-server/src/db.rs`, `nexus-server/src/agent_loop.rs`

---

## P1-HIGH

### FIX-03: Session Creation Logic Duplicated (Channel 不应知道如何创建 Session)

**问题：** `gateway.rs:handle_inbound` 和 `discord/gateway_conn.rs:handle_message` 都写了完全相同的 session 创建逻辑：
```
get_or_create_session → register_session → spawn(agent_loop) → publish_inbound
```
违反 DRY，且 channel 承担了不属于它的职责。

**修复：** 将 session 创建逻辑下沉到一个公共函数（放在 `session.rs` 或 `bus.rs` 均可，推荐放 `session.rs`）：

```rust
// session.rs 新增：
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
                crate::agent_loop::run_session(sid, inbox_rx, state_clone).await;
            });
        }
    }
    state.bus.publish_inbound(event).await;
}
```

然后 `gateway.rs:handle_inbound` 和 `discord/gateway_conn.rs:handle_message` 各自简化为构造 `InboundEvent` + 调用 `ensure_session_and_publish`。

**Files:** `nexus-server/src/session.rs`, `nexus-server/src/channels/gateway.rs`, `nexus-server/src/channels/discord/gateway_conn.rs`

---

### FIX-04: Session Memory Leak (agent_loop 结束不清理)

**问题：** 当 `agent_loop::run_session` 的 inbox channel 关闭（`while let Some(event) = inbox.recv().await` 退出），session 在 `bus.inbound_routes` 和 `session_manager.sessions` 中永远不会被移除。随着 session 增多，内存持续增长。

**修复：** 在 `agent_loop::run_session` 的 `while let` 循环退出后添加清理：

```rust
// agent_loop.rs run_session 末尾（while let 循环之后）
state.bus.unregister_session(&session_id);
state.session_manager.remove_session(&session_id).await;
info!("agent_session cleaned up: session_id={}", session_id);
```

**Files:** `nexus-server/src/agent_loop.rs`

---

### FIX-05: Config SSOT — GatewayChannel 直接读 env var

**问题：** `GatewayChannel::new()` 在 `gateway.rs:62-64` 直接调用 `std::env::var("NEXUS_GATEWAY_WS_URL")` 和 `std::env::var("NEXUS_GATEWAY_TOKEN")`，绕过了 `config.rs` 中已经读取的配置。违反 SSOT 原则。

**修复：** `GatewayChannel::new()` 从 `state.config` 读取：

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

### FIX-06: auth.rs 中 create_device_token 绕过 db.rs

**问题：** `auth.rs:197-204` 直接写 `sqlx::query("INSERT INTO device_tokens...")`，绕过 `db.rs`。违反与 FIX-02 相同的原则。

**修复：**
1. 在 `db.rs` 新增：

```rust
pub async fn create_device_token(
    db: &PgPool,
    token: &str,
    user_id: &str,
    device_name: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO device_tokens (token, user_id, device_name) VALUES ($1, $2, $3)"
    )
    .bind(token)
    .bind(user_id)
    .bind(device_name)
    .execute(db)
    .await?;
    Ok(())
}
```

2. `auth.rs` 中调用 `db::create_device_token(&state.db, &token, user_id, &payload.device_name).await`

**Files:** `nexus-server/src/db.rs`, `nexus-server/src/auth.rs`

---

### FIX-07: Confusing `mut state` + `Arc::clone` in main.rs

**问题：** `main.rs:57-58`：`state` 声明为 `mut`，然后 `state_arc = Arc::new(state.clone())`，随后 `state.channel_manager_handle.write()` 修改的是 `state`（不是 `state_arc`）。虽然因为 interior mutability（`Arc<RwLock<...>>`）凑巧能工作，但极其混乱。

**修复：** 消除 `mut`，统一使用 `state_arc`：

```rust
let state = state::AppState::new(pool, config.clone(), bus.clone(), session_manager);
let state_arc = Arc::new(state);

let mut channel_manager = ChannelManager::new(state_arc.bus.clone());
channel_manager.register(GatewayChannel::new(state_arc.clone()));
channel_manager.register(DiscordChannel::new(state_arc.clone()));
let channel_manager_handle = channel_manager.start();

*state_arc.channel_manager_handle.write().await = Some(channel_manager_handle);

// AppState is Clone (all fields are Arc), so deref + clone for axum state
let app = Router::new()
    // ...
    .with_state((*state_arc).clone());
```

**Files:** `nexus-server/src/main.rs`

---

### FIX-08: bus.rs publish_inbound 静默丢消息

**问题：** `bus.rs:70-75` 当 session_id 不在路由表中时，消息被静默丢弃，无日志。调试时完全不可见。

**修复：** 添加 warn 日志：

```rust
pub async fn publish_inbound(&self, event: InboundEvent) {
    let session_id = event.session_id.clone();
    if let Some(tx) = self.inbound_routes.get(&session_id) {
        if tx.send(event).await.is_err() {
            warn!("bus: session {} inbox closed, removing from routes", session_id);
            self.inbound_routes.remove(&session_id);
        }
    } else {
        warn!("bus: no route for session_id={}, message dropped", session_id);
    }
}
```

**Files:** `nexus-server/src/bus.rs`

---

## P2-MEDIUM

### FIX-09: Remove unused `once_cell` dependency

`Cargo.toml` 有 `once_cell = "1"` 但代码使用 `std::sync::LazyLock`。

**Files:** `nexus-server/Cargo.toml`

---

### FIX-10: Clean up context.rs — 删除死代码

1. 删除 `use std::collections::HashMap;`（未使用）
2. 删除 `use sqlx::PgPool;`（未使用）
3. 删除 `const RAG_TOP_K: usize = 5;`（stub，未实现）
4. 删除 `pub async fn embed_text(...)` 函数（stub，未实现）
5. 删除 RAG 注入的注释代码块（段 4 的所有注释）
6. 循环变量：`for (device_id, device_state)` → `for (_device_id, device_state)` 或改为 `for device_state in devices.values()`

注意：`build_device_section` 中 `for (device_id, device_state)` 有两处（line 90 和 line 122），都不使用 key。

**Files:** `nexus-server/src/context.rs`

---

### FIX-11: Delete dead functions in db.rs

| 函数 | 处理 |
|------|------|
| `update_device_name` | **删除** — 协议重构后 client 不再上报 device_name |
| `create_session` | **删除** — 被 FIX-02 的 `ensure_session` 取代 |

**Files:** `nexus-server/src/db.rs`

---

### FIX-12: session.rs — 调整 list_sessions

保留 `list_sessions`，但改为按 user_id 过滤（WebUI 需要列出某用户的所有 session）：

```rust
pub async fn list_sessions_by_user(&self, user_id: &str) -> Vec<String> {
    // 注意：当前 SessionHandle 不存储 user_id。
    // 短期方案：从 DB 查询。内存中的 sessions HashMap 只管活跃 session。
    // 此方法暂不实现，保留签名供 admin API 使用，实际数据从 DB 查。
    todo!("implement via db::list_sessions_by_user")
}
```

实际上更务实的做法：admin API 直接查 DB（`db.rs` 加函数），不经过 `SessionManager`。`SessionManager` 只管内存中的活跃 session。

删除当前的 `list_sessions`（返回所有 session，无 user 过滤，不安全）。

**Files:** `nexus-server/src/session.rs`, `nexus-server/src/db.rs`

---

### FIX-13: Remove commented-out module declarations in main.rs

删除 `// mod api;` 和 `// mod memory;`（line 8, 15）。stub 文件已确认不存在。

**Files:** `nexus-server/src/main.rs`

---

### FIX-14: Delete `DeviceOffline` variant (never constructed)

`RouteError::DeviceOffline(String)` 从未被构造。当前代码只区分 "not found" 和 "send failed"。

1. 删除 `DeviceOffline(String)` 变体
2. 删除 `tools_registry.rs` 中对应的 `Display` match arm
3. 删除 `agent_loop.rs:321` 中对应的 match arm

**Files:** `nexus-server/src/tools_registry.rs`, `nexus-server/src/agent_loop.rs`

---

## P3-LOW

### FIX-15: Stale `device_id` comments in state.rs and tools_registry.rs

`state.rs` 顶部注释多处使用 `device_id`，但内部 key 现在是 token。更新注释中的 `device_id` → `device_key`（指代 token）。

`tools_registry.rs` 中 `cancel_pending_requests_for_device` 参数名仍是 `device_id`，改为 `device_key`。

**Files:** `nexus-server/src/state.rs`, `nexus-server/src/tools_registry.rs`

---

### FIX-16: `ConnHandle.handle` never read

`discord/mod.rs:29`：`handle: JoinHandle<()>` 从未被读取。改为 `_handle: JoinHandle<()>`。
（handle 保持存活以防 task 被 detach，这是正确行为，只需标记为有意不读取。）

**Files:** `nexus-server/src/channels/discord/mod.rs`

---

## ADMIN API（新功能）

### API-01: Admin API 端点设计

在 M3 实现完整的 Admin API，使用户可以通过 WebUI 管理所有配置。

#### 端点清单

| 方法 | 路径 | 权限 | 描述 |
|------|------|------|------|
| `POST` | `/api/auth/register` | Public（admin_token 可选） | 已有 |
| `POST` | `/api/auth/login` | Public | 已有 |
| `POST` | `/api/device-tokens` | JWT | 已有，需迁移 SQL 到 db.rs（FIX-06） |
| `GET` | `/api/device-tokens` | JWT | **新增**：列出当前用户所有 device token |
| `DELETE` | `/api/device-tokens/:token` | JWT | **新增**：吊销 device token |
| `POST` | `/api/discord-config` | JWT | 已有 |
| `GET` | `/api/discord-config` | JWT | **新增**：获取当前用户的 Discord 配置 |
| `DELETE` | `/api/discord-config` | JWT | **新增**：删除当前用户的 Discord 配置 |
| `GET` | `/api/sessions` | JWT | **新增**：列出当前用户的所有 session |
| `DELETE` | `/api/sessions/:session_id` | JWT | **新增**：删除指定 session |
| `GET` | `/api/llm-config` | JWT + admin | **新增**：获取当前 LLM 配置 |
| `PUT` | `/api/llm-config` | JWT + admin | **新增**：更新 LLM 配置（运行时热更新） |

#### API-01a: GET /api/device-tokens

返回当前用户的所有 device token（脱敏显示 token）。

**db.rs 新增：**
```rust
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct DeviceTokenInfo {
    pub token: String,
    pub device_name: Option<String>,
    pub revoked: bool,
    pub created_at: Option<chrono::NaiveDateTime>,
}

pub async fn list_device_tokens(
    db: &PgPool,
    user_id: &str,
) -> Result<Vec<DeviceTokenInfo>, sqlx::Error> {
    sqlx::query_as::<_, DeviceTokenInfo>(
        "SELECT token, device_name, revoked, created_at FROM device_tokens WHERE user_id = $1 ORDER BY created_at DESC"
    )
    .bind(user_id)
    .fetch_all(db)
    .await
}
```

**auth.rs handler：**
```rust
pub async fn list_device_tokens(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    match db::list_device_tokens(&state.db, &claims.sub).await {
        Ok(tokens) => Json(tokens).into_response(),
        Err(e) => {
            tracing::error!("list_device_tokens error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to list tokens").into_response()
        }
    }
}
```

#### API-01b: DELETE /api/device-tokens/:token

吊销指定 device token（soft delete：设置 `revoked = TRUE`）。

**db.rs 新增：**
```rust
pub async fn revoke_device_token(
    db: &PgPool,
    token: &str,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE device_tokens SET revoked = TRUE WHERE token = $1 AND user_id = $2 AND revoked = FALSE"
    )
    .bind(token)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() > 0)
}
```

#### API-01c: GET /api/discord-config

返回当前用户的 Discord 配置。已有 `db::get_discord_config_by_user_id`。

#### API-01d: DELETE /api/discord-config

删除当前用户的 Discord 配置。

**db.rs 新增：**
```rust
pub async fn delete_discord_config(
    db: &PgPool,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM discord_configs WHERE user_id = $1")
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}
```

#### API-01e: GET /api/sessions

列出当前用户的所有 session（从 DB 查询）。

**db.rs 新增：**
```rust
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn list_sessions_by_user(
    db: &PgPool,
    user_id: &str,
) -> Result<Vec<SessionInfo>, sqlx::Error> {
    sqlx::query_as::<_, SessionInfo>(
        "SELECT session_id, created_at FROM sessions WHERE user_id = $1 ORDER BY created_at DESC"
    )
    .bind(user_id)
    .fetch_all(db)
    .await
}
```

#### API-01f: DELETE /api/sessions/:session_id

删除指定 session 及其所有消息。需要：
1. 验证 session 属于当前用户
2. 从内存中移除活跃 session（如果存在）
3. 从 DB 删除 messages + session

**db.rs 新增：**
```rust
pub async fn delete_session(
    db: &PgPool,
    session_id: &str,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    // 先删 messages（外键约束）
    sqlx::query("DELETE FROM messages WHERE session_id = $1")
        .bind(session_id)
        .execute(db)
        .await?;
    let result = sqlx::query(
        "DELETE FROM sessions WHERE session_id = $1 AND user_id = $2"
    )
    .bind(session_id)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() > 0)
}
```

**handler 还需要清理内存中的 session：**
```rust
// 清理内存中的活跃 session
state.bus.unregister_session(&session_id);
state.session_manager.remove_session(&session_id).await;
```

#### API-01g: GET/PUT /api/llm-config (Admin Only)

**GET** 返回当前 LLM 配置（脱敏 api_key）。
**PUT** 更新 LLM 配置（运行时热更新）。

需要将 `config.llm` 改为 `Arc<RwLock<LlmConfig>>` 以支持运行时修改：

**config.rs 修改：**
```rust
// ServerConfig 中
pub llm: Arc<RwLock<LlmConfig>>,
```

**state.rs / agent_loop.rs 修改：**
所有读取 `state.config.llm` 的地方改为 `state.config.llm.read().await`。

**auth.rs handler：**
```rust
pub async fn get_llm_config(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    if !claims.is_admin {
        return (StatusCode::FORBIDDEN, "Admin only").into_response();
    }
    let llm = state.config.llm.read().await;
    Json(json!({
        "api_base": llm.api_base,
        "api_key": format!("{}...{}", &llm.api_key[..8], &llm.api_key[llm.api_key.len()-4..]),
        "model": llm.model,
    })).into_response()
}

pub async fn update_llm_config(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Json(payload): Json<UpdateLlmConfigRequest>,
) -> Response {
    if !claims.is_admin {
        return (StatusCode::FORBIDDEN, "Admin only").into_response();
    }
    let mut llm = state.config.llm.write().await;
    if let Some(api_base) = payload.api_base { llm.api_base = api_base; }
    if let Some(api_key) = payload.api_key { llm.api_key = api_key; }
    if let Some(model) = payload.model { llm.model = model; }
    (StatusCode::OK, "LLM config updated").into_response()
}
```

**Files:** `nexus-server/src/config.rs`, `nexus-server/src/state.rs`, `nexus-server/src/auth.rs`, `nexus-server/src/agent_loop.rs`, `nexus-server/src/main.rs`

---

### API-02: Route Registration

在 `main.rs` 中注册所有新端点：

```rust
let protected = Router::new()
    // Device tokens
    .route("/api/device-tokens", axum::routing::post(auth::create_device_token))
    .route("/api/device-tokens", axum::routing::get(auth::list_device_tokens))
    .route("/api/device-tokens/:token", axum::routing::delete(auth::revoke_device_token))
    // Discord config
    .route("/api/discord-config", axum::routing::post(auth::upsert_discord_config))
    .route("/api/discord-config", axum::routing::get(auth::get_discord_config))
    .route("/api/discord-config", axum::routing::delete(auth::delete_discord_config))
    // Sessions
    .route("/api/sessions", axum::routing::get(auth::list_sessions))
    .route("/api/sessions/:session_id", axum::routing::delete(auth::delete_session))
    // LLM config (admin only)
    .route("/api/llm-config", axum::routing::get(auth::get_llm_config))
    .route("/api/llm-config", axum::routing::put(auth::update_llm_config))
    .layer(axum::middleware::from_fn_with_state(state.clone(), auth::jwt_middleware));
```

**Files:** `nexus-server/src/main.rs`

---

## DEFERRED (不在 M3 范围)

| # | Issue | 原因 |
|---|-------|------|
| I9 | `BrowserOutbound::Connected` in nexus-gateway | Gateway 是独立 crate，M4 处理 |
| M3 | Unused fields on InboundEvent/OutboundEvent | 近期 Discord 媒体支持需要 |
| M6 | Error type inconsistency (统一 AgentError) | 较大重构，M4+ |
| M8 | Duplicate JWT in nexus-gateway | 抽到 nexus-common 是独立重构 |
| M9 | Typing timeout task not cancelled on disconnect | 已有 120s safety net，可接受 |

---

## 执行顺序

1. **FIX-01** (P0) — 修复 tools 传递，立即恢复多步推理
2. **FIX-02** (P0) — Raw SQL 移到 db.rs
3. **FIX-03** (P1) — Session 创建逻辑去重
4. **FIX-04** (P1) — Session 内存泄漏修复
5. **FIX-05** (P1) — Config SSOT
6. **FIX-06** (P1) — auth.rs SQL 移到 db.rs
7. **FIX-07** (P1) — main.rs 初始化简化
8. **FIX-08** (P1) — bus 日志补全
9. **FIX-09 ~ FIX-16** (P2/P3) — 死代码清理、命名修复
10. **API-01 + API-02** — Admin API 全部端点
11. **编译验证** — `cargo build` + `cargo test`
