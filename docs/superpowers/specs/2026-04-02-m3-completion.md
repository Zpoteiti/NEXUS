# M3 Completion Spec — Everything Before Gateway

**Date:** 2026-04-02
**Goal:** Finish all server + client work so M4 can focus solely on building nexus-gateway without touching server code.

**Scope:** Message persistence, client robustness, context compression, memory/soul system, Discord media, agent decision streaming, API completion, config cleanup.

---

## Priority & Execution Order

| Phase | Tasks | Theme |
|-------|-------|-------|
| **Phase 1** | TASK-1, TASK-2 | Fix broken fundamentals |
| **Phase 2** | TASK-3, TASK-4 | Memory, soul, context compression |
| **Phase 3** | TASK-5, TASK-6 | Discord media + agent decision streaming |
| **Phase 4** | TASK-7, TASK-8 | Config cleanup + API completion |
| **Phase 5** | Compile + test | Verification |

---

## TASK-1: Message Persistence (assistant tool_calls not saved to DB)

### Problem

`agent_loop.rs` saves user messages and assistant text replies, but **assistant tool_calls messages are never written to DB**. The `messages` table lacks `tool_name` and `tool_arguments` columns. When session history is reconstructed, LLM sees orphaned tool results without the preceding assistant tool_calls — violating the OpenAI API contract.

### Fix

**1. Extend messages table schema (`db.rs` → `init_db`)**

```sql
ALTER TABLE messages ADD COLUMN IF NOT EXISTS tool_name TEXT;
ALTER TABLE messages ADD COLUMN IF NOT EXISTS tool_arguments TEXT;
```

**2. Extend `save_message` signature (`db.rs`)**

```rust
pub async fn save_message(
    db: &PgPool,
    session_id: &str,
    role: &str,
    content: &str,
    tool_call_id: Option<&str>,
    tool_name: Option<&str>,
    tool_arguments: Option<&str>,
) -> Result<String, sqlx::Error>
```

Update the INSERT to include `tool_name` and `tool_arguments`.

**3. Save tool_calls in `execute_tool_calls_loop` (`agent_loop.rs`)**

After building the assistant tool_calls message for `current_messages`, also persist:

```rust
for tc in &current_tool_calls {
    let _ = db::save_message(
        &state.db, session_id, "assistant", "",
        Some(&tc.id), Some(&tc.name), Some(&tc.arguments.to_string()),
    ).await;
}
```

Save tool results with tool_call_id:

```rust
let _ = db::save_message(
    &state.db, session_id, "tool", &content,
    Some(&tc.id), None, None,
).await;
```

**4. Update all existing `save_message` call sites**

Add `None, None` for the two new params on user and assistant text message saves.

**5. Fix `get_session_history` reconstruction (`db.rs`)**

Extend the SELECT to include `tool_name` and `tool_arguments`. Reconstruct proper format:

```rust
if role == "assistant" && tool_name.is_some() {
    // Reconstruct tool_calls array format
    json!({
        "role": "assistant",
        "tool_calls": [{
            "id": tool_call_id,
            "type": "function",
            "function": {
                "name": tool_name,
                "arguments": tool_arguments
            }
        }]
    })
} else if role == "tool" {
    json!({
        "role": "tool",
        "tool_call_id": tool_call_id,
        "content": content
    })
} else {
    json!({ "role": role, "content": content })
}
```

**Files:** `nexus-server/src/db.rs`, `nexus-server/src/agent_loop.rs`

---

## TASK-2: Client Tool Execution Robustness

### Problem

`sanitize_path()` calls `Path::canonicalize()` — a synchronous blocking syscall in an async context. On Windows or slow filesystems, this blocks the tokio executor. FS tools have no timeout protection.

### Fix

**1. `env.rs` — async wrapper with `spawn_blocking`**

```rust
pub async fn sanitize_path_async(raw: &str, restrict: bool) -> Result<PathBuf, String> {
    let raw = raw.to_string();
    let restrict = restrict;
    tokio::task::spawn_blocking(move || sanitize_path(&raw, restrict))
        .await
        .unwrap_or_else(|_| Err("path resolution panicked".to_string()))
}
```

Keep the synchronous `sanitize_path` for tests.

