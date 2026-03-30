# M3 实现计划：JWT 认证 + DB 持久化 + E2E 工具链路

> 创建时间：2026-03-30
> 前置条件：M2 握手与工具注册已验收（M0–M2 ✅）
> 目标：打通完整 E2E 链路，满足 M3 验收条件

---

## 背景与根本问题

当前 `sender_id` 在 `nexus-gateway/src/browser.rs:81` 被赋值为随机 `chat_id`（UUID），而
`agent_loop.rs` 以 `event.sender_id` 作为 `user_id` 去查找该用户的设备：

```rust
// context.rs - build_device_section 和 get_all_tools_schema 都用这个 user_id
devices_by_user.get(user_id) // → None，因为 UUID 不是真实 user_id
```

**结果**：`get_all_tools_schema` 返回空 → MockLLM 回复 "No tools available" → E2E 不通。

**修复路径**：
1. nexus-gateway 提取 JWT，从 Claims 取出真实 `user_id`
2. nexus-server 实现注册/登录端点签发 JWT
3. DB 实现 session 持久化，context.rs 接入历史

---

## 阶段划分

```
Phase 1 → Cargo.toml 依赖
Phase 2 → DB Schema + 核心 CRUD（create_user, get_user_by_email, create_session, save_message, get_session_history）
Phase 3 → auth.rs 实现（register, login, verify_jwt, jwt_middleware）
Phase 4 → nexus-gateway JWT 验证（sender_id = user_id）
Phase 5 → context.rs + agent_loop.rs DB 接入（历史拼装、消息持久化）
Phase 6 → E2E 验证
```

每个阶段均可独立执行，执行前先阅读列出的文件，执行后运行验证命令。

---

## Phase 1 — Cargo.toml 依赖

### 目标
给 nexus-server 和 nexus-gateway 添加密码哈希与 JWT 依赖。

### 文件操作

**`nexus-server/Cargo.toml`** — 追加以下依赖：
```toml
bcrypt = "0.15"
jsonwebtoken = "9"
```

**`nexus-gateway/Cargo.toml`** — 追加以下依赖：
```toml
jsonwebtoken = "9"
```

**`nexus-server/src/config.rs`** — `ServerConfig` 结构体新增字段：
```rust
pub jwt_secret: String,
pub bcrypt_cost: u32,
```

`load_config()` 中新增读取：
```rust
let jwt_secret = std::env::var("JWT_SECRET")
    .unwrap_or_else(|_| panic!("JWT_SECRET 未设置"));
let bcrypt_cost = std::env::var("BCRYPT_COST")
    .ok()
    .and_then(|v| v.parse::<u32>().ok())
    .unwrap_or(12);
```

### 验证
```bash
cd NEXUS && cargo build 2>&1 | grep -E "^error"
# 期望：无 error 输出
```

---

## Phase 2 — DB Schema + 核心 CRUD

### 前置：先读这些文件
- `nexus-server/src/db.rs`（现有 `init_db` 实现，users 和 device_tokens 表）
- `nexus-server/src/main.rs`（`db::init_db` 调用位置）

### Schema 变更（修改 `init_db` 中的 CREATE TABLE）

**users 表**：需新增 `password_hash` 和 `is_admin` 列。
由于 `IF NOT EXISTS` 不会修改已存在的表，需用 `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`：

```sql
-- 追加到 init_db，在现有两条 sqlx::query 之后
ALTER TABLE users ADD COLUMN IF NOT EXISTS password_hash TEXT NOT NULL DEFAULT '';
ALTER TABLE users ADD COLUMN IF NOT EXISTS is_admin BOOLEAN NOT NULL DEFAULT FALSE;
```

**新建 sessions 表**：
```sql
CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(user_id),
    created_at TIMESTAMPTZ DEFAULT NOW(),
    last_consolidated TEXT
);
```

**新建 messages 表**：
```sql
CREATE TABLE IF NOT EXISTS messages (
    message_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    role TEXT NOT NULL,                   -- "user" | "assistant" | "tool"
    content TEXT NOT NULL,
    tool_call_id TEXT,                    -- 仅 role=tool 时非 NULL
    is_consolidated BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
```

### 实现的函数（在 db.rs 中逐一实现）

全部在已有注释 TODO 下方实现，不删除注释（注释即文档）。

