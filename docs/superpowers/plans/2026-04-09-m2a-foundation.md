# M2a: Server Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the nexus-server foundation: DB schema, auth, REST API, and WebSocket device handler — everything needed before the agent loop.

**Architecture:** Axum HTTP server with PostgreSQL (sqlx), JWT auth, DashMap-based device routing, WebSocket handler for client devices. All state in a shared `AppState` behind `Arc`.

**Tech Stack:** Rust 2024 edition, axum 0.8, sqlx (PostgreSQL), jsonwebtoken, bcrypt, dashmap, tokio, serde, tracing, uuid, tower-http

**Spec:** `docs/superpowers/specs/2026-04-09-m2-server-design.md` (sections 1-8, 12)

**Database:** PostgreSQL on `localhost:5432`, db/user/password all `nexus`

---

## File Map

| File | Responsibility |
|---|---|
| `Cargo.toml` (workspace) | Add nexus-server member |
| `nexus-server/Cargo.toml` | Crate deps |
| `nexus-common/src/consts.rs` | New server constants |
| `nexus-common/src/protocol.rs` | File transfer protocol messages |
| `nexus-server/src/main.rs` | Entry point, env loading, startup |
| `nexus-server/src/config.rs` | ServerConfig, LlmConfig structs |
| `nexus-server/src/state.rs` | AppState with DashMaps |
| `nexus-server/src/db/mod.rs` | init_db, PgPool alias |
| `nexus-server/src/db/users.rs` | Users CRUD |
| `nexus-server/src/db/sessions.rs` | Sessions CRUD |
| `nexus-server/src/db/messages.rs` | Messages CRUD + history reconstruction |
| `nexus-server/src/db/devices.rs` | Device tokens CRUD |
| `nexus-server/src/db/system_config.rs` | System config CRUD |
| `nexus-server/src/auth/mod.rs` | JWT sign/verify, middleware, register/login |
| `nexus-server/src/auth/device.rs` | Device token API endpoints |
| `nexus-server/src/api.rs` | User, session, file endpoints |
| `nexus-server/src/file_store.rs` | File upload/download, cleanup |
| `nexus-server/src/session.rs` | SessionHandle struct |
| `nexus-server/src/ws.rs` | WebSocket handler, heartbeat reaper |

---

### Task 1: nexus-common Protocol Additions

**Files:**
- Modify: `nexus-common/src/protocol.rs`
- Modify: `nexus-common/src/consts.rs`

- [ ] **Step 1: Add file transfer messages to protocol.rs**

Add these variants to `ServerToClient`:

```rust
FileRequest {
    request_id: String,
    path: String,
},
FileSend {
    request_id: String,
    filename: String,
    content_base64: String,
    destination: String,
},
```

Add these variants to `ClientToServer`:

```rust
FileResponse {
    request_id: String,
    content_base64: String,
    mime_type: Option<String>,
    error: Option<String>,
},
FileSendAck {
    request_id: String,
    error: Option<String>,
},
```

- [ ] **Step 2: Add server constants to consts.rs**

Append to `nexus-common/src/consts.rs`:

```rust
// Server constants
pub const TOOL_EXECUTION_TIMEOUT_SEC: u64 = 120;
pub const MEMORY_TEXT_MAX_CHARS: usize = 4096;
pub const USER_MESSAGE_MAX_CHARS: usize = 4000;
pub const CONTEXT_COMPRESSION_THRESHOLD: usize = 16_000;
pub const FILE_UPLOAD_MAX_BYTES: usize = 25 * 1024 * 1024;
pub const FILE_CLEANUP_AGE_HOURS: u64 = 24;
pub const WEB_FETCH_MAX_BODY_BYTES: usize = 1_048_576;
pub const WEB_FETCH_MAX_OUTPUT_CHARS: usize = 50_000;
pub const WEB_FETCH_TIMEOUT_SEC: u64 = 15;
pub const WEB_FETCH_CONNECT_TIMEOUT_SEC: u64 = 10;
pub const WEB_FETCH_MAX_REDIRECTS: usize = 5;
pub const WEB_FETCH_CONCURRENT_MAX: usize = 50;
pub const DB_POOL_MAX_CONNECTIONS: u32 = 200;
pub const RATE_LIMIT_CACHE_TTL_SEC: u64 = 60;
pub const JWT_EXPIRY_DAYS: i64 = 7;
pub const BCRYPT_COST: u32 = 12;
pub const HEARTBEAT_REAPER_INTERVAL_SEC: u64 = 30;
pub const CRON_POLL_INTERVAL_SEC: u64 = 10;
pub const COMPRESSION_SUMMARY_MAX_TOKENS: u32 = 12_000;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p nexus-common`
Expected: All existing + new tests pass

- [ ] **Step 4: Commit**

```bash
git add nexus-common/src/protocol.rs nexus-common/src/consts.rs
git commit -m "feat(common): add file transfer protocol + server constants"
```

---

### Task 2: Crate Scaffold + Config + State

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Create: `nexus-server/Cargo.toml`
- Create: `nexus-server/src/main.rs`
- Create: `nexus-server/src/config.rs`
- Create: `nexus-server/src/state.rs`
- Create: `nexus-server/src/session.rs`

- [ ] **Step 1: Add nexus-server to workspace**

In root `Cargo.toml`, change members:

```toml
[workspace]
members = ["nexus-common", "nexus-client", "nexus-server"]
```

- [ ] **Step 2: Create nexus-server/Cargo.toml**

```toml
[package]
name = "nexus-server"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[dependencies]
nexus-common = { path = "../nexus-common", features = ["axum"] }
tokio = { version = "1", features = ["full"] }
axum = { version = "0.8", features = ["ws", "multipart"] }
axum-extra = { version = "0.10", features = ["typed-header"] }
tower-http = { version = "0.6", features = ["cors", "limit"] }
sqlx = { version = "0.8", features = ["runtime-tokio", "tls-native-tls", "postgres", "chrono", "json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
dashmap = "6"
jsonwebtoken = "9"
bcrypt = "0.17"
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }
tokio-util = "0.7"
tokio-tungstenite = { version = "0.26", features = ["native-tls"] }
futures-util = "0.3"

[dev-dependencies]
reqwest = { version = "0.12", features = ["json", "multipart"] }
tempfile = "3"

[lints.rust]
unsafe_code = "forbid"
```

- [ ] **Step 3: Create config.rs**

```rust
//! Server configuration from env vars + DB-stored LLM config.

use serde::{Deserialize, Serialize};

/// Configuration loaded from environment variables at startup.
/// All fields are required — server panics if any are missing.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub database_url: String,
    pub admin_token: String,
    pub jwt_secret: String,
    pub server_port: u16,
    pub gateway_ws_url: String,
    pub gateway_token: String,
    pub skills_dir: String,
}

impl ServerConfig {
    pub fn from_env() -> Self {
        Self {
            database_url: env_required("DATABASE_URL"),
            admin_token: env_required("ADMIN_TOKEN"),
            jwt_secret: env_required("JWT_SECRET"),
            server_port: env_required("SERVER_PORT")
                .parse()
                .expect("SERVER_PORT must be a number"),
            gateway_ws_url: env_required("NEXUS_GATEWAY_WS_URL"),
            gateway_token: env_required("NEXUS_GATEWAY_TOKEN"),
            skills_dir: std::env::var("NEXUS_SKILLS_DIR")
                .unwrap_or_else(|_| {
                    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                    format!("{home}/.nexus/skills")
                }),
        }
    }
}

fn env_required(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{key} must be set"))
}

/// LLM provider config stored in system_config table. Hot-reloadable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub api_base: String,
    pub model: String,
    pub api_key: String,
    pub context_window: u32,
}
```