**2. `fs.rs` — async path resolution + per-tool timeout**

```rust
async fn resolve_path_async(path: &str) -> Result<PathBuf, ToolError> {
    env::sanitize_path_async(path, true).await
        .map_err(|e| ToolError::InvalidParams(e))
}
```

Wrap each tool's `execute` with timeout:

```rust
match tokio::time::timeout(Duration::from_secs(30), self.execute_inner(arguments)).await {
    Ok(result) => result,
    Err(_) => ToolResult { exit_code: -1, output: "tool execution timed out after 30s".into() },
}
```

**3. `executor.rs` — top-level 120s timeout**

```rust
pub async fn execute_tool_request(req: ExecuteToolRequest) -> ToolExecutionResult {
    match tokio::time::timeout(
        Duration::from_secs(120),
        execute_inner(&req),
    ).await {
        Ok(result) => result,
        Err(_) => ToolExecutionResult {
            request_id: req.request_id,
            exit_code: -1,
            output: "execution timed out after 120s".into(),
        },
    }
}
```

**4. Fix MCP tool discovery — `init_mcp()` on startup**

Currently `discover_mcp_tools_internal()` returns empty vec and `init_mcp()` is never called during startup. Fix:

- Call `init_mcp()` during client startup (after WebSocket connection established)
- After MCP servers connect, discover tools and include them in the `RegisterTools` message to server
- If MCP server connection fails, log warning but don't block startup

**Files:** `nexus-client/src/env.rs`, `nexus-client/src/tools/fs.rs`, `nexus-client/src/executor.rs`, `nexus-client/src/mcp_client.rs`, `nexus-client/src/main.rs`

---

## TASK-3: Memory, Soul & Preferences System

### Design

Per-user cross-session memory and personality. Reference: `nanobot/agent/memory.py`.

- **Soul**: LLM-generated agent persona per user. Shapes how agent communicates with that specific user.
- **Preferences**: User-specific settings (language, verbosity, etc).
- **Memory**: Long-term facts consolidated from conversations via LLM summarization.
- **History entries**: Timestamped summaries for searchability.

Admin can set a **default soul** (system-wide). Admin **cannot** read individual users' soul/memory (privacy).

### 3.1 DB Schema Extensions (`db.rs` → `init_db`)

**Extend users table:**

```sql
ALTER TABLE users ADD COLUMN IF NOT EXISTS soul TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS preferences JSONB;
```

**Default soul table (admin-managed):**

```sql
CREATE TABLE IF NOT EXISTS system_config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT NOW()
);
```

Store default soul as `key = 'default_soul'`.

**Memory chunks table (requires pgvector):**

```sql
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS memory_chunks (
    id SERIAL PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    user_id TEXT NOT NULL REFERENCES users(user_id),
    history_entry TEXT NOT NULL,
    memory_text TEXT NOT NULL,
    embedding vector,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
```

`vector` type without dimensions — actual dimensions determined at insert time, configurable via admin API.

### 3.2 Config Extensions (`config.rs`)

**Extend LlmConfig:**

```rust
pub struct LlmConfig {
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    pub context_window: usize,       // default 204800
    pub max_output_tokens: usize,    // default 131072
}
```

**New EmbeddingConfig:**

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EmbeddingConfig {
    pub api_base: String,     // "http://localhost:xxxx/v1"
    pub api_key: String,      // can be empty for local models
    pub model: String,        // "Qwen3-Embedding-8B"
    pub dimensions: usize,    // 1024
}
```

**ServerConfig:**

```rust
pub struct ServerConfig {
    // ... existing fields ...
    pub llm: Arc<RwLock<Option<LlmConfig>>>,           // None on first boot
    pub embedding: Arc<RwLock<Option<EmbeddingConfig>>>, // None until configured
}
```

Note: `llm` becomes `Option<LlmConfig>` — server starts without LLM config. Admin must configure via REST API. Agent loop checks `llm.is_some()` before processing.

### 3.3 DB Functions (`db.rs`)

```rust
// Soul & preferences
pub async fn get_user_soul(db: &PgPool, user_id: &str) -> Result<Option<String>, sqlx::Error>
pub async fn update_user_soul(db: &PgPool, user_id: &str, soul: &str) -> Result<(), sqlx::Error>
pub async fn get_user_preferences(db: &PgPool, user_id: &str) -> Result<Option<Value>, sqlx::Error>
pub async fn update_user_preferences(db: &PgPool, user_id: &str, prefs: &Value) -> Result<(), sqlx::Error>