#### `create_user`
```rust
pub async fn create_user(
    db: &PgPool,
    email: &str,
    password_hash: &str,
    is_admin: bool,
) -> Result<String, sqlx::Error> {
    let user_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO users (user_id, email, password_hash, is_admin) VALUES ($1, $2, $3, $4)"
    )
    .bind(&user_id)
    .bind(email)
    .bind(password_hash)
    .bind(is_admin)
    .execute(db)
    .await?;
    Ok(user_id)
}
```

#### `User` 结构体（新增，供 get_user_by_email 返回）
```rust
pub struct User {
    pub user_id: String,
    pub email: String,
    pub password_hash: String,
    pub is_admin: bool,
}
```

#### `get_user_by_email`
```rust
pub async fn get_user_by_email(
    db: &PgPool,
    email: &str,
) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        "SELECT user_id, email, password_hash, is_admin FROM users WHERE email = $1",
        email
    )
    .fetch_optional(db)
    .await
}
```

#### `create_session`
```rust
pub async fn create_session(
    db: &PgPool,
    user_id: &str,
) -> Result<String, sqlx::Error> {
    let session_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO sessions (session_id, user_id) VALUES ($1, $2)"
    )
    .bind(&session_id)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(session_id)
}
```

#### `save_message`
```rust
pub async fn save_message(
    db: &PgPool,
    session_id: &str,
    role: &str,
    content: &str,
    tool_call_id: Option<&str>,
) -> Result<String, sqlx::Error> {
    let message_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO messages (message_id, session_id, role, content, tool_call_id)
         VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(&message_id)
    .bind(session_id)
    .bind(role)
    .bind(content)
    .bind(tool_call_id)
    .execute(db)
    .await?;
    Ok(message_id)
}
```

#### `get_session_history`
```rust
pub async fn get_session_history(
    db: &PgPool,
    session_id: &str,
) -> Result<Vec<serde_json::Value>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT role, content, tool_call_id
         FROM messages
         WHERE session_id = $1 AND is_consolidated = FALSE
         ORDER BY created_at ASC",
        session_id
    )
    .fetch_all(db)
    .await?;

    let messages = rows.iter().map(|row| {
        let mut msg = serde_json::json!({
            "role": row.role,
            "content": row.content,
        });
        if let Some(id) = &row.tool_call_id {
            msg["tool_call_id"] = serde_json::Value::String(id.clone());
        }
        msg
    }).collect();

    Ok(messages)
}
```

### 验证
```bash
cd NEXUS && cargo build --package nexus-server 2>&1 | grep -E "^error"
# 期望：无 error

# 运行后检查表是否创建
# psql $DATABASE_URL -c "\d users"
# 期望：password_hash 和 is_admin 列存在
# psql $DATABASE_URL -c "\d sessions"
# psql $DATABASE_URL -c "\d messages"
```

---

## Phase 3 — auth.rs 实现

### 前置：先读这些文件
- `nexus-server/src/auth.rs`（完整的 TODO 规格注释，逐条实现）
- `nexus-server/src/config.rs`（Phase 1 之后的版本，含 jwt_secret）
- `nexus-server/src/main.rs`（当前路由，`mod auth` 已注释掉）

### 依赖 import
```rust
use axum::{
    extract::{Json, State},
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use bcrypt::{hash, verify, DEFAULT_COST};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use crate::config::ServerConfig;
use crate::db;
use crate::state::AppState;
```

### 结构体
```rust
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub admin_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user_id: String,
    pub is_admin: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,       // user_id
    pub is_admin: bool,
    pub exp: usize,        // Unix timestamp（秒）
}
```

### `verify_jwt` （先实现，其他函数依赖它）
```rust
pub fn verify_jwt(token: &str, secret: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    let key = DecodingKey::from_secret(secret.as_bytes());
    let data = decode::<Claims>(token, &key, &Validation::default())?;
    Ok(data.claims)
}
```

### `sign_jwt` （内部辅助函数）
```rust
fn sign_jwt(user_id: &str, is_admin: bool, secret: &str) -> Result<String, jsonwebtoken::errors::Error> {
    let exp = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::days(7))
        .unwrap()
        .timestamp() as usize;
    let claims = Claims { sub: user_id.to_string(), is_admin, exp };
    encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes()))
}
```