- [ ] **Step 4: Create session.rs**

```rust
//! Per-session handle: inbox channel + mutex for DB write serialization.

use crate::bus::InboundEvent;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

pub struct SessionHandle {
    pub user_id: String,
    pub inbox_tx: mpsc::Sender<InboundEvent>,
    pub lock: Arc<Mutex<()>>,
}
```

- [ ] **Step 5: Create state.rs**

```rust
//! Global application state shared across all handlers via Arc.

use crate::config::{LlmConfig, ServerConfig};
use crate::session::SessionHandle;
use dashmap::DashMap;
use nexus_common::protocol::ToolExecutionResult;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock, Semaphore};
use tokio_util::sync::CancellationToken;

pub type WsSink = futures_util::stream::SplitSink<
    axum::extract::ws::WebSocket,
    axum::extract::ws::Message,
>;

pub struct DeviceConnection {
    pub user_id: String,
    pub device_name: String,
    pub sink: Arc<Mutex<WsSink>>,
    pub last_seen: Arc<AtomicI64>,
    pub tools: Vec<Value>,
}

pub struct AppState {
    pub db: PgPool,
    pub config: ServerConfig,

    // Hot-reloadable LLM config
    pub llm_config: Arc<RwLock<Option<LlmConfig>>>,

    // Online device routing: "user_id:device_name" → connection
    pub devices: DashMap<String, DeviceConnection>,
    // user_id → [device_keys]
    pub devices_by_user: DashMap<String, Vec<String>>,

    // Tool request/response matching: device_key → { request_id → sender }
    pub pending: DashMap<String, DashMap<String, oneshot::Sender<ToolExecutionResult>>>,

    // Per-user tool schema cache
    pub tool_schema_cache: DashMap<String, Vec<Value>>,

    // Rate limiting: user_id → (remaining, last_refill)
    pub rate_limiter: DashMap<String, (u32, Instant)>,
    pub rate_limit_config: Arc<RwLock<u32>>,

    // Default soul cache
    pub default_soul: Arc<RwLock<Option<String>>>,

    // Session handles
    pub sessions: DashMap<String, SessionHandle>,

    // Web fetch concurrency limit
    pub web_fetch_semaphore: Arc<Semaphore>,

    // Outbound event channel (agent loop → channel handlers)
    pub outbound_tx: mpsc::Sender<crate::bus::OutboundEvent>,

    // Shutdown signal
    pub shutdown: CancellationToken,
}

impl AppState {
    pub fn device_key(user_id: &str, device_name: &str) -> String {
        format!("{user_id}:{device_name}")
    }
}
```

- [ ] **Step 6: Create stub main.rs**

```rust
mod config;
mod session;
mod state;

use config::ServerConfig;
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = ServerConfig::from_env();
    info!("NEXUS Server starting on port {}...", config.server_port);
}
```

- [ ] **Step 7: Build**

Run: `cargo build -p nexus-server`
Expected: Compiles (with dead_code warnings — that's fine for now)

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml nexus-server/
git commit -m "feat(server): scaffold crate with config, state, session structs"
```

---

### Task 3: Database Module — init_db + Users CRUD

**Files:**
- Create: `nexus-server/src/db/mod.rs`
- Create: `nexus-server/src/db/users.rs`
- Modify: `nexus-server/src/main.rs`

- [ ] **Step 1: Create db/mod.rs with init_db**

```rust
pub mod users;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tracing::info;

pub async fn init_db(database_url: &str) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(nexus_common::consts::DB_POOL_MAX_CONNECTIONS)
        .connect(database_url)
        .await
        .expect("Failed to connect to database");

    create_tables(&pool).await;
    info!("Database initialized");
    pool
}

async fn create_tables(pool: &PgPool) {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
            user_id        TEXT PRIMARY KEY,
            email          TEXT UNIQUE NOT NULL,
            password_hash  TEXT NOT NULL DEFAULT '',
            is_admin       BOOLEAN DEFAULT FALSE,
            soul           TEXT,
            memory_text    TEXT NOT NULL DEFAULT '',
            created_at     TIMESTAMPTZ DEFAULT NOW()
        )"
    )
    .execute(pool)
    .await
    .expect("Failed to create users table");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS device_tokens (
            token          TEXT PRIMARY KEY,
            user_id        TEXT NOT NULL REFERENCES users(user_id),
            device_name    TEXT NOT NULL,
            fs_policy      JSONB NOT NULL DEFAULT '{\"mode\":\"sandbox\"}',
            mcp_config     JSONB NOT NULL DEFAULT '[]',
            workspace_path TEXT NOT NULL DEFAULT '',
            shell_timeout  BIGINT NOT NULL DEFAULT 60,
            ssrf_whitelist JSONB NOT NULL DEFAULT '[]',
            created_at     TIMESTAMPTZ DEFAULT NOW(),
            UNIQUE(user_id, device_name)
        )"
    )
    .execute(pool)
    .await
    .expect("Failed to create device_tokens table");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS sessions (
            session_id     TEXT PRIMARY KEY,
            user_id        TEXT NOT NULL REFERENCES users(user_id),
            created_at     TIMESTAMPTZ DEFAULT NOW()
        )"
    )
    .execute(pool)
    .await
    .expect("Failed to create sessions table");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS messages (
            message_id     TEXT PRIMARY KEY,
            session_id     TEXT NOT NULL REFERENCES sessions(session_id),
            role           TEXT NOT NULL,
            content        TEXT NOT NULL,
            tool_call_id   TEXT,
            tool_name      TEXT,
            tool_arguments TEXT,
            compressed     BOOLEAN DEFAULT FALSE,
            created_at     TIMESTAMPTZ DEFAULT NOW()
        )"
    )
    .execute(pool)
    .await
    .expect("Failed to create messages table");

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, created_at)"
    )
    .execute(pool)
    .await
    .expect("Failed to create messages index");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS discord_configs (
            user_id           TEXT PRIMARY KEY REFERENCES users(user_id),
            bot_token         TEXT NOT NULL,
            bot_user_id       TEXT,
            owner_discord_id  TEXT,
            enabled           BOOLEAN DEFAULT TRUE,
            allowed_users     TEXT[] DEFAULT '{}',
            created_at        TIMESTAMPTZ DEFAULT NOW(),
            updated_at        TIMESTAMPTZ DEFAULT NOW()
        )"
    )
    .execute(pool)
    .await
    .expect("Failed to create discord_configs table");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS system_config (
            key            TEXT PRIMARY KEY,
            value          TEXT NOT NULL,
            updated_at     TIMESTAMPTZ DEFAULT NOW()
        )"
    )
    .execute(pool)
    .await
    .expect("Failed to create system_config table");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS cron_jobs (
            job_id          TEXT PRIMARY KEY,
            user_id         TEXT NOT NULL REFERENCES users(user_id),
            name            TEXT NOT NULL,
            enabled         BOOLEAN DEFAULT TRUE,
            cron_expr       TEXT,
            every_seconds   INTEGER,
            timezone        TEXT DEFAULT 'UTC',
            message         TEXT NOT NULL,
            channel         TEXT NOT NULL,
            chat_id         TEXT NOT NULL,
            delete_after_run BOOLEAN DEFAULT FALSE,
            deliver         BOOLEAN DEFAULT TRUE,
            next_run_at     TIMESTAMPTZ,
            last_run_at     TIMESTAMPTZ,
            run_count       INTEGER DEFAULT 0,
            created_at      TIMESTAMPTZ DEFAULT NOW()
        )"
    )
    .execute(pool)
    .await
    .expect("Failed to create cron_jobs table");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS skills (
            skill_id       TEXT PRIMARY KEY,
            user_id        TEXT NOT NULL REFERENCES users(user_id),
            name           TEXT NOT NULL,
            description    TEXT NOT NULL DEFAULT '',
            always_on      BOOLEAN DEFAULT FALSE,
            skill_path     TEXT NOT NULL,
            created_at     TIMESTAMPTZ DEFAULT NOW(),
            UNIQUE(user_id, name)
        )"
    )
    .execute(pool)
    .await
    .expect("Failed to create skills table");
}
```

- [ ] **Step 2: Create db/users.rs**

```rust
use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub user_id: String,
    pub email: String,
    pub password_hash: String,
    pub is_admin: bool,
    pub soul: Option<String>,
    pub memory_text: String,
    pub created_at: DateTime<Utc>,
}