// System config (default soul, etc)
pub async fn get_system_config(db: &PgPool, key: &str) -> Result<Option<String>, sqlx::Error>
pub async fn set_system_config(db: &PgPool, key: &str, value: &str) -> Result<(), sqlx::Error>

// Memory chunks
pub async fn save_memory_chunk(
    db: &PgPool, session_id: &str, user_id: &str,
    history_entry: &str, memory_text: &str, embedding: Option<&[f32]>,
) -> Result<(), sqlx::Error>

pub async fn get_latest_memory_text(db: &PgPool, session_id: &str) -> Result<Option<String>, sqlx::Error>

pub async fn vector_search_memory(
    db: &PgPool, user_id: &str, query_embedding: &[f32], top_k: usize,
) -> Result<Vec<MemoryChunk>, sqlx::Error>

pub async fn get_unconsolidated_messages(db: &PgPool, session_id: &str) -> Result<Vec<StoredMessage>, sqlx::Error>

pub async fn mark_messages_consolidated(db: &PgPool, message_ids: &[String]) -> Result<(), sqlx::Error>

pub async fn update_session_last_consolidated(db: &PgPool, session_id: &str, last_message_id: &str) -> Result<(), sqlx::Error>
```

### 3.4 Embedding (`context.rs`)

Replace stub with real implementation. Standard OpenAI `v1/embeddings` API:

```rust
pub async fn embed_text(config: &EmbeddingConfig, text: &str) -> Vec<f32> {
    // POST {config.api_base}/embeddings
    // Body: {"model": config.model, "input": text, "dimensions": config.dimensions}
    // Response: {"data": [{"embedding": [...]}]}
    // On failure: return empty Vec (skip embedding, don't block flow)
}
```

### 3.5 Context Compression (`memory.rs` — full implementation)

Reference: `nanobot/agent/memory.py`

**Token estimation:**

```rust
fn estimate_tokens(messages: &[Value]) -> usize {
    messages.iter()
        .map(|m| {
            let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let tool_args = m.get("tool_calls").map(|v| v.to_string().len()).unwrap_or(0);
            (content.len() + tool_args) / 3
        })
        .sum()
}
```

**Trigger condition:**

```rust
let budget = llm_config.context_window - llm_config.max_output_tokens - 1024; // safety buffer
let target = budget / 2;
if estimate_tokens(&all_messages) >= budget {
    // Loop consolidation rounds, max 5
}
```

**`pick_consolidation_boundary`:** Scan from start, find furthest `role=user` boundary where preceding messages have enough tokens to remove. Never cut between assistant/tool pairs.

**`save_memory` tool (built-in, not registered to client):**

```json
{
  "type": "function",
  "function": {
    "name": "save_memory",
    "description": "Save the memory consolidation result.",
    "parameters": {
      "type": "object",
      "properties": {
        "history_entry": {
          "type": "string",
          "description": "Timestamped summary: [YYYY-MM-DD HH:MM] key events and decisions."
        },
        "memory_update": {
          "type": "string",
          "description": "Full updated long-term memory as markdown. Merge new info with existing."
        }
      },
      "required": ["history_entry", "memory_update"]
    }
  }
}
```

**Consolidation flow:**

1. `get_unconsolidated_messages` → get messages to compress
2. `pick_consolidation_boundary` → determine cutoff
3. Format messages as `[timestamp] ROLE: content`
4. Get current `memory_text` via `get_latest_memory_text`
5. Call LLM with consolidation prompt + `save_memory` tool (tool_choice = forced, fallback to auto per nanobot pattern)
6. Parse LLM's `save_memory` call
7. Call `embed_text` → generate embedding
8. Save to `memory_chunks` table
9. `mark_messages_consolidated` + `update_session_last_consolidated`
10. On failure: increment counter, after 3 consecutive failures → raw archive fallback

**3-strike fallback:**

```rust
static FAILURE_COUNTS: LazyLock<DashMap<String, usize>> = LazyLock::new(DashMap::new);
const MAX_FAILURES: usize = 3;
```

On 3rd failure: dump raw messages as `[RAW]`-prefixed history entry, skip LLM, reset counter.

### 3.6 Soul/Preferences in System Prompt (`context.rs`)

Update `build_system_prompt` to inject:

- **Section 2**: User soul (from `db::get_user_soul`, fall back to `get_system_config("default_soul")`)
- **Section 4**: RAG memory injection (vector search on user input → format relevant chunks)

```rust
// Section 2 — Soul
let user_soul = db::get_user_soul(&state.db, user_id).await.ok().flatten();
let soul = user_soul.or_else(|| db::get_system_config(&state.db, "default_soul").await.ok().flatten());
if let Some(soul) = soul {
    sections.push(format!("## Personality\n{}", soul));
}