### `register` handler
```rust
pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> impl IntoResponse {
    let is_admin = payload.admin_token.as_deref()
        .map(|t| t == state.config.admin_token)
        .unwrap_or(false);

    let password_hash = match hash(&payload.password, state.config.bcrypt_cost) {
        Ok(h) => h,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Hash failed").into_response(),
    };

    let user_id = match db::create_user(&state.db, &payload.email, &password_hash, is_admin).await {
        Ok(id) => id,
        Err(_) => return (StatusCode::CONFLICT, "Email already exists").into_response(),
    };

    let token = match sign_jwt(&user_id, is_admin, &state.config.jwt_secret) {
        Ok(t) => t,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "JWT sign failed").into_response(),
    };

    Json(AuthResponse { token, user_id, is_admin }).into_response()
}
```

### `login` handler
```rust
pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> impl IntoResponse {
    let user = match db::get_user_by_email(&state.db, &payload.email).await {
        Ok(Some(u)) => u,
        _ => return (StatusCode::UNAUTHORIZED, "Invalid credentials").into_response(),
    };

    let ok = verify(&payload.password, &user.password_hash).unwrap_or(false);
    if !ok {
        return (StatusCode::UNAUTHORIZED, "Invalid credentials").into_response();
    }

    let token = match sign_jwt(&user.user_id, user.is_admin, &state.config.jwt_secret) {
        Ok(t) => t,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "JWT sign failed").into_response(),
    };

    Json(AuthResponse { token, user_id: user.user_id, is_admin: user.is_admin }).into_response()
}
```

### `jwt_middleware`
```rust
pub async fn jwt_middleware(
    State(state): State<AppState>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let auth_header = req.headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match auth_header {
        Some(token) => match verify_jwt(token, &state.config.jwt_secret) {
            Ok(claims) => {
                req.extensions_mut().insert(claims);
                next.run(req).await
            }
            Err(_) => (StatusCode::UNAUTHORIZED, "Invalid token").into_response(),
        },
        None => (StatusCode::UNAUTHORIZED, "Missing Authorization header").into_response(),
    }
}
```

### main.rs 修改

解除注释 `mod auth`，添加路由：
```rust
// mod auth;   →   mod auth;

let app = Router::new()
    .route("/ws", get(ws::ws_handler))
    .route("/api/auth/register", axum::routing::post(auth::register))
    .route("/api/auth/login", axum::routing::post(auth::login))
    .fallback(...)
    .with_state(state);
```

### 验证
```bash
cargo build --package nexus-server 2>&1 | grep -E "^error"

# 功能验证（服务器运行中）
curl -s -X POST http://localhost:8080/api/auth/register \
  -H "Content-Type: application/json" \
  -d '{"email":"test@example.com","password":"password123"}' | jq .
# 期望：{"token":"eyJ...","user_id":"...","is_admin":false}

curl -s -X POST http://localhost:8080/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"test@example.com","password":"password123"}' | jq .
# 期望：同上结构
```

---

## Phase 4 — nexus-gateway JWT 验证

### 前置：先读这些文件
- `nexus-gateway/src/browser.rs`（`forward_browser_message` 中 `sender_id: chat_id`，行 81）
- `nexus-gateway/src/main.rs`（配置加载与路由）
- `nexus-gateway/src/state.rs`（AppState 结构）

### 目标：`sender_id = user_id`，不再是 `chat_id`

**nexus-gateway/src/state.rs** — AppState 新增 JWT secret 字段（或通过 Arc<Config> 传递）：
```rust
pub struct AppState {
    pub nexus_tx: Arc<tokio::sync::RwLock<Option<mpsc::Sender<String>>>>,
    pub browser_conns: DashMap<String, mpsc::Sender<String>>,
    pub gateway_token: String,
    pub jwt_secret: String,    // 新增
}

impl AppState {
    pub fn new(gateway_token: String, jwt_secret: String) -> Arc<Self> {
        Arc::new(Self {
            nexus_tx: Arc::new(tokio::sync::RwLock::new(None)),
            browser_conns: DashMap::new(),
            gateway_token,
            jwt_secret,
        })
    }
}
```

**nexus-gateway/src/main.rs** — 读取 `JWT_SECRET` 环境变量，传入 AppState。

**nexus-gateway/src/browser.rs** — 修改 `browser_ws_handler`：

关键变更：从 `Authorization` header 提取 JWT，提取失败返回 401，成功后将 `user_id` 传入 `browser_connection`。

```rust
use axum::extract::TypedHeader;
use axum::headers::{Authorization, authorization::Bearer};

pub async fn browser_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
) -> impl IntoResponse {
    // 验证 JWT
    let claims = match verify_jwt(auth.token(), &state.jwt_secret) {
        Ok(c) => c,
        Err(_) => return (StatusCode::UNAUTHORIZED, "Invalid token").into_response(),
    };
    let user_id = claims.sub;
    ws.on_upgrade(move |socket| browser_connection(socket, state, user_id))
        .into_response()
}
```