pub async fn create_user(
    pool: &PgPool,
    user_id: &str,
    email: &str,
    password_hash: &str,
    is_admin: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO users (user_id, email, password_hash, is_admin) VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(email)
    .bind(password_hash)
    .bind(is_admin)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn find_by_email(pool: &PgPool, email: &str) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = $1")
        .bind(email)
        .fetch_optional(pool)
        .await
}

pub async fn find_by_id(pool: &PgPool, user_id: &str) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>("SELECT * FROM users WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
}

pub async fn update_soul(
    pool: &PgPool,
    user_id: &str,
    soul: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET soul = $1 WHERE user_id = $2")
        .bind(soul)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_memory(
    pool: &PgPool,
    user_id: &str,
    memory: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET memory_text = $1 WHERE user_id = $2")
        .bind(memory)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 3: Wire DB init into main.rs**

```rust
mod config;
mod db;
mod session;
mod state;

use config::ServerConfig;
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = ServerConfig::from_env();
    let pool = db::init_db(&config.database_url).await;
    info!("NEXUS Server starting on port {}...", config.server_port);
}
```

- [ ] **Step 4: Build and test DB connection**

Run: `DATABASE_URL=postgres://nexus:nexus@localhost:5432/nexus ADMIN_TOKEN=test JWT_SECRET=testsecret32charsminimumrequired SERVER_PORT=8080 NEXUS_GATEWAY_WS_URL=ws://localhost:9090/ws/nexus NEXUS_GATEWAY_TOKEN=test cargo run -p nexus-server`
Expected: "Database initialized" then "NEXUS Server starting on port 8080..."

- [ ] **Step 5: Commit**

```bash
git add nexus-server/src/db/ nexus-server/src/main.rs
git commit -m "feat(server): database init with all 8 tables + users CRUD"
```

---

### Task 4: Remaining DB CRUD Modules

**Files:**
- Create: `nexus-server/src/db/sessions.rs`
- Create: `nexus-server/src/db/messages.rs`
- Create: `nexus-server/src/db/devices.rs`
- Create: `nexus-server/src/db/system_config.rs`
- Modify: `nexus-server/src/db/mod.rs`

- [ ] **Step 1: Create db/sessions.rs**

```rust
use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Session {
    pub session_id: String,
    pub user_id: String,
    pub created_at: DateTime<Utc>,
}

pub async fn create_session(
    pool: &PgPool,
    session_id: &str,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO sessions (session_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(session_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_by_user(
    pool: &PgPool,
    user_id: &str,
) -> Result<Vec<Session>, sqlx::Error> {
    sqlx::query_as::<_, Session>(
        "SELECT * FROM sessions WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn delete_session(pool: &PgPool, session_id: &str) -> Result<bool, sqlx::Error> {
    // Delete messages first (FK constraint)
    sqlx::query("DELETE FROM messages WHERE session_id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;
    let result = sqlx::query("DELETE FROM sessions WHERE session_id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn find_by_id(
    pool: &PgPool,
    session_id: &str,
) -> Result<Option<Session>, sqlx::Error> {
    sqlx::query_as::<_, Session>("SELECT * FROM sessions WHERE session_id = $1")
        .bind(session_id)
        .fetch_optional(pool)
        .await
}
```

- [ ] **Step 2: Create db/messages.rs**

```rust
use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Message {
    pub message_id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_arguments: Option<String>,
    pub compressed: bool,
    pub created_at: DateTime<Utc>,
}

pub async fn insert(
    pool: &PgPool,
    message_id: &str,
    session_id: &str,
    role: &str,
    content: &str,
    tool_call_id: Option<&str>,
    tool_name: Option<&str>,
    tool_arguments: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO messages (message_id, session_id, role, content, tool_call_id, tool_name, tool_arguments)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(message_id)
    .bind(session_id)
    .bind(role)
    .bind(content)
    .bind(tool_call_id)
    .bind(tool_name)
    .bind(tool_arguments)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_uncompressed(
    pool: &PgPool,
    session_id: &str,
) -> Result<Vec<Message>, sqlx::Error> {
    sqlx::query_as::<_, Message>(
        "SELECT * FROM messages WHERE session_id = $1 AND compressed = FALSE ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
}

pub async fn list_paginated(
    pool: &PgPool,
    session_id: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<Message>, sqlx::Error> {
    sqlx::query_as::<_, Message>(
        "SELECT * FROM messages WHERE session_id = $1 ORDER BY created_at ASC LIMIT $2 OFFSET $3",
    )
    .bind(session_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

pub async fn mark_compressed(
    pool: &PgPool,
    message_ids: &[String],
) -> Result<(), sqlx::Error> {
    if message_ids.is_empty() {
        return Ok(());
    }
    sqlx::query("UPDATE messages SET compressed = TRUE WHERE message_id = ANY($1)")
        .bind(message_ids)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 3: Create db/devices.rs**

```rust
use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct DeviceToken {
    pub token: String,
    pub user_id: String,
    pub device_name: String,
    pub fs_policy: serde_json::Value,
    pub mcp_config: serde_json::Value,
    pub workspace_path: String,
    pub shell_timeout: i64,
    pub ssrf_whitelist: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

pub async fn create_token(
    pool: &PgPool,
    token: &str,
    user_id: &str,
    device_name: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO device_tokens (token, user_id, device_name) VALUES ($1, $2, $3)",
    )
    .bind(token)
    .bind(user_id)
    .bind(device_name)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn find_by_token(
    pool: &PgPool,
    token: &str,
) -> Result<Option<DeviceToken>, sqlx::Error> {
    sqlx::query_as::<_, DeviceToken>("SELECT * FROM device_tokens WHERE token = $1")
        .bind(token)
        .fetch_optional(pool)
        .await
}

pub async fn list_by_user(
    pool: &PgPool,
    user_id: &str,
) -> Result<Vec<DeviceToken>, sqlx::Error> {
    sqlx::query_as::<_, DeviceToken>(
        "SELECT * FROM device_tokens WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn delete_token(pool: &PgPool, token: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM device_tokens WHERE token = $1")
        .bind(token)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn update_fs_policy(
    pool: &PgPool,
    user_id: &str,
    device_name: &str,
    fs_policy: &serde_json::Value,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE device_tokens SET fs_policy = $1 WHERE user_id = $2 AND device_name = $3",
    )
    .bind(fs_policy)
    .bind(user_id)
    .bind(device_name)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn update_mcp_config(
    pool: &PgPool,
    user_id: &str,
    device_name: &str,
    mcp_config: &serde_json::Value,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE device_tokens SET mcp_config = $1 WHERE user_id = $2 AND device_name = $3",
    )
    .bind(mcp_config)
    .bind(user_id)
    .bind(device_name)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}
```

- [ ] **Step 4: Create db/system_config.rs**

```rust
use sqlx::PgPool;

pub async fn get(pool: &PgPool, key: &str) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM system_config WHERE key = $1")
            .bind(key)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.0))
}