// Section 4 — RAG memory
let embedding_config = state.config.embedding.read().await.clone();
if let Some(ref emb_config) = embedding_config {
    let query_emb = embed_text(emb_config, user_input).await;
    if !query_emb.is_empty() {
        let chunks = db::vector_search_memory(&state.db, user_id, &query_emb, 5).await
            .unwrap_or_default();
        if !chunks.is_empty() {
            let memory_text = chunks.iter()
                .map(|c| c.memory_text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            sections.push(format!("## Relevant Memory\n{}", memory_text));
        }
    }
}
```

### 3.7 Agent Loop Integration (`agent_loop.rs`)

Call `maybe_consolidate` before LLM call in `run_single_turn`:

```rust
crate::memory::maybe_consolidate(
    session_id, &event.sender_id, &state.db, &llm_config, &state.config.embedding,
).await;
```

### 3.8 Cargo.toml

```toml
pgvector = "0.4"
```

**Files:** `nexus-server/src/config.rs`, `nexus-server/src/db.rs`, `nexus-server/src/context.rs`, `nexus-server/src/memory.rs`, `nexus-server/src/agent_loop.rs`, `nexus-server/Cargo.toml`

---

## TASK-4: Remove Hardcoded LLM Config

### Problem

`config.rs:63-67` has a real MiniMax API key hardcoded as default. Already in git history = compromised.

### Fix

1. **Remove hardcoded LLM config.** `ServerConfig.llm` becomes `Arc<RwLock<Option<LlmConfig>>>` initialized to `None`.
2. **Server starts without LLM.** Agent loop checks `state.config.llm.read().await` — if `None`, respond with "LLM not configured. Admin must configure via API."
3. **Admin configures via existing `PUT /api/llm-config`.** Update handler to create config if `None`.
4. **Persist LLM/Embedding config to DB** via `system_config` table so it survives restart:
   - `PUT /api/llm-config` → `set_system_config("llm_config", serde_json::to_string(&config))`
   - `load_config()` → after DB init, load from `get_system_config("llm_config")`

**Files:** `nexus-server/src/config.rs`, `nexus-server/src/agent_loop.rs`, `nexus-server/src/auth.rs`, `nexus-server/src/main.rs`

---

## TASK-5: Discord Media Support (Both Directions)

### Design

Reference: `nanobot/channels/discord.py` (lines 389-414 inbound, 183-211 outbound) and `nanobot/providers/base.py` (VLM fallback, lines 299-303).

### 5.1 Inbound: User sends media to Discord

**`discord/gateway_conn.rs` — extract attachments from MESSAGE_CREATE:**

```rust
// Parse attachments from Discord message payload
let attachments: Vec<DiscordAttachment> = msg.attachments.unwrap_or_default();
let mut media_paths = Vec::new();

for att in &attachments {
    if att.size > 20 * 1024 * 1024 {
        // Skip files > 20MB, add text marker
        content.push_str(&format!("\n[attachment: {} - too large]", att.filename));
        continue;
    }
    // Download to temp dir
    match download_attachment(&att.url, &att.filename).await {
        Ok(path) => media_paths.push(path),
        Err(e) => {
            warn!("Failed to download attachment {}: {}", att.filename, e);
            content.push_str(&format!("\n[attachment: {} - download failed]", att.filename));
        }
    }
}
```

**`discord/rest.rs` — new `download_attachment` function:**

```rust
pub async fn download_attachment(url: &str, filename: &str) -> Result<String, String> {
    // Download file to /tmp/nexus-media/{uuid}_{filename}
    // Return absolute path
}
```

**`InboundEvent.media`** — currently `Vec::new()`. Populate with downloaded file paths.

**`context.rs` — format media for LLM (OpenAI vision format):**

```rust
fn build_user_content(text: &str, media: &[String]) -> Value {
    if media.is_empty() {
        return json!(text);
    }
    let mut parts: Vec<Value> = Vec::new();
    for path in media {
        if let Some(mime) = detect_image_mime(path) {
            let data = std::fs::read(path).ok();
            if let Some(bytes) = data {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                parts.push(json!({
                    "type": "image_url",
                    "image_url": { "url": format!("data:{};base64,{}", mime, b64) }
                }));
            }
        }
        // Non-image files: add text marker
        else {
            parts.push(json!({
                "type": "text",
                "text": format!("[file: {}]", Path::new(path).file_name().unwrap_or_default().to_string_lossy())
            }));
        }
    }
    parts.push(json!({ "type": "text", "text": text }));
    json!(parts)
}
```

### 5.2 VLM Fallback

**`providers/mod.rs` — retry without images on non-transient error:**

Reference: `nanobot/providers/base.py:299-303` — `_strip_image_content()`.

```rust
pub async fn call_with_retry(
    config: &LlmConfig,
    request: ChatCompletionRequest,
) -> Result<ChatCompletionResponse, ProviderError> {
    match call_with_retry_inner(config, request.clone()).await {
        Ok(resp) => Ok(resp),
        Err(e) if !e.is_transient() && has_image_content(&request.messages) => {
            warn!("LLM error with image content, retrying without images: {}", e);
            let stripped = strip_image_content(request);
            call_with_retry_inner(config, stripped).await
        }
        Err(e) => Err(e),
    }
}

fn strip_image_content(mut request: ChatCompletionRequest) -> ChatCompletionRequest {
    // Replace image_url blocks with text placeholders: "[image omitted]"
    // Keep text blocks unchanged
    request
}

fn has_image_content(messages: &[Value]) -> bool {
    // Check if any message contains image_url type content
}
```

### 5.3 Outbound: Agent sends files to Discord

**`discord/rest.rs` — file upload via multipart:**

```rust
pub async fn send_message_with_files(
    bot_token: &str,
    channel_id: &str,
    content: &str,
    file_paths: &[String],
) -> Result<(), String> {
    // Use multipart/form-data with discord.File-style upload
    // Max 10 files, 25MB each per Discord limits
}
```

**`OutboundEvent.media`** — already defined as `Vec<String>`. Agent loop populates when tools produce files. Discord channel manager checks `media` and uses `send_message_with_files`.

**`discord/mod.rs` — outbound dispatch:**

```rust
async fn send(&self, event: OutboundEvent) {
    if event.media.is_empty() {
        rest::send_message(&token, &event.chat_id, &event.content).await;
    } else {
        rest::send_message_with_files(&token, &event.chat_id, &event.content, &event.media).await;
    }
}
```

**Files:** `nexus-server/src/channels/discord/gateway_conn.rs`, `nexus-server/src/channels/discord/rest.rs`, `nexus-server/src/channels/discord/mod.rs`, `nexus-server/src/context.rs`, `nexus-server/src/providers/mod.rs`

---

## TASK-6: Agent Decision Streaming to Discord

### Design

NOT token-level LLM streaming (`stream=true`). Instead: send real-time events about agent decisions through the existing message bus → Discord channel.

When the agent decides to call a tool, send a status message to Discord immediately (before waiting for tool execution). This keeps users informed during multi-step agent loops.

Reference: nanobot uses `_progress` metadata and `_tool_hint` flags via `AgentHook.before_execute_tools()`.

### 6.1 OutboundEvent Metadata Conventions

Use `OutboundEvent.metadata` (currently unused) for event type flags:

```rust
// Progress update (tool call intent)
metadata.insert("_progress".into(), json!(true));
metadata.insert("_tool_hint".into(), json!(true));

// Error notification
metadata.insert("_progress".into(), json!(true));
metadata.insert("_error".into(), json!(true));
```

### 6.2 Agent Loop — Emit Progress Events (`agent_loop.rs`)

In `execute_tool_calls_loop`, before routing each tool call:

```rust
for tc in &current_tool_calls {
    // Emit progress: "🔧 Calling tool_name on device_name..."
    let device_hint = /* resolve device name from tool registry */;
    let hint = format!("🔧 `{}` on {}", tc.name, device_hint);
    let mut metadata = HashMap::new();
    metadata.insert("_progress".into(), json!(true));
    metadata.insert("_tool_hint".into(), json!(true));
    state.bus.publish_outbound(OutboundEvent {
        channel: event.channel.clone(),
        chat_id: event.chat_id.clone(),
        content: hint,
        media: Vec::new(),
        metadata,
    }).await;

    // ... proceed with tool execution
}
```

Also emit on errors:

```rust
// After tool execution failure
let mut metadata = HashMap::new();
metadata.insert("_progress".into(), json!(true));
metadata.insert("_error".into(), json!(true));
state.bus.publish_outbound(OutboundEvent {
    channel: event.channel.clone(),
    chat_id: event.chat_id.clone(),
    content: format!("⚠️ Tool `{}` failed: {}", tc.name, error_msg),
    media: Vec::new(),
    metadata,
}).await;
```

### 6.3 Discord Channel — Handle Progress Events

**`discord/mod.rs` — dispatch based on metadata:**

```rust
async fn send(&self, event: OutboundEvent) {
    let is_progress = event.metadata.get("_progress").and_then(|v| v.as_bool()).unwrap_or(false);

    if is_progress {
        // Send as a lighter-weight message (no typing indicator reset)
        // Optionally format differently (embed, italic, etc.)
        rest::send_message(&token, &event.chat_id, &event.content).await;
    } else {
        // Normal response — stop typing, send full message
        self.cancel_typing(&event.chat_id).await;
        // ... normal send logic with media support
    }
}
```

### 6.4 Typing Indicator Integration

- **Start typing** when InboundEvent received (already implemented)
- **Keep typing** during tool execution (already implemented — 120s timeout)
- **Cancel typing** only when final response sent (not on progress updates)

**Files:** `nexus-server/src/agent_loop.rs`, `nexus-server/src/channels/discord/mod.rs`, `nexus-server/src/channels/discord/rest.rs`

---

## TASK-7: API Completion

### 7.1 Existing API TODOs (`api.rs`)

Implement all TODO endpoints. These are for the WebUI (Vue frontend in M4+) but the REST API must exist in M3.

**`GET /api/sessions/:id/messages`** — Return paginated messages for a session (owned by current user).

```rust
// db.rs
pub async fn get_session_messages(
    db: &PgPool, session_id: &str, user_id: &str, limit: i64, offset: i64,
) -> Result<Vec<MessageInfo>, sqlx::Error>

#[derive(sqlx::FromRow, Serialize)]
pub struct MessageInfo {
    pub message_id: String,
    pub role: String,
    pub content: String,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_arguments: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}
```

**`GET /api/devices`** — List currently connected devices for the authenticated user.

```rust
// Read from state.devices + state.devices_by_user (in-memory)
// Return: [{ device_name, device_key_masked, tools_count, last_seen }]
```

No DB needed — devices are ephemeral (connected clients).

**`GET /api/memories`** — List memory chunks for current user.

```rust
// db.rs
pub async fn list_memory_chunks(
    db: &PgPool, user_id: &str, limit: i64, offset: i64,
) -> Result<Vec<MemoryChunkInfo>, sqlx::Error>
```

**`GET /api/user/soul`** — Get current user's soul.
**`PATCH /api/user/soul`** — Update current user's soul.
**`GET /api/user/preferences`** — Get current user's preferences.
**`PATCH /api/user/preferences`** — Update current user's preferences.

```rust
// Handlers call db::get_user_soul, db::update_user_soul,
// db::get_user_preferences, db::update_user_preferences
```

**`GET /api/admin/default-soul`** — Admin: get default soul.
**`PUT /api/admin/default-soul`** — Admin: set default soul.

```rust
// Uses system_config table: key = "default_soul"
```

### 7.2 Embedding Config Admin API

**`GET /api/embedding-config`** — Admin only. Returns current embedding config (masked API key).
**`PUT /api/embedding-config`** — Admin only. Updates embedding config at runtime.

Same pattern as existing `GET/PUT /api/llm-config`.

### 7.3 Route Registration (`main.rs`)

```rust
let protected = Router::new()
    // ... existing routes ...
    // Sessions (existing + new)
    .route("/api/sessions/{session_id}/messages", get(api::get_session_messages))
    // Devices
    .route("/api/devices", get(api::list_devices))
    // Memories
    .route("/api/memories", get(api::list_memories))
    // User soul & preferences
    .route("/api/user/soul", get(api::get_soul).patch(api::update_soul))
    .route("/api/user/preferences", get(api::get_preferences).patch(api::update_preferences))
    // Admin
    .route("/api/admin/default-soul", get(api::get_default_soul).put(api::set_default_soul))
    .route("/api/embedding-config", get(auth::get_embedding_config).put(auth::update_embedding_config))
    .layer(middleware);
```

**Files:** `nexus-server/src/api.rs`, `nexus-server/src/auth.rs`, `nexus-server/src/db.rs`, `nexus-server/src/main.rs`

---

## TASK-8: Configuration & Auth Cleanup

### 8.1 LLM/Embedding Config Persistence

Configs must survive server restart. Store in `system_config` table:

- On `PUT /api/llm-config`: write to DB + update in-memory `Arc<RwLock<Option<LlmConfig>>>`
- On `PUT /api/embedding-config`: write to DB + update in-memory
- On server startup: after `init_db`, load from `system_config` table into `ServerConfig`

```rust
// main.rs — after init_db
if let Some(llm_json) = db::get_system_config(&pool, "llm_config").await? {
    let llm: LlmConfig = serde_json::from_str(&llm_json)?;
    *state_arc.config.llm.write().await = Some(llm);
}
if let Some(emb_json) = db::get_system_config(&pool, "embedding_config").await? {
    let emb: EmbeddingConfig = serde_json::from_str(&emb_json)?;
    *state_arc.config.embedding.write().await = Some(emb);
}
```

### 8.2 Remove Hardcoded API Key

Delete lines 63-67 in `config.rs`. Replace with:

```rust
llm: Arc::new(RwLock::new(None)),
embedding: Arc::new(RwLock::new(None)),
```

### 8.3 Agent Loop Guard for Missing LLM Config

```rust
// agent_loop.rs — run_single_turn
let llm_config = match state.config.llm.read().await.clone() {
    Some(config) => config,
    None => {
        state.bus.publish_outbound(make_outbound(&event,
            "⚠️ LLM not configured. An admin must set up the LLM provider via the API first.".into()
        )).await;
        return Ok("LLM not configured".into());
    }
};
```

**Files:** `nexus-server/src/config.rs`, `nexus-server/src/main.rs`, `nexus-server/src/agent_loop.rs`, `nexus-server/src/auth.rs`

---

## Deferred (NOT in M3)

| Item | Reason |
|------|--------|
| Gateway (nexus-gateway) | M4 |
| WebUI (Vue) | M4+ |
| Email verification, 2FA, password reset | Too complex for now |
| Device token expiration | Tokens are permanent by design; manual revocation is sufficient |
| Path traversal guardrails enhancement | Current guardrails.rs is sufficient |
| Process registry (process.rs) | Aspirational, not from nanobot, not blocking |
| Subagent/spawn system | Post-M4 |

---

## Execution Order

1. **TASK-1** — Message persistence (unblocks TASK-3 history reconstruction)
2. **TASK-4** — Remove hardcoded API key + config refactor (unblocks TASK-3 config extensions)
3. **TASK-2** — Client robustness (independent, can parallel with TASK-3)
4. **TASK-3** — Memory/soul/context compression (biggest task, depends on TASK-1 + TASK-4)
5. **TASK-5** — Discord media (independent of memory system)
6. **TASK-6** — Agent decision streaming (depends on working agent loop)
7. **TASK-7** — API completion (depends on TASK-3 for memory/soul DB functions)
8. **TASK-8** — Config persistence + cleanup (finalize)
9. **`cargo build` + `cargo clippy` + E2E test**