`browser_connection` 签名变更：
```rust
async fn browser_connection(socket: WebSocket, state: SharedState, user_id: String)
```

`forward_browser_message` 调用变更：
```rust
// 之前：sender_id: chat_id.to_string()
// 之后：
sender_id: user_id.clone(),   // 真实 user_id，来自 JWT Claims.sub
```

**注意**：`TypedHeader` 需要 axum-extra crate，或改用手动提取方式：
```rust
// 手动提取（不需要额外依赖）
pub async fn browser_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let user_id = match token {
        Some(t) => match verify_jwt(t, &state.jwt_secret) {
            Ok(c) => c.sub,
            Err(_) => return (StatusCode::UNAUTHORIZED, "Invalid token").into_response(),
        },
        None => return (StatusCode::UNAUTHORIZED, "Missing auth").into_response(),
    };

    ws.on_upgrade(move |socket| browser_connection(socket, state, user_id))
        .into_response()
}
```

`verify_jwt` 在 gateway 中本地实现（与 server 相同逻辑，不依赖 nexus-server crate）：
```rust
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub is_admin: bool,
    pub exp: usize,
}

fn verify_jwt(token: &str, secret: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    let key = jsonwebtoken::DecodingKey::from_secret(secret.as_bytes());
    let data = jsonwebtoken::decode::<Claims>(token, &key, &jsonwebtoken::Validation::default())?;
    Ok(data.claims)
}
```

### 验证
```bash
cargo build --package nexus-gateway 2>&1 | grep -E "^error"

# wscat 测试（获取 JWT 后连接）
TOKEN=$(curl -s -X POST http://localhost:8080/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"test@example.com","password":"password123"}' | jq -r .token)

wscat -c "ws://localhost:9090/ws/browser" -H "Authorization: Bearer $TOKEN"
# 期望：连接建立，发消息后 sender_id = 真实 user_id
```

---

## Phase 5 — context.rs + agent_loop.rs DB 接入

### 前置：先读这些文件
- `nexus-server/src/context.rs`（`build_message_history` 返回 `Vec::new()`，行 140-144）
- `nexus-server/src/agent_loop.rs`（完整实现，找到 `run_session` 和消息追加点）
- `nexus-server/src/session.rs`（SessionManager，`get_or_create_session` 签名）

### context.rs — `build_message_history` 接入 DB

替换 `build_message_history` 的 TODO 实现：
```rust
pub async fn build_message_history(
    state: &AppState,
    session_id: &str,
) -> Vec<serde_json::Value> {
    use nexus_common::consts::MAX_HISTORY_MESSAGES;

    match db::get_session_history(&state.db, session_id).await {
        Ok(messages) => truncate_and_fix_orphans(messages, MAX_HISTORY_MESSAGES),
        Err(e) => {
            tracing::warn!("get_session_history failed: {}", e);
            Vec::new()
        }
    }
}
```

### agent_loop.rs — 三处 DB 接入

阅读 `agent_loop.rs` 全文后，找到以下三个位置：

**1. session 创建**（`run_session` 开头，首次处理消息时）：
```rust
// 在创建 session 或首次使用 session_id 时，对应持久化到 DB
let db_session_id = db::create_session(&state.db, &event.sender_id).await
    .unwrap_or_else(|_| session_id.clone()); // 降级：用内存 session_id
```

**2. 用户消息写入**（处理 `InboundEvent.content` 之后）：
```rust
let _ = db::save_message(&state.db, &db_session_id, "user", &event.content, None).await;
```

**3. assistant/tool 消息写入**（LLM 返回 tool_calls 后，以及最终回复后）：
```rust
// tool_call assistant turn
let _ = db::save_message(&state.db, &db_session_id, "assistant",
    &serde_json::to_string(&tool_calls_json).unwrap_or_default(), None).await;

// tool result
let _ = db::save_message(&state.db, &db_session_id, "tool",
    &result.output, Some(&request_id)).await;

// final reply
let _ = db::save_message(&state.db, &db_session_id, "assistant", &final_reply, None).await;
```

**注意**：这些写入用 `let _ =` 忽略错误，不中断 agent_loop。M3 阶段不要求完美持久化，只要链路通即可。

### 验证
```bash
cargo build --package nexus-server 2>&1 | grep -E "^error"

# 运行 E2E 后检查
# psql $DATABASE_URL -c "SELECT count(*) FROM messages;"
# 期望：count > 0
```