pub async fn set(pool: &PgPool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO system_config (key, value, updated_at) VALUES ($1, $2, NOW())
         ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = NOW()",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 5: Update db/mod.rs exports**

Add to the top of `db/mod.rs`:

```rust
pub mod devices;
pub mod messages;
pub mod sessions;
pub mod system_config;
pub mod users;
```

- [ ] **Step 6: Build**

Run: `cargo build -p nexus-server`
Expected: Compiles

- [ ] **Step 7: Commit**

```bash
git add nexus-server/src/db/
git commit -m "feat(server): sessions, messages, devices, system_config CRUD modules"
```

---

### Task 5: Auth — JWT + Register + Login

**Files:**
- Create: `nexus-server/src/auth/mod.rs`
- Modify: `nexus-server/src/main.rs`

- [ ] **Step 1: Create auth/mod.rs**

```rust
pub mod device;

use crate::state::AppState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router, middleware};
use axum::routing::{get, post};
use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use nexus_common::consts::{BCRYPT_COST, JWT_EXPIRY_DAYS};
use nexus_common::error::{ApiError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,       // user_id
    pub is_admin: bool,
    pub exp: i64,
}

pub fn sign_jwt(user_id: &str, is_admin: bool, secret: &str) -> String {
    let exp = Utc::now().timestamp() + JWT_EXPIRY_DAYS * 86400;
    let claims = Claims {
        sub: user_id.to_string(),
        is_admin,
        exp,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("JWT encoding failed")
}

pub fn verify_jwt(token: &str, secret: &str) -> Result<Claims, ApiError> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map(|data| data.claims)
    .map_err(|e| ApiError::new(ErrorCode::AuthFailed, format!("Invalid token: {e}")))
}

/// Extract Claims from Authorization header. Returns 401 on failure.
pub fn extract_claims(headers: &HeaderMap, secret: &str) -> Result<Claims, ApiError> {
    let header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::new(ErrorCode::Unauthorized, "Missing Authorization header"))?;
    let token = header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::new(ErrorCode::Unauthorized, "Invalid Authorization format"))?;
    verify_jwt(token, secret)
}

// -- Register / Login handlers --

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub admin_token: Option<String>,
}

#[derive(Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user_id: String,
    pub is_admin: bool,
}

pub async fn register(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    let is_admin = req
        .admin_token
        .as_deref()
        .map(|t| t == state.config.admin_token)
        .unwrap_or(false);

    let password_hash =
        bcrypt::hash(&req.password, BCRYPT_COST).map_err(|e| {
            ApiError::new(ErrorCode::InternalError, format!("Hash error: {e}"))
        })?;

    let user_id = uuid::Uuid::new_v4().to_string();

    crate::db::users::create_user(&state.db, &user_id, &req.email, &password_hash, is_admin)
        .await
        .map_err(|e| {
            if e.to_string().contains("duplicate") || e.to_string().contains("unique") {
                ApiError::new(ErrorCode::Conflict, "Email already registered")
            } else {
                ApiError::new(ErrorCode::InternalError, format!("DB error: {e}"))
            }
        })?;

    let token = sign_jwt(&user_id, is_admin, &state.config.jwt_secret);
    info!("User registered: {}", req.email);

    Ok(Json(AuthResponse {
        token,
        user_id,
        is_admin,
    }))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    let user = crate::db::users::find_by_email(&state.db, &req.email)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("DB error: {e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::AuthFailed, "Invalid credentials"))?;

    let valid = bcrypt::verify(&req.password, &user.password_hash).unwrap_or(false);
    if !valid {
        return Err(ApiError::new(ErrorCode::AuthFailed, "Invalid credentials"));
    }

    let token = sign_jwt(&user.user_id, user.is_admin, &state.config.jwt_secret);

    Ok(Json(AuthResponse {
        token,
        user_id: user.user_id,
        is_admin: user.is_admin,
    }))
}

pub fn auth_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
}
```

- [ ] **Step 2: Create auth/device.rs (stub for now)**

```rust
//! Device token API endpoints. Implemented in Task 7.
```

- [ ] **Step 3: Update main.rs with axum server + auth routes**

```rust
mod auth;
mod config;
mod db;
mod session;
mod state;

use crate::state::AppState;
use config::ServerConfig;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = ServerConfig::from_env();
    let pool = db::init_db(&config.database_url).await;

    let (outbound_tx, _outbound_rx) = mpsc::channel(1000);

    let state = Arc::new(AppState {
        db: pool,
        config: config.clone(),
        llm_config: Arc::new(RwLock::new(None)),
        devices: Default::default(),
        devices_by_user: Default::default(),
        pending: Default::default(),
        tool_schema_cache: Default::default(),
        rate_limiter: Default::default(),
        rate_limit_config: Arc::new(RwLock::new(0)),
        default_soul: Arc::new(RwLock::new(None)),
        sessions: Default::default(),
        web_fetch_semaphore: Arc::new(Semaphore::new(
            nexus_common::consts::WEB_FETCH_CONCURRENT_MAX,
        )),
        outbound_tx,
        shutdown: CancellationToken::new(),
    });

    let app = axum::Router::new()
        .merge(auth::auth_routes())
        .with_state(Arc::clone(&state));

    let addr = format!("0.0.0.0:{}", config.server_port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    info!("NEXUS Server listening on {addr}");
    axum::serve(listener, app).await.unwrap();
}
```

- [ ] **Step 4: Build and test**

Run: `cargo build -p nexus-server`
Expected: Compiles

Start server, then test register + login:

```bash
# Terminal 1: start server
DATABASE_URL=postgres://nexus:nexus@localhost:5432/nexus ADMIN_TOKEN=test JWT_SECRET=testsecret32charsminimumrequired SERVER_PORT=8080 NEXUS_GATEWAY_WS_URL=ws://localhost:9090/ws/nexus NEXUS_GATEWAY_TOKEN=test cargo run -p nexus-server

# Terminal 2: test
curl -s -X POST http://localhost:8080/api/auth/register -H 'Content-Type: application/json' -d '{"email":"test@test.com","password":"pass123","admin_token":"test"}'
# Expected: {"token":"eyJ...","user_id":"...","is_admin":true}

curl -s -X POST http://localhost:8080/api/auth/login -H 'Content-Type: application/json' -d '{"email":"test@test.com","password":"pass123"}'
# Expected: {"token":"eyJ...","user_id":"...","is_admin":true}
```

- [ ] **Step 5: Commit**

```bash
git add nexus-server/src/auth/ nexus-server/src/main.rs
git commit -m "feat(server): auth module — register, login, JWT sign/verify"
```

---

### Task 6: User + Session API Endpoints

**Files:**
- Create: `nexus-server/src/api.rs`
- Modify: `nexus-server/src/main.rs`

- [ ] **Step 1: Create api.rs**

```rust
//! User, session, and file endpoints. All require JWT.

use crate::auth::{extract_claims, Claims};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{delete, get, patch};
use axum::{Json, Router};
use nexus_common::consts::MEMORY_TEXT_MAX_CHARS;
use nexus_common::error::{ApiError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

fn claims(headers: &HeaderMap, state: &AppState) -> Result<Claims, ApiError> {
    extract_claims(headers, &state.config.jwt_secret)
}

// -- User Profile --

#[derive(Serialize)]
struct ProfileResponse {
    user_id: String,
    email: String,
    is_admin: bool,
    created_at: String,
}

async fn get_profile(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<ProfileResponse>, ApiError> {
    let c = claims(&headers, &state)?;
    let user = crate::db::users::find_by_id(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "User not found"))?;
    Ok(Json(ProfileResponse {
        user_id: user.user_id,
        email: user.email,
        is_admin: user.is_admin,
        created_at: user.created_at.to_rfc3339(),
    }))
}

// -- Soul --

async fn get_soul(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let user = crate::db::users::find_by_id(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "User not found"))?;
    Ok(Json(serde_json::json!({ "soul": user.soul })))
}

#[derive(Deserialize)]
struct SoulUpdate {
    soul: String,
}

async fn patch_soul(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<SoulUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    crate::db::users::update_soul(&state.db, &c.sub, Some(&req.soul))
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(serde_json::json!({ "message": "Soul updated" })))
}

// -- Memory --

async fn get_memory(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let user = crate::db::users::find_by_id(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "User not found"))?;
    Ok(Json(serde_json::json!({ "memory": user.memory_text })))
}

#[derive(Deserialize)]
struct MemoryUpdate {
    memory: String,
}

async fn patch_memory(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<MemoryUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    if req.memory.len() > MEMORY_TEXT_MAX_CHARS {
        return Err(ApiError::new(
            ErrorCode::ValidationFailed,
            format!("Memory exceeds {MEMORY_TEXT_MAX_CHARS} characters"),
        ));
    }
    crate::db::users::update_memory(&state.db, &c.sub, &req.memory)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(serde_json::json!({ "message": "Memory updated" })))
}

// -- Sessions --

async fn list_sessions(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<crate::db::sessions::Session>>, ApiError> {
    let c = claims(&headers, &state)?;
    let sessions = crate::db::sessions::list_by_user(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(sessions))
}

async fn delete_session(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    // Verify ownership
    let session = crate::db::sessions::find_by_id(&state.db, &session_id)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Session not found"))?;
    if session.user_id != c.sub {
        return Err(ApiError::new(ErrorCode::Forbidden, "Not your session"));
    }
    crate::db::sessions::delete_session(&state.db, &session_id)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    state.sessions.remove(&session_id);
    Ok(Json(serde_json::json!({ "message": "Session deleted" })))
}

#[derive(Deserialize)]
struct MessageQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn get_messages(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(q): Query<MessageQuery>,
) -> Result<Json<Vec<crate::db::messages::Message>>, ApiError> {
    let c = claims(&headers, &state)?;
    // Verify ownership
    let session = crate::db::sessions::find_by_id(&state.db, &session_id)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Session not found"))?;
    if session.user_id != c.sub {
        return Err(ApiError::new(ErrorCode::Forbidden, "Not your session"));
    }
    let limit = q.limit.unwrap_or(50).min(500);
    let offset = q.offset.unwrap_or(0);
    let msgs = crate::db::messages::list_paginated(&state.db, &session_id, limit, offset)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(msgs))
}

pub fn api_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/user/profile", get(get_profile))
        .route("/api/user/soul", get(get_soul).patch(patch_soul))
        .route("/api/user/memory", get(get_memory).patch(patch_memory))
        .route("/api/sessions", get(list_sessions))
        .route(
            "/api/sessions/{session_id}",
            delete(delete_session),
        )
        .route(
            "/api/sessions/{session_id}/messages",
            get(get_messages),
        )
}
```

- [ ] **Step 2: Merge api_routes into main.rs**

In `main.rs`, add `mod api;` and merge routes:

```rust
let app = axum::Router::new()
    .merge(auth::auth_routes())
    .merge(api::api_routes())
    .with_state(Arc::clone(&state));
```

- [ ] **Step 3: Build and test**

Run: `cargo build -p nexus-server`
Expected: Compiles

- [ ] **Step 4: Commit**

```bash
git add nexus-server/src/api.rs nexus-server/src/main.rs
git commit -m "feat(server): user profile, soul, memory, session API endpoints"
```

---

### Task 7: Device Token API + Policy Push

**Files:**
- Modify: `nexus-server/src/auth/device.rs`
- Modify: `nexus-server/src/main.rs`

- [ ] **Step 1: Implement auth/device.rs**

```rust
//! Device token CRUD + policy management endpoints.

use crate::auth::extract_claims;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use nexus_common::consts::DEVICE_TOKEN_PREFIX;
use nexus_common::error::{ApiError, ErrorCode};
use nexus_common::protocol::{FsPolicy, McpServerEntry, ServerToClient};
use serde::Deserialize;
use std::sync::Arc;

fn claims(headers: &HeaderMap, state: &AppState) -> Result<crate::auth::Claims, ApiError> {
    extract_claims(headers, &state.config.jwt_secret)
}

// -- Token CRUD --

#[derive(Deserialize)]
struct CreateTokenRequest {
    device_name: String,
}

async fn create_token(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateTokenRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let token = format!(
        "{}{}",
        DEVICE_TOKEN_PREFIX,
        uuid::Uuid::new_v4().simple().to_string()
    );
    crate::db::devices::create_token(&state.db, &token, &c.sub, &req.device_name)
        .await
        .map_err(|e| {
            if e.to_string().contains("unique") || e.to_string().contains("duplicate") {
                ApiError::new(ErrorCode::Conflict, "Device name already exists")
            } else {
                ApiError::new(ErrorCode::InternalError, format!("{e}"))
            }
        })?;
    Ok(Json(serde_json::json!({
        "token": token,
        "device_name": req.device_name,
    })))
}

async fn list_tokens(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<crate::db::devices::DeviceToken>>, ApiError> {
    let c = claims(&headers, &state)?;
    let tokens = crate::db::devices::list_by_user(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(tokens))
}

async fn delete_token(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    // Verify ownership
    let dt = crate::db::devices::find_by_token(&state.db, &token)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Token not found"))?;
    if dt.user_id != c.sub {
        return Err(ApiError::new(ErrorCode::Forbidden, "Not your token"));
    }
    crate::db::devices::delete_token(&state.db, &token)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    // Clean up in-memory state
    let key = AppState::device_key(&c.sub, &dt.device_name);
    state.devices.remove(&key);
    if let Some(mut keys) = state.devices_by_user.get_mut(&c.sub) {
        keys.retain(|k| k != &key);
    }
    state.tool_schema_cache.remove(&c.sub);
    Ok(Json(serde_json::json!({ "message": "Token deleted" })))
}

// -- Device Status --

#[derive(serde::Serialize)]
struct DeviceStatus {
    device_name: String,
    status: String,
    tools_count: usize,
    fs_policy: serde_json::Value,
}

async fn list_devices(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<DeviceStatus>>, ApiError> {
    let c = claims(&headers, &state)?;
    let tokens = crate::db::devices::list_by_user(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    let mut devices = Vec::new();
    for dt in tokens {
        let key = AppState::device_key(&c.sub, &dt.device_name);
        let (status, tools_count) = if let Some(conn) = state.devices.get(&key) {
            ("online".to_string(), conn.tools.len())
        } else {
            ("offline".to_string(), 0)
        };
        devices.push(DeviceStatus {
            device_name: dt.device_name,
            status,
            tools_count,
            fs_policy: dt.fs_policy,
        });
    }
    Ok(Json(devices))
}

// -- Policy --

async fn get_policy(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(device_name): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let tokens = crate::db::devices::list_by_user(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    let dt = tokens
        .into_iter()
        .find(|t| t.device_name == device_name)
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Device not found"))?;
    Ok(Json(serde_json::json!({
        "device_name": dt.device_name,
        "fs_policy": dt.fs_policy,
    })))
}

#[derive(Deserialize)]
struct PolicyUpdate {
    fs_policy: serde_json::Value,
}

async fn patch_policy(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(device_name): Path<String>,
    Json(req): Json<PolicyUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let updated =
        crate::db::devices::update_fs_policy(&state.db, &c.sub, &device_name, &req.fs_policy)
            .await
            .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    if !updated {
        return Err(ApiError::new(ErrorCode::NotFound, "Device not found"));
    }
    // Push ConfigUpdate to connected client
    let key = AppState::device_key(&c.sub, &device_name);
    if let Some(conn) = state.devices.get(&key) {
        let fs_policy: FsPolicy =
            serde_json::from_value(req.fs_policy.clone()).unwrap_or_default();
        let msg = ServerToClient::ConfigUpdate {
            fs_policy: Some(fs_policy),
            mcp_servers: None,
            workspace_path: None,
            shell_timeout: None,
            ssrf_whitelist: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let mut sink = conn.sink.lock().await;
        let _ = futures_util::SinkExt::send(
            &mut *sink,
            axum::extract::ws::Message::Text(json.into()),
        )
        .await;
    }
    Ok(Json(serde_json::json!({
        "device_name": device_name,
        "fs_policy": req.fs_policy,
    })))
}

// -- MCP Config --

async fn get_mcp(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(device_name): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let tokens = crate::db::devices::list_by_user(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    let dt = tokens
        .into_iter()
        .find(|t| t.device_name == device_name)
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Device not found"))?;
    Ok(Json(serde_json::json!({
        "device_name": dt.device_name,
        "mcp_servers": dt.mcp_config,
    })))
}

#[derive(Deserialize)]
struct McpUpdate {
    mcp_servers: serde_json::Value,
}

async fn put_mcp(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(device_name): Path<String>,
    Json(req): Json<McpUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let updated =
        crate::db::devices::update_mcp_config(&state.db, &c.sub, &device_name, &req.mcp_servers)
            .await
            .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    if !updated {
        return Err(ApiError::new(ErrorCode::NotFound, "Device not found"));
    }
    // Push ConfigUpdate to connected client
    let key = AppState::device_key(&c.sub, &device_name);
    if let Some(conn) = state.devices.get(&key) {
        let mcp_servers: Vec<McpServerEntry> =
            serde_json::from_value(req.mcp_servers.clone()).unwrap_or_default();
        let msg = ServerToClient::ConfigUpdate {
            fs_policy: None,
            mcp_servers: Some(mcp_servers),
            workspace_path: None,
            shell_timeout: None,
            ssrf_whitelist: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let mut sink = conn.sink.lock().await;
        let _ = futures_util::SinkExt::send(
            &mut *sink,
            axum::extract::ws::Message::Text(json.into()),
        )
        .await;
    }
    Ok(Json(serde_json::json!({
        "device_name": device_name,
        "mcp_servers": req.mcp_servers,
    })))
}

pub fn device_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/device-tokens", post(create_token).get(list_tokens))
        .route("/api/device-tokens/{token}", delete(delete_token))
        .route("/api/devices", get(list_devices))
        .route(
            "/api/devices/{device_name}/policy",
            get(get_policy).patch(patch_policy),
        )
        .route(
            "/api/devices/{device_name}/mcp",
            get(get_mcp).put(put_mcp),
        )
}
```

- [ ] **Step 2: Merge device_routes into main.rs**

Add to `main.rs` app builder:

```rust
let app = axum::Router::new()
    .merge(auth::auth_routes())
    .merge(auth::device::device_routes())
    .merge(api::api_routes())
    .with_state(Arc::clone(&state));
```

- [ ] **Step 3: Build**

Run: `cargo build -p nexus-server`
Expected: Compiles

- [ ] **Step 4: Commit**

```bash
git add nexus-server/src/auth/device.rs nexus-server/src/main.rs
git commit -m "feat(server): device token CRUD + policy/MCP endpoints with ConfigUpdate push"
```

---

### Task 8: File Store

**Files:**
- Create: `nexus-server/src/file_store.rs`
- Modify: `nexus-server/src/api.rs`
- Modify: `nexus-server/src/main.rs`

- [ ] **Step 1: Create file_store.rs**

```rust
//! File upload/download with user-isolated paths. Hourly cleanup of old files.

use nexus_common::consts::{FILE_CLEANUP_AGE_HOURS, FILE_UPLOAD_MAX_BYTES};
use nexus_common::error::{ApiError, ErrorCode};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{info, warn};

const UPLOAD_BASE: &str = "/tmp/nexus-uploads";

pub fn user_upload_dir(user_id: &str) -> PathBuf {
    PathBuf::from(UPLOAD_BASE).join(user_id)
}

pub async fn save_upload(
    user_id: &str,
    filename: &str,
    data: &[u8],
) -> Result<String, ApiError> {
    if data.len() > FILE_UPLOAD_MAX_BYTES {
        return Err(ApiError::new(
            ErrorCode::ValidationFailed,
            format!("File exceeds {}MB limit", FILE_UPLOAD_MAX_BYTES / 1024 / 1024),
        ));
    }
    let file_id = uuid::Uuid::new_v4().to_string();
    let dir = user_upload_dir(user_id);
    fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("mkdir: {e}")))?;

    let safe_name = sanitize_filename(filename);
    let path = dir.join(format!("{file_id}_{safe_name}"));
    fs::write(&path, data)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("write: {e}")))?;
    Ok(file_id)
}

pub async fn load_file(user_id: &str, file_id: &str) -> Result<(Vec<u8>, String), ApiError> {
    // Validate file_id (no path traversal)
    if file_id.contains("..") || file_id.contains('/') || file_id.contains('\\') {
        return Err(ApiError::new(ErrorCode::ValidationFailed, "Invalid file ID"));
    }
    let dir = user_upload_dir(user_id);
    let mut entries = fs::read_dir(&dir)
        .await
        .map_err(|_| ApiError::new(ErrorCode::NotFound, "No files found"))?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(file_id) {
            let data = fs::read(entry.path())
                .await
                .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("read: {e}")))?;
            let original_name = name
                .strip_prefix(&format!("{file_id}_"))
                .unwrap_or(&name)
                .to_string();
            return Ok((data, original_name));
        }
    }
    Err(ApiError::new(ErrorCode::NotFound, "File not found"))
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// Spawn background cleanup task. Deletes files older than FILE_CLEANUP_AGE_HOURS.
pub fn spawn_cleanup_task() {
    tokio::spawn(async {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            if let Err(e) = cleanup_old_files().await {
                warn!("File cleanup error: {e}");
            }
        }
    });
}

async fn cleanup_old_files() -> Result<(), std::io::Error> {
    let base = Path::new(UPLOAD_BASE);
    if !base.exists() {
        return Ok(());
    }
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(FILE_CLEANUP_AGE_HOURS * 3600);
    let mut count = 0u32;
    let mut dirs = fs::read_dir(base).await?;
    while let Some(user_dir) = dirs.next_entry().await? {
        if !user_dir.file_type().await?.is_dir() {
            continue;
        }
        let mut files = fs::read_dir(user_dir.path()).await?;
        while let Some(file) = files.next_entry().await? {
            if let Ok(meta) = file.metadata().await {
                if let Ok(modified) = meta.modified() {
                    if modified < cutoff {
                        let _ = fs::remove_file(file.path()).await;
                        count += 1;
                    }
                }
            }
        }
    }
    if count > 0 {
        info!("Cleaned up {count} old files");
    }
    Ok(())
}
```

- [ ] **Step 2: Add file upload/download endpoints to api.rs**

Add to `api.rs`:

```rust
use axum::body::Body;
use axum::http::Response;
use axum::extract::Multipart;

async fn upload_file(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    while let Ok(Some(field)) = multipart.next_field().await {
        let filename = field
            .file_name()
            .unwrap_or("upload")
            .to_string();
        let data = field
            .bytes()
            .await
            .map_err(|e| ApiError::new(ErrorCode::ValidationFailed, format!("read: {e}")))?;
        let file_id = crate::file_store::save_upload(&c.sub, &filename, &data).await?;
        return Ok(Json(serde_json::json!({
            "file_id": file_id,
            "file_name": filename,
        })));
    }
    Err(ApiError::new(ErrorCode::ValidationFailed, "No file provided"))
}

async fn download_file(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(file_id): Path<String>,
) -> Result<Response<Body>, ApiError> {
    let c = claims(&headers, &state)?;
    let (data, filename) = crate::file_store::load_file(&c.sub, &file_id).await?;
    let mime = nexus_common::mime::detect_mime_from_extension(&filename)
        .unwrap_or("application/octet-stream");
    Ok(Response::builder()
        .header("Content-Type", mime)
        .header(
            "Content-Disposition",
            format!("attachment; filename=\"{filename}\""),
        )
        .header("X-Content-Type-Options", "nosniff")
        .body(Body::from(data))
        .unwrap())
}
```

Add file routes to `api_routes()`:

```rust
.route("/api/files", post(upload_file))
.route("/api/files/{file_id}", get(download_file))
```

- [ ] **Step 3: Add file_store module and spawn cleanup in main.rs**

Add `mod file_store;` to main.rs and call `file_store::spawn_cleanup_task();` after creating state.

- [ ] **Step 4: Build**

Run: `cargo build -p nexus-server`
Expected: Compiles

- [ ] **Step 5: Commit**

```bash
git add nexus-server/src/file_store.rs nexus-server/src/api.rs nexus-server/src/main.rs
git commit -m "feat(server): file store — upload, download, hourly cleanup"
```

---

### Task 9: WebSocket Handler — Device Login + Heartbeat + Tool Routing

**Files:**
- Create: `nexus-server/src/ws.rs`
- Create: `nexus-server/src/bus.rs` (stub for OutboundEvent type)
- Modify: `nexus-server/src/main.rs`

- [ ] **Step 1: Create bus.rs stub (just types for now)**

```rust
//! Message bus types. Full implementation in M2b.

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct InboundEvent {
    pub session_id: String,
    pub user_id: String,
    pub content: String,
    pub channel: String,
    pub chat_id: Option<String>,
    pub sender_id: Option<String>,
    pub media: Vec<String>,
    pub cron_job_id: Option<String>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct OutboundEvent {
    pub channel: String,
    pub chat_id: Option<String>,
    pub session_id: String,
    pub user_id: String,
    pub content: String,
    pub media: Vec<String>,
    pub is_progress: bool,
    pub metadata: HashMap<String, String>,
}
```

- [ ] **Step 2: Create ws.rs**

```rust
//! Client WebSocket handler: device login, heartbeat, tool registration, tool results.

use crate::state::{AppState, DeviceConnection};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use nexus_common::consts::{HEARTBEAT_REAPER_INTERVAL_SEC, PROTOCOL_VERSION};
use nexus_common::protocol::{
    ClientToServer, FsPolicy, McpServerEntry, ServerToClient, ToolExecutionResult,
};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_connection(socket, state))
}

async fn handle_connection(socket: WebSocket, state: Arc<AppState>) {
    let (mut sink, mut stream) = socket.split();

    // Send RequireLogin
    let msg = ServerToClient::RequireLogin {
        message: "NEXUS Server v1.0".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    if sink.send(Message::Text(json.into())).await.is_err() {
        return;
    }

    // Await SubmitToken
    let (user_id, device_name, device_token) = match stream.next().await {
        Some(Ok(Message::Text(text))) => {
            match serde_json::from_str::<ClientToServer>(&text) {
                Ok(ClientToServer::SubmitToken {
                    token,
                    protocol_version,
                }) => {
                    if protocol_version != PROTOCOL_VERSION {
                        let fail = ServerToClient::LoginFailed {
                            reason: format!(
                                "Protocol mismatch: expected {PROTOCOL_VERSION}, got {protocol_version}"
                            ),
                        };
                        let _ = sink
                            .send(Message::Text(serde_json::to_string(&fail).unwrap().into()))
                            .await;
                        return;
                    }
                    match crate::db::devices::find_by_token(&state.db, &token).await {
                        Ok(Some(dt)) => (dt.user_id.clone(), dt.device_name.clone(), dt),
                        _ => {
                            let fail = ServerToClient::LoginFailed {
                                reason: "Invalid token".into(),
                            };
                            let _ = sink
                                .send(Message::Text(
                                    serde_json::to_string(&fail).unwrap().into(),
                                ))
                                .await;
                            return;
                        }
                    }
                }
                _ => return,
            }
        }
        _ => return,
    };

    // Send LoginSuccess
    let fs_policy: FsPolicy =
        serde_json::from_value(device_token.fs_policy.clone()).unwrap_or_default();
    let mcp_servers: Vec<McpServerEntry> =
        serde_json::from_value(device_token.mcp_config.clone()).unwrap_or_default();
    let ssrf_whitelist: Vec<String> =
        serde_json::from_value(device_token.ssrf_whitelist.clone()).unwrap_or_default();

    let success = ServerToClient::LoginSuccess {
        user_id: user_id.clone(),
        device_name: device_name.clone(),
        fs_policy,
        mcp_servers,
        workspace_path: device_token.workspace_path.clone(),
        shell_timeout: device_token.shell_timeout as u64,
        ssrf_whitelist,
    };
    let json = serde_json::to_string(&success).unwrap();
    if sink.send(Message::Text(json.into())).await.is_err() {
        return;
    }

    info!("Device connected: {user_id}:{device_name}");

    // Register device in state
    let device_key = AppState::device_key(&user_id, &device_name);
    let sink = Arc::new(Mutex::new(sink));
    let last_seen = Arc::new(AtomicI64::new(chrono::Utc::now().timestamp()));

    state.devices.insert(
        device_key.clone(),
        DeviceConnection {
            user_id: user_id.clone(),
            device_name: device_name.clone(),
            sink: Arc::clone(&sink),
            last_seen: Arc::clone(&last_seen),
            tools: Vec::new(),
        },
    );
    state
        .devices_by_user
        .entry(user_id.clone())
        .or_default()
        .push(device_key.clone());

    // Create pending map for this device
    state
        .pending
        .entry(device_key.clone())
        .or_default();

    // Message loop
    while let Some(Ok(msg)) = stream.next().await {
        let Message::Text(text) = msg else {
            continue;
        };
        let Ok(client_msg) = serde_json::from_str::<ClientToServer>(&text) else {
            warn!("Bad message from {device_key}");
            continue;
        };
        last_seen.store(chrono::Utc::now().timestamp(), Ordering::SeqCst);

        match client_msg {
            ClientToServer::Heartbeat { .. } => {
                let ack = ServerToClient::HeartbeatAck;
                let json = serde_json::to_string(&ack).unwrap();
                let mut s = sink.lock().await;
                let _ = s.send(Message::Text(json.into())).await;
            }
            ClientToServer::RegisterTools { schemas } => {
                if let Some(mut conn) = state.devices.get_mut(&device_key) {
                    conn.tools = schemas;
                }
                state.tool_schema_cache.remove(&user_id);
                info!("Tools registered for {device_key}");
            }
            ClientToServer::ToolExecutionResult(result) => {
                resolve_pending(&state, &device_key, result);
            }
            ClientToServer::FileResponse {
                request_id,
                content_base64,
                mime_type,
                error,
            } => {
                // Resolve file request pending (reuse tool result channel)
                let result = ToolExecutionResult {
                    request_id: request_id.clone(),
                    exit_code: if error.is_some() { 1 } else { 0 },
                    output: if let Some(e) = error {
                        e
                    } else {
                        // Pack base64 + mime as JSON for the caller
                        serde_json::json!({
                            "content_base64": content_base64,
                            "mime_type": mime_type,
                        })
                        .to_string()
                    },
                };
                resolve_pending(&state, &device_key, result);
            }
            ClientToServer::FileSendAck { request_id, error } => {
                let result = ToolExecutionResult {
                    request_id: request_id.clone(),
                    exit_code: if error.is_some() { 1 } else { 0 },
                    output: error.unwrap_or_else(|| "ok".into()),
                };
                resolve_pending(&state, &device_key, result);
            }
            _ => {}
        }
    }

    // Disconnect cleanup
    info!("Device disconnected: {device_key}");
    state.devices.remove(&device_key);
    if let Some(mut keys) = state.devices_by_user.get_mut(&user_id) {
        keys.retain(|k| k != &device_key);
    }
    // Drop all pending oneshots (unblocks waiting agent loops)
    state.pending.remove(&device_key);
    state.tool_schema_cache.remove(&user_id);
}

fn resolve_pending(state: &AppState, device_key: &str, result: ToolExecutionResult) {
    if let Some(device_pending) = state.pending.get(device_key) {
        if let Some((_, sender)) = device_pending.remove(&result.request_id) {
            let _ = sender.send(result);
        }
    }
}

/// Spawn heartbeat reaper task. Checks every 30s for stale devices.
pub fn spawn_heartbeat_reaper(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(
            HEARTBEAT_REAPER_INTERVAL_SEC,
        ));
        loop {
            interval.tick().await;
            let now = chrono::Utc::now().timestamp();
            let timeout = nexus_common::consts::HEARTBEAT_INTERVAL_SEC as i64 * 4; // 60s
            let mut stale = Vec::new();
            for entry in state.devices.iter() {
                let last = entry.value().last_seen.load(Ordering::SeqCst);
                if now - last > timeout {
                    stale.push(entry.key().clone());
                }
            }
            for key in stale {
                warn!("Reaping stale device: {key}");
                if let Some((_, conn)) = state.devices.remove(&key) {
                    if let Some(mut keys) = state.devices_by_user.get_mut(&conn.user_id) {
                        keys.retain(|k| k != &key);
                    }
                    state.pending.remove(&key);
                    state.tool_schema_cache.remove(&conn.user_id);
                }
            }
        }
    });
}
```

- [ ] **Step 3: Wire WebSocket + reaper into main.rs**

Add `mod bus;` and `mod ws;` to main.rs. Add the ws route and spawn the reaper:

```rust
use axum::routing::get;

// After creating state:
file_store::spawn_cleanup_task();
ws::spawn_heartbeat_reaper(Arc::clone(&state));

let app = axum::Router::new()
    .merge(auth::auth_routes())
    .merge(auth::device::device_routes())
    .merge(api::api_routes())
    .route("/ws", get(ws::ws_handler))
    .with_state(Arc::clone(&state));
```

- [ ] **Step 4: Build**

Run: `cargo build -p nexus-server`
Expected: Compiles

- [ ] **Step 5: Commit**

```bash
git add nexus-server/src/ws.rs nexus-server/src/bus.rs nexus-server/src/main.rs
git commit -m "feat(server): WebSocket handler — device login, heartbeat, tool routing, reaper"
```

---

### Task 10: Integration Test — Full M2a Smoke Test

- [ ] **Step 1: Reset database**

```bash
psql -U nexus -d nexus -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public;"
```

- [ ] **Step 2: Start server**

```bash
DATABASE_URL=postgres://nexus:nexus@localhost:5432/nexus \
ADMIN_TOKEN=test \
JWT_SECRET=testsecret32charsminimumrequired \
SERVER_PORT=8080 \
NEXUS_GATEWAY_WS_URL=ws://localhost:9090/ws/nexus \
NEXUS_GATEWAY_TOKEN=test \
cargo run -p nexus-server
```

- [ ] **Step 3: Test auth flow**

```bash
# Register admin
curl -s -X POST http://localhost:8080/api/auth/register \
  -H 'Content-Type: application/json' \
  -d '{"email":"admin@test.com","password":"pass","admin_token":"test"}' | jq .

# Login
TOKEN=$(curl -s -X POST http://localhost:8080/api/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"email":"admin@test.com","password":"pass"}' | jq -r .token)

echo "JWT: $TOKEN"
```

- [ ] **Step 4: Test user endpoints**

```bash
# Profile
curl -s http://localhost:8080/api/user/profile -H "Authorization: Bearer $TOKEN" | jq .

# Soul
curl -s -X PATCH http://localhost:8080/api/user/soul \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"soul":"You are a helpful assistant"}' | jq .

# Memory
curl -s -X PATCH http://localhost:8080/api/user/memory \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"memory":"User prefers dark mode"}' | jq .
```

- [ ] **Step 5: Test device endpoints**

```bash
# Create device token
DEVICE=$(curl -s -X POST http://localhost:8080/api/device-tokens \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"device_name":"test-laptop"}' | jq -r .token)

echo "Device token: $DEVICE"

# List devices
curl -s http://localhost:8080/api/devices -H "Authorization: Bearer $TOKEN" | jq .
```

- [ ] **Step 6: Test WebSocket connection with nexus-client**

```bash
NEXUS_SERVER_WS_URL=ws://localhost:8080/ws \
NEXUS_AUTH_TOKEN=$DEVICE \
cargo run -p nexus-client
```

Expected: Client connects, logs "Login success", starts heartbeat. Server logs "Device connected" and "Tools registered".

- [ ] **Step 7: Commit (if any fixes needed)**

```bash
git add -u
git commit -m "fix(server): M2a integration test fixes"
```

---

## Summary

| Task | What | Files |
|---|---|---|
| 1 | nexus-common protocol + constants | 2 files modified |
| 2 | Crate scaffold + config + state | 6 files created |
| 3 | DB init + users CRUD | 2 files created, 1 modified |
| 4 | Sessions, messages, devices, system_config CRUD | 4 files created, 1 modified |
| 5 | Auth — JWT + register + login | 2 files created, 1 modified |
| 6 | User + session API endpoints | 1 file created, 1 modified |
| 7 | Device token API + policy push | 1 file modified, 1 modified |
| 8 | File store — upload, download, cleanup | 1 file created, 2 modified |
| 9 | WebSocket handler + heartbeat reaper | 2 files created, 1 modified |
| 10 | Integration smoke test | Manual testing |