---

## Phase 6 — E2E 验收

### 前置准备

1. 环境变量（`.env` 文件或 shell export）：
```bash
DATABASE_URL=postgres://user:pass@localhost/nexus
ADMIN_TOKEN=admin-secret
JWT_SECRET=dev-jwt-secret-at-least-32-chars-long
NEXUS_GATEWAY_TOKEN=dev-token
```

2. nexus-gateway 的 `.env`：
```bash
JWT_SECRET=dev-jwt-secret-at-least-32-chars-long
NEXUS_GATEWAY_TOKEN=dev-token
```

### 验收步骤（对应 ACCEPTANCE.md M3 条目）

**步骤 1：启动所有服务**
```bash
# Terminal 1
cd NEXUS && cargo run --package nexus-server

# Terminal 2
cd NEXUS && cargo run --package nexus-gateway

# Terminal 3
cd NEXUS && cargo run --package nexus-client
```

**步骤 2：注册用户并获取 JWT**
```bash
TOKEN=$(curl -s -X POST http://localhost:8080/api/auth/register \
  -H "Content-Type: application/json" \
  -d '{"email":"demo@nexus.dev","password":"password123"}' | jq -r .token)
echo "TOKEN=$TOKEN"
```

**步骤 3：通过 wscat 发送消息（验收条件 1）**
```bash
# 发送一条会触发工具调用的消息
wscat -c "ws://localhost:9090/ws/browser" -H "Authorization: Bearer $TOKEN" \
  -x '{"type":"chat","content":"list files in current directory"}'
```

预期 nexus-server 日志顺序：
```
LLM 调用 → ToolExecutionRequest{tool:"list_dir",...} → ToolExecutionResult → 最终回复
```

预期 wscat 收到：
```json
{"type":"chat","content":"Files: ...","chat_id":"..."}
```

**步骤 4：多轮对话（验收条件 2）**

发第二条消息，检查 nexus-server 日志中 LLM 请求的 messages 数组包含前一轮消息。

**步骤 5：断线测试（验收条件 3）**
```bash
# 强制终止 nexus-client
# 检查 nexus-server 日志：挂起的 oneshot channel 被 drop，返回 exit_code=-2
```

**步骤 6：DB 验证**
```sql
SELECT count(*) FROM sessions;   -- > 0
SELECT count(*) FROM messages;   -- > 0，含 user/assistant/tool 三种 role
```

---

## Anti-Pattern 警示

| 风险 | 预防措施 |
|------|----------|
| bcrypt 在异步代码中阻塞事件循环 | 用 `tokio::task::spawn_blocking(|| bcrypt::hash(...))` |
| JWT_SECRET 长度不足 HS256 安全要求 | 必须 ≥ 32 字符，生产环境用随机生成值 |
| `ALTER TABLE ADD COLUMN` 对已有行的 NOT NULL 约束 | password_hash 用 `DEFAULT ''`，后续手动清理测试数据 |
| gateway 和 server JWT 验证逻辑不同步 | Claims 结构体完全相同（sub, is_admin, exp），两边均用 HS256 |
| agent_loop DB 写入失败中断主循环 | 所有 save_message 调用用 `let _ =` 忽略错误 |

---

## 依赖关系图

```
Phase 1 (deps)
    ↓
Phase 2 (DB schema + CRUD)
    ↓
Phase 3 (auth.rs) ←── 需要 Phase 2 的 create_user / get_user_by_email
    ↓
Phase 4 (gateway JWT) ←── 需要 Phase 3 的 JWT secret 对齐
    ↓
Phase 5 (context + agent_loop) ←── 需要 Phase 2 的 create_session / save_message / get_session_history
    ↓
Phase 6 (E2E) ←── 需要全部前序 Phase
```

Phase 2–3 和 Phase 4 可以在不同 session 中并行执行，但 Phase 4 需要等 Phase 3 确定 JWT_SECRET 与 Claims 格式后再开始。

---

## 完成后预期状态（M3 验收通过）

- `POST /api/auth/register` 和 `POST /api/auth/login` 正常工作
- `ws://localhost:9090/ws/browser` 需要有效 JWT 才能连接
- 浏览器（或 wscat）发送消息后，日志显示完整 ReAct 链路
- `sender_id = user_id` = JWT Claims.sub = DB users.user_id
- `devices_by_user[user_id]` 非空，工具路由正确
- DB 中有 sessions 和 messages 记录
