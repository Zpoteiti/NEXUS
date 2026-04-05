# M3 Completion Features Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close all functional and non-functional gaps between NEXUS and nanobot before M4 (gateway/frontend only).

**Architecture:** Formalize server-native tools as a trait-based registry (replacing ad-hoc special cases in agent_loop.rs). Add server-side rmcp client for admin-shared MCP tools. Merge same-named tools across devices into unified schemas with multi-value device_name enums. Add cron scheduler, proactive messaging, edit file tool, concurrent execution, hooks, and checkpointing.

**Tech Stack:** Rust, rmcp 1.3, tokio, sqlx (PostgreSQL), cron (croniter equivalent), serde_json

**Key design decisions:**
- Server-only native tools (memory, cron, message): NO device_name param
- Server MCP tools (admin-added): device_name="server" in enum
- Same tool on server + device: merged schema with multi-value enum `["server", "xiaoshu"]`
- Cron jobs stored in DB, fire as inbound messages, get dedicated sessions `cron:{job_id}`
- MAX_AGENT_ITERATIONS = 200 (already done)

---

## File Structure

### New files
- `nexus-server/src/server_tools/mod.rs` — ServerTool trait + registry
- `nexus-server/src/server_tools/memory.rs` — save_memory (extracted from agent_loop.rs)
- `nexus-server/src/server_tools/message.rs` — proactive messaging tool
- `nexus-server/src/server_tools/cron.rs` — cron create/list/remove tools
- `nexus-server/src/server_tools/send_file.rs` — send_file (extracted from agent_loop.rs)
- `nexus-server/src/cron.rs` — cron scheduler service
- `nexus-server/src/server_mcp.rs` — server-side rmcp client manager
- `nexus-client/src/tools/edit.rs` — edit_file tool

### Modified files
- `nexus-server/src/main.rs` — register server MCP, start cron scheduler
- `nexus-server/src/agent_loop.rs` — extract server tools, add hooks, checkpointing, concurrent dispatch
- `nexus-server/src/context.rs` — unified tool schema building with merge logic
- `nexus-server/src/tools_registry.rs` — tool schema merging across devices + server
- `nexus-server/src/state.rs` — add server_tools, server_mcp, cron fields to AppState
- `nexus-server/src/db.rs` — add cron_jobs table, checkpoint table
- `nexus-server/src/bus.rs` — no changes needed (already supports what we need)
- `nexus-server/Cargo.toml` — add rmcp, cron dependencies
- `nexus-client/src/tools/mod.rs` — register EditFileTool
- `nexus-client/src/executor.rs` — concurrent tool execution

---

## Task 1: Edit File Tool (Client)

**Files:**
- Create: `nexus-client/src/tools/edit.rs`
- Modify: `nexus-client/src/tools/mod.rs`
- Modify: `nexus-client/src/executor.rs`

### Purpose
Add an `edit_file` tool that applies targeted line-based edits without rewriting entire files. Supports replacing a specific string occurrence with a new string (same pattern as Claude Code's Edit tool).

- [ ] **Step 1: Create edit tool implementation**

Create `nexus-client/src/tools/edit.rs`:

```rust
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;

use crate::env::{self, FsOp};
use crate::tools::{LocalTool, ToolError};
use nexus_common::protocol::FsPolicy;

pub struct EditFileTool;

impl EditFileTool {
    pub fn new() -> Self { EditFileTool }
}

impl Default for EditFileTool {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl LocalTool for EditFileTool {
    fn name(&self) -> &'static str { "edit_file" }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "edit_file",
                "description": "Apply a targeted edit to a file by replacing a specific string with a new string. More efficient than rewriting the entire file.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "Path to the file to edit."
                        },
                        "old_string": {
                            "type": "string",
                            "description": "The exact string to find and replace. Must be unique in the file."
                        },
                        "new_string": {
                            "type": "string",
                            "description": "The replacement string."
                        }
                    },
                    "required": ["file_path", "old_string", "new_string"]
                }
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String, ToolError> {
        self.execute_with_policy(args, &FsPolicy::Sandbox).await
    }
}

impl EditFileTool {
    pub async fn execute_with_policy(&self, args: Value, policy: &FsPolicy) -> Result<String, ToolError> {
        let file_path = args.get("file_path").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("missing file_path".into()))?;
        let old_string = args.get("old_string").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("missing old_string".into()))?;
        let new_string = args.get("new_string").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("missing new_string".into()))?;

        let resolved = env::sanitize_path_with_policy(file_path, FsOp::Write, policy)
            .map_err(|e| ToolError::Blocked(e))?;

        edit_file_core(&resolved, old_string, new_string).await
    }
}

async fn edit_file_core(path: &Path, old_string: &str, new_string: &str) -> Result<String, ToolError> {
    let content = tokio::fs::read_to_string(path).await
        .map_err(|e| ToolError::ExecutionFailed(format!("failed to read file: {}", e)))?;

    let count = content.matches(old_string).count();
    if count == 0 {
        return Err(ToolError::ExecutionFailed(
            "old_string not found in file".into()
        ));
    }
    if count > 1 {
        return Err(ToolError::ExecutionFailed(format!(
            "old_string found {} times — must be unique. Provide more surrounding context.", count
        )));
    }

    let new_content = content.replacen(old_string, new_string, 1);
    tokio::fs::write(path, &new_content).await
        .map_err(|e| ToolError::ExecutionFailed(format!("failed to write file: {}", e)))?;

    Ok(format!("Edited {}: replaced 1 occurrence ({} chars → {} chars)",
        path.display(), old_string.len(), new_string.len()))
}
```

- [ ] **Step 2: Register edit tool**

In `nexus-client/src/tools/mod.rs`, add:
```rust
pub mod edit;
```

In `nexus-client/src/executor.rs`, add `EditFileTool` to `FS_TOOLS` array and `LOCAL_TOOL_REGISTRY`, following the same pattern as `WriteFileTool`.

Add `"edit_file"` to the `FS_TOOLS` constant and add the tool to the registry:
```rust
use crate::tools::edit::EditFileTool;
// In FS_TOOLS:
static FS_TOOLS: &[&str] = &["read_file", "write_file", "edit_file", "list_dir", "stat"];
// In LOCAL_TOOL_REGISTRY:
registry.insert("edit_file", Box::new(EditFileTool::new()));
```

Add the `execute_with_policy` dispatch in `execute_fs_tool()`:
```rust
"edit_file" => EditFileTool::new().execute_with_policy(arguments, policy).await,
```

- [ ] **Step 3: Build and test**

```bash
cargo build --package nexus-client
cargo test --package nexus-client
```

- [ ] **Step 4: Commit**

```bash
git add nexus-client/src/tools/edit.rs nexus-client/src/tools/mod.rs nexus-client/src/executor.rs
git commit -m "feat(client): add edit_file tool for targeted string replacement"
```

---

## Task 2: Abstracted Server-Native Tools

**Files:**
- Create: `nexus-server/src/server_tools/mod.rs`
- Create: `nexus-server/src/server_tools/memory.rs`
- Create: `nexus-server/src/server_tools/send_file.rs`
- Modify: `nexus-server/src/main.rs` — add `mod server_tools`
- Modify: `nexus-server/src/agent_loop.rs` — replace inline save_memory/send_file with trait dispatch
- Modify: `nexus-server/src/context.rs` — collect server tool schemas from registry
- Modify: `nexus-server/src/state.rs` — add `server_tools` field

### Purpose
Extract save_memory and send_file from agent_loop.rs into a trait-based server tool registry. This makes adding new server-side tools (cron, message) clean and consistent.

- [ ] **Step 1: Create ServerTool trait and registry**

Create `nexus-server/src/server_tools/mod.rs`:

```rust
pub mod memory;
pub mod send_file;

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::state::AppState;

/// Result of executing a server-native tool.
pub struct ServerToolResult {
    pub output: String,
    /// Optional media file paths (e.g., from send_file)
    pub media: Vec<String>,
}

/// A tool that executes on the server, not on any client device.
/// Server-native tools do NOT have a device_name parameter.
#[async_trait]
pub trait ServerTool: Send + Sync {
    fn name(&self) -> &str;
    fn schema(&self) -> Value;
    async fn execute(
        &self,
        state: &Arc<AppState>,
        user_id: &str,
        session_id: &str,
        arguments: Value,
    ) -> Result<ServerToolResult, String>;
}

pub struct ServerToolRegistry {
    tools: HashMap<String, Box<dyn ServerTool>>,
}

impl ServerToolRegistry {
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    pub fn register(&mut self, tool: Box<dyn ServerTool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn ServerTool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn schemas(&self) -> Vec<Value> {
        self.tools.values().map(|t| t.schema()).collect()
    }

    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }
}
```

- [ ] **Step 2: Extract save_memory into ServerTool**

Create `nexus-server/src/server_tools/memory.rs` — move the save_memory logic from `agent_loop.rs:execute_single_tool()` lines 538–578 into this trait impl. Keep the same embedding + dedup logic.

```rust
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::info;

use super::{ServerTool, ServerToolResult};
use crate::state::AppState;

pub struct SaveMemoryTool;

#[async_trait]
impl ServerTool for SaveMemoryTool {
    fn name(&self) -> &str { "save_memory" }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "save_memory",
                "description": "Save an important fact, preference, or context to long-term memory for future reference.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "The memory content to save."
                        }
                    },
                    "required": ["content"]
                }
            }
        })
    }

    async fn execute(
        &self,
        state: &Arc<AppState>,
        user_id: &str,
        session_id: &str,
        arguments: Value,
    ) -> Result<ServerToolResult, String> {
        let content = arguments.get("content")
            .and_then(|v| v.as_str())
            .ok_or("missing content parameter")?
            .to_string();

        // Generate embedding if configured
        let embedding = {
            let emb_config = state.config.embedding.read().await;
            if let Some(ref config) = *emb_config {
                let config_clone = config.clone();
                drop(emb_config);
                let vec = crate::context::embed_text_throttled(
                    &config_clone, &content, &state.embedding_semaphore
                ).await;
                if vec.is_empty() { None } else { Some(vec) }
            } else {
                None
            }
        };

        // Dedup check
        if let Some(ref emb) = embedding {
            if let Ok(Some(_)) = crate::db::find_similar_memory(&state.db, user_id, emb, 0.92).await {
                info!("save_memory: skipping duplicate (cosine > 0.92)");
                return Ok(ServerToolResult {
                    output: "Memory already exists (duplicate detected).".into(),
                    media: vec![],
                });
            }
        }

        crate::db::save_memory_chunk(
            &state.db, session_id, user_id,
            "", &content, embedding.as_deref(),
        ).await.map_err(|e| format!("save_memory failed: {}", e))?;

        Ok(ServerToolResult {
            output: "Memory saved.".into(),
            media: vec![],
        })
    }
}
```

- [ ] **Step 3: Extract send_file into ServerTool**

Create `nexus-server/src/server_tools/send_file.rs` — move send_file logic from `agent_loop.rs:execute_single_tool()` lines 468–535. This tool DOES need device_name (it pulls a file from a device), so it's a server tool but with a device_name parameter in its schema. Handle it as a special case in the schema — it provides its own device_name enum.

- [ ] **Step 4: Register tools in AppState**

Add to `nexus-server/src/state.rs`:
```rust
use crate::server_tools::ServerToolRegistry;

// In AppState:
pub server_tools: Arc<ServerToolRegistry>,
```

In `AppState::new()`:
```rust
let mut server_tool_reg = ServerToolRegistry::new();
server_tool_reg.register(Box::new(crate::server_tools::memory::SaveMemoryTool));
server_tool_reg.register(Box::new(crate::server_tools::send_file::SendFileTool));
// ... more tools added in later tasks
```

- [ ] **Step 5: Update agent_loop.rs to use registry**

In `execute_single_tool()`, replace the inline save_memory and send_file blocks with:
```rust
// Check server-native tools first
if let Some(tool) = state.server_tools.get(tool_name) {
    let result = tool.execute(state, user_id, session_id, arguments).await
        .map_err(|e| format!("server tool error: {}", e))?;
    // Handle media from result
    return Ok((result.output, result.media));
}
// Otherwise route to device...
```

- [ ] **Step 6: Update context.rs to collect server tool schemas**

In `get_all_tools_schema()`, replace the hardcoded save_memory/send_file schemas with:
```rust
// Add server-native tool schemas (no device_name)
all_schemas.extend(state.server_tools.schemas());
```

- [ ] **Step 7: Build and test**

```bash
cargo build --package nexus-server
cargo test --package nexus-server
```

- [ ] **Step 8: Commit**

```bash
git add nexus-server/src/server_tools/ nexus-server/src/main.rs nexus-server/src/agent_loop.rs nexus-server/src/context.rs nexus-server/src/state.rs
git commit -m "refactor(server): extract server-native tools into trait-based registry"
```

---

## Task 3: Tool Schema Merging

**Files:**
- Modify: `nexus-server/src/tools_registry.rs`
- Modify: `nexus-server/src/context.rs`

### Purpose
When the same tool exists on multiple devices (or on server + device), merge into one schema with multi-value device_name enum instead of duplicate entries.

- [ ] **Step 1: Add merge logic to context.rs**

Replace the current per-device schema collection in `get_all_tools_schema()` with a merge step:

```rust
/// Merge tools with the same name across devices into unified schemas.
/// Each merged tool gets a device_name enum with all locations that have it.
fn merge_tool_schemas(
    device_tools: Vec<(String, Vec<Value>)>,  // (device_name, schemas)
    server_mcp_tools: Vec<Value>,             // server MCP tools (device_name="server")
) -> Vec<Value> {
    use std::collections::HashMap;

    // Group by tool function name → list of device names
    let mut tool_map: HashMap<String, (Value, Vec<String>)> = HashMap::new();

    for (device_name, schemas) in device_tools {
        for schema in schemas {
            let func_name = schema.pointer("/function/name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if func_name.is_empty() { continue; }

            tool_map.entry(func_name)
                .and_modify(|(_, devices)| devices.push(device_name.clone()))
                .or_insert((schema, vec![device_name.clone()]));
        }
    }

    // Server MCP tools get device_name="server"
    for schema in server_mcp_tools {
        let func_name = schema.pointer("/function/name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if func_name.is_empty() { continue; }

        tool_map.entry(func_name)
            .and_modify(|(_, devices)| {
                if !devices.contains(&"server".to_string()) {
                    devices.push("server".to_string());
                }
            })
            .or_insert((schema, vec!["server".to_string()]));
    }

    // Build final schemas with device_name enum injected
    tool_map.into_values().map(|(schema, devices)| {
        inject_device_name_param(&schema, &devices)
    }).collect()
}
```

- [ ] **Step 2: Update get_all_tools_schema to use merge**

Collect per-device tools as `(device_name, schemas)` tuples, then call `merge_tool_schemas()`.

- [ ] **Step 3: Build and test**

```bash
cargo build --package nexus-server
```

- [ ] **Step 4: Commit**

```bash
git add nexus-server/src/tools_registry.rs nexus-server/src/context.rs
git commit -m "feat(server): merge same-named tools across devices into unified schema with multi-value device_name"
```

---

## Task 4: Server-Side rmcp Client

**Files:**
- Create: `nexus-server/src/server_mcp.rs`
- Modify: `nexus-server/Cargo.toml` — add rmcp dependency
- Modify: `nexus-server/src/main.rs` — add mod, init server MCP
- Modify: `nexus-server/src/state.rs` — add server_mcp field
- Modify: `nexus-server/src/db.rs` — add server_mcp_config to system_config
- Modify: `nexus-server/src/auth.rs` — add admin API for server MCP config
- Modify: `nexus-server/src/agent_loop.rs` — route server MCP tool calls
- Modify: `nexus-server/src/context.rs` — include server MCP schemas in merge

### Purpose
Admins can configure shared MCP servers on the NEXUS server. All users' agents automatically get these tools with device_name="server". Reuse the same rmcp pattern from nexus-client.

- [ ] **Step 1: Add rmcp to server dependencies**

In `nexus-server/Cargo.toml`:
```toml
rmcp = { version = "1", features = ["client", "transport-child-process"] }
```

- [ ] **Step 2: Create server MCP manager**

Create `nexus-server/src/server_mcp.rs` — similar to `nexus-client/src/mcp_client.rs` but:
- Lives on the server, managed by admin
- Tools appear with device_name="server"
- Uses `Arc<RwLock<ServerMcpManager>>` in AppState
- Config stored in `system_config` table as `server_mcp_config`
- `initialize()`, `list_all_tools()`, `call_tool()` methods

Key differences from client MCP:
- No `normalize_schema_for_openai()` needed (already applied at client level if relevant)
- Actually, server MCP tools also need normalization since they come from MCP servers
- Reuse the normalize logic

```rust
pub struct ServerMcpManager {
    sessions: HashMap<String, ServerMcpSession>,
}

pub struct ServerMcpSession {
    server_name: String,
    client: rmcp::service::RunningService<rmcp::RoleClient, ()>,
    tool_name_map: HashMap<String, String>,  // wrapped → original
    tool_timeout: u64,
}
```

Tool naming: `mcp_{server_name}_{tool_name}` (same convention as client).

- [ ] **Step 3: Add admin API endpoints**

In `nexus-server/src/auth.rs`, add:
- `GET /api/server-mcp` — list server MCP config
- `PUT /api/server-mcp` — update server MCP config (admin only)

Config format matches client MCP: `{ "mcp_servers": [...] }` with McpServerEntry.

In `nexus-server/src/main.rs`, register routes:
```rust
.route("/api/server-mcp", axum::routing::get(auth::get_server_mcp).put(auth::update_server_mcp))
```

- [ ] **Step 4: Initialize on startup, reinit on config change**

In `main.rs`, after loading system config:
```rust
if let Ok(Some(mcp_json)) = db::get_system_config(&pool, "server_mcp_config").await {
    if let Ok(entries) = serde_json::from_str::<Vec<McpServerEntry>>(&mcp_json) {
        state.server_mcp.write().await.initialize(&entries).await;
    }
}
```

When admin PUTs new config, reinitialize the manager (same pattern as embedding config).

- [ ] **Step 5: Integrate with tool schema merging**

In `context.rs:get_all_tools_schema()`, collect server MCP schemas:
```rust
let server_mcp = state.server_mcp.read().await;
let server_mcp_schemas = server_mcp.all_tool_schemas();
// Pass to merge_tool_schemas()
```

- [ ] **Step 6: Route server MCP calls in agent_loop**

In `execute_single_tool()`, after checking server-native tools and before routing to device:
```rust
// Check server MCP tools
if tool_name.starts_with("mcp_") && device_name == "server" {
    let manager = state.server_mcp.read().await;
    let result = manager.call_tool(tool_name, arguments).await
        .map_err(|e| format!("server MCP error: {}", e))?;
    return Ok((result, vec![]));
}
```

- [ ] **Step 7: Build and test**

```bash
cargo build --package nexus-server
```

- [ ] **Step 8: Commit**

```bash
git add nexus-server/src/server_mcp.rs nexus-server/Cargo.toml nexus-server/src/main.rs nexus-server/src/state.rs nexus-server/src/auth.rs nexus-server/src/agent_loop.rs nexus-server/src/context.rs nexus-server/src/db.rs
git commit -m "feat(server): add server-side rmcp client for admin-shared MCP tools"
```

---

## Task 5: Proactive Messaging Tool

**Files:**
- Create: `nexus-server/src/server_tools/message.rs`
- Modify: `nexus-server/src/state.rs` — register tool

### Purpose
Let the agent send messages to a user's channel proactively (not just as a response). Server-native tool (no device_name). Essential for cron job result delivery.

- [ ] **Step 1: Create message tool**

```rust
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use super::{ServerTool, ServerToolResult};
use crate::bus::OutboundEvent;
use crate::state::AppState;

pub struct MessageTool;

#[async_trait]
impl ServerTool for MessageTool {
    fn name(&self) -> &str { "message" }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "message",
                "description": "Send a message to a specific channel/chat proactively. Use this to deliver results, reminders, or notifications.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "channel": {
                            "type": "string",
                            "description": "The channel type (e.g., 'discord', 'telegram')."
                        },
                        "chat_id": {
                            "type": "string",
                            "description": "The target chat/channel ID."
                        },
                        "content": {
                            "type": "string",
                            "description": "The message content to send."
                        }
                    },
                    "required": ["channel", "chat_id", "content"]
                }
            }
        })
    }

    async fn execute(
        &self,
        state: &Arc<AppState>,
        _user_id: &str,
        _session_id: &str,
        arguments: Value,
    ) -> Result<ServerToolResult, String> {
        let channel = arguments.get("channel").and_then(|v| v.as_str())
            .ok_or("missing channel")?;
        let chat_id = arguments.get("chat_id").and_then(|v| v.as_str())
            .ok_or("missing chat_id")?;
        let content = arguments.get("content").and_then(|v| v.as_str())
            .ok_or("missing content")?;

        let event = OutboundEvent {
            channel: channel.to_string(),
            chat_id: chat_id.to_string(),
            content: content.to_string(),
            media: vec![],
            metadata: Default::default(),
        };

        state.bus.publish_outbound(event).await;

        Ok(ServerToolResult {
            output: format!("Message sent to {}:{}", channel, chat_id),
            media: vec![],
        })
    }
}
```

- [ ] **Step 2: Register in server_tools**

Add `pub mod message;` to `server_tools/mod.rs` and register in AppState:
```rust
server_tool_reg.register(Box::new(crate::server_tools::message::MessageTool));
```

- [ ] **Step 3: Build and commit**

```bash
cargo build --package nexus-server
git add nexus-server/src/server_tools/message.rs nexus-server/src/server_tools/mod.rs nexus-server/src/state.rs
git commit -m "feat(server): add proactive message tool for agent-initiated channel delivery"
```

---

## Task 6: Cron Tool + Scheduler + REST API

**Files:**
- Create: `nexus-server/src/server_tools/cron.rs` — cron_create, cron_list, cron_remove tools
- Create: `nexus-server/src/cron.rs` — scheduler service
- Modify: `nexus-server/src/db.rs` — add cron_jobs table
- Modify: `nexus-server/src/main.rs` — start scheduler, register cron routes
- Modify: `nexus-server/src/state.rs` — add cron handle
- Modify: `nexus-server/src/auth.rs` — add user-facing cron REST API
- Modify: `nexus-server/Cargo.toml` — add `cron` crate for expression parsing

### Purpose
3 agent tools (create, list, remove). Jobs stored in PostgreSQL. Scheduler fires jobs by injecting prompts as inbound messages through the bus. REST API lets users manage their cron jobs from the frontend (not just via agent).

- [ ] **Step 1: Add cron_jobs table**

In `nexus-server/src/db.rs`, add to `init_db()`:

```sql
CREATE TABLE IF NOT EXISTS cron_jobs (
    job_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(user_id),
    name TEXT NOT NULL,
    enabled BOOLEAN DEFAULT TRUE,
    cron_expr TEXT,
    every_seconds INTEGER,
    run_at TIMESTAMPTZ,
    timezone TEXT DEFAULT 'UTC',
    message TEXT NOT NULL,
    channel TEXT NOT NULL,
    chat_id TEXT NOT NULL,
    delete_after_run BOOLEAN DEFAULT FALSE,
    next_run_at TIMESTAMPTZ,
    last_run_at TIMESTAMPTZ,
    run_count INTEGER DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
```

Add DB helper functions:
- `create_cron_job()` — insert job, compute next_run_at
- `list_cron_jobs(user_id)` — list all jobs for user
- `delete_cron_job(user_id, job_id)` — delete job
- `get_due_cron_jobs()` — select WHERE enabled AND next_run_at <= NOW()
- `update_cron_job_after_run()` — update last_run_at, next_run_at, run_count

- [ ] **Step 2: Create cron scheduler service**

Create `nexus-server/src/cron.rs`:

```rust
use std::sync::Arc;
use tokio::time::{Duration, sleep};
use tracing::{info, warn};

use crate::bus::{InboundEvent, MessageBus};
use crate::state::AppState;

/// Runs the cron scheduler loop. Checks for due jobs every 10 seconds.
pub async fn run_cron_scheduler(state: Arc<AppState>) {
    let poll_interval = Duration::from_secs(10);

    loop {
        match crate::db::get_due_cron_jobs(&state.db).await {
            Ok(jobs) => {
                for job in jobs {
                    info!("cron: firing job '{}' ({})", job.name, job.job_id);

                    let session_id = format!("cron:{}", job.job_id);
                    let reminder = format!(
                        "[Scheduled Task] Timer finished.\n\n\
                         Task '{}' has been triggered.\n\
                         Scheduled instruction: {}",
                        job.name, job.message
                    );

                    let event = InboundEvent {
                        channel: job.channel.clone(),
                        sender_id: job.user_id.clone(),
                        chat_id: job.chat_id.clone(),
                        content: reminder,
                        session_id,
                        timestamp: Some(chrono::Utc::now()),
                        media: vec![],
                        metadata: {
                            let mut m = std::collections::HashMap::new();
                            m.insert("cron_job_id".into(), serde_json::json!(job.job_id));
                            m
                        },
                    };

                    state.bus.publish_inbound(event).await;

                    // Update job state
                    if let Err(e) = crate::db::update_cron_job_after_run(
                        &state.db, &job.job_id, job.delete_after_run
                    ).await {
                        warn!("cron: failed to update job after run: {}", e);
                    }
                }
            }
            Err(e) => {
                warn!("cron: failed to query due jobs: {}", e);
            }
        }

        sleep(poll_interval).await;
    }
}
```

- [ ] **Step 3: Create cron tools (create, list, remove)**

Create `nexus-server/src/server_tools/cron.rs` with three ServerTool implementations:

- `CronCreateTool` — parameters: `message`, `cron_expr` (optional), `every_seconds` (optional), `at` (optional), `timezone`, `channel`, `chat_id`
- `CronListTool` — no parameters, lists user's jobs
- `CronRemoveTool` — parameters: `job_id`

Prevent nested cron: check `metadata.cron_job_id` in the event. If present, reject cron_create with an error message.

- [ ] **Step 4: Register tools and start scheduler**

Register all three cron tools in server_tools registry.

In `main.rs`, spawn the scheduler:
```rust
let state_for_cron = state_arc.clone();
tokio::spawn(crate::cron::run_cron_scheduler(state_for_cron));
```

- [ ] **Step 5: Add cron expression parsing**

Add to `nexus-server/Cargo.toml`:
```toml
cron = "0.15"
```

Use it in `db::create_cron_job()` to compute `next_run_at` from cron expression.

- [ ] **Step 6: Add user-facing REST API for cron management**

In `nexus-server/src/auth.rs`, add JWT-protected endpoints:

- `GET /api/cron-jobs` — list all cron jobs for the authenticated user
- `POST /api/cron-jobs` — create a new cron job (same params as agent tool: message, cron_expr, every_seconds, at, timezone, channel, chat_id)
- `DELETE /api/cron-jobs/{job_id}` — delete a cron job (only if owned by user)
- `PATCH /api/cron-jobs/{job_id}` — update a cron job (enable/disable, change schedule or message)

```rust
/// GET /api/cron-jobs
pub async fn list_cron_jobs(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    match db::list_cron_jobs(&state.db, &claims.sub).await {
        Ok(jobs) => Json(jobs).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("failed to list cron jobs: {}", e)).into_response(),
    }
}

/// POST /api/cron-jobs
pub async fn create_cron_job(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Json(payload): Json<CreateCronJobRequest>,
) -> Response {
    // Validate: at least one of cron_expr, every_seconds, or at must be provided
    // Compute next_run_at
    // Insert into DB
    // Return created job
}

/// DELETE /api/cron-jobs/{job_id}
pub async fn delete_cron_job(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(job_id): Path<String>,
) -> Response {
    // Verify ownership (user_id matches)
    // Delete from DB
}

/// PATCH /api/cron-jobs/{job_id}
pub async fn update_cron_job(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(job_id): Path<String>,
    Json(payload): Json<UpdateCronJobRequest>,
) -> Response {
    // Verify ownership
    // Update fields (enabled, cron_expr, message, etc.)
    // Recompute next_run_at if schedule changed
}
```

In `nexus-server/src/main.rs`, register routes in the protected router:
```rust
// Cron jobs
.route("/api/cron-jobs", axum::routing::get(auth::list_cron_jobs).post(auth::create_cron_job))
.route("/api/cron-jobs/{job_id}", axum::routing::delete(auth::delete_cron_job).patch(auth::update_cron_job))
```

- [ ] **Step 7: Build and test**

```bash
cargo build --package nexus-server
```

- [ ] **Step 8: Commit**

```bash
git add nexus-server/src/cron.rs nexus-server/src/server_tools/cron.rs nexus-server/src/db.rs nexus-server/src/main.rs nexus-server/src/auth.rs nexus-server/Cargo.toml
git commit -m "feat(server): add cron scheduler with create/list/remove tools and REST API"
```

---

## Task 7: Hooks (Agent Loop Extension Points)

**Files:**
- Modify: `nexus-server/src/agent_loop.rs`

### Purpose
Add `before_iteration`, `after_iteration`, and `finalize_content` hooks to the agent loop. `finalize_content` strips `<think>` blocks from reasoning models.

- [ ] **Step 1: Define hook trait**

Add at the top of `agent_loop.rs`:

```rust
/// Agent loop hooks for extensibility.
struct AgentHooks;

impl AgentHooks {
    /// Called before each LLM call in the tool loop.
    fn before_iteration(iteration: u32, tool_count: usize) {
        tracing::debug!("agent hook: before_iteration #{}, {} pending tools", iteration, tool_count);
    }

    /// Called after each LLM response in the tool loop.
    fn after_iteration(iteration: u32, finish_reason: &str) {
        tracing::debug!("agent hook: after_iteration #{}, finish_reason={}", iteration, finish_reason);
    }

    /// Post-process LLM content before returning to user.
    /// Strips <think>...</think> blocks from reasoning models.
    fn finalize_content(content: &str) -> String {
        // Strip <think>...</think> blocks (reasoning model artifacts)
        let re = regex::Regex::new(r"(?s)<think>.*?</think>").unwrap();
        let cleaned = re.replace_all(content, "").to_string();
        cleaned.trim().to_string()
    }
}
```

- [ ] **Step 2: Wire hooks into execute_tool_calls_loop**

In `execute_tool_calls_loop()`:
- Call `AgentHooks::before_iteration()` at start of each loop iteration
- Call `AgentHooks::after_iteration()` after receiving LLM response
- Call `AgentHooks::finalize_content()` on the final reply before returning

In `run_single_turn()`:
- Call `AgentHooks::finalize_content()` on non-tool-call replies too

- [ ] **Step 3: Add regex dependency**

In `nexus-server/Cargo.toml`:
```toml
regex = "1"
```

- [ ] **Step 4: Build and test**

```bash
cargo build --package nexus-server
```

- [ ] **Step 5: Commit**

```bash
git add nexus-server/src/agent_loop.rs nexus-server/Cargo.toml
git commit -m "feat(server): add agent loop hooks with finalize_content for think-block stripping"
```

---

## Task 8: Concurrent Tool Execution

**Files:**
- Modify: `nexus-server/src/agent_loop.rs` — send all tool calls, await in parallel
- Modify: `nexus-server/src/tools_registry.rs` — ensure route_tool works concurrently

### Purpose
When LLM returns multiple tool_calls, execute them in parallel instead of serially. Server sends all requests, then awaits all responses concurrently.

- [ ] **Step 1: Parallelize tool execution in agent loop**

In `execute_tool_calls_loop()`, replace the sequential tool execution loop with parallel dispatch:

```rust
// Execute all tool calls concurrently
let mut futures = Vec::new();
for tc in &current_tool_calls {
    let state = state.clone();
    let user_id = user_id.to_string();
    let session_id = session_id.to_string();
    let channel = event_channel.to_string();
    let chat_id = event_chat_id.to_string();
    let tc_clone = tc.clone();

    futures.push(tokio::spawn(async move {
        let result = execute_single_tool(
            &state, &user_id, &session_id, &channel, &chat_id,
            &tc_clone.name, tc_clone.arguments.clone(), &tc_clone.id,
        ).await;
        (tc_clone, result)
    }));
}

// Await all results
let results = futures::future::join_all(futures).await;
for join_result in results {
    let (tc, result) = join_result.map_err(|e| format!("task join error: {}", e))?;
    // Process result, add to messages, handle media...
}
```

Note: `execute_single_tool` already uses oneshot channels for device communication, so concurrent calls to different devices are naturally parallel. Calls to the same device serialize at the WebSocket level.

- [ ] **Step 2: Add futures dependency**

In `nexus-server/Cargo.toml` (already has `futures-util`, may need full `futures`):
```toml
futures = "0.3"
```

- [ ] **Step 3: Ensure route_tool is concurrent-safe**

`route_tool` uses `DashMap` for pending (already done) and `mpsc::Sender` for WebSocket (cloneable). No changes needed — already concurrent-safe.

- [ ] **Step 4: Build and test**

```bash
cargo build --package nexus-server
```

- [ ] **Step 5: Commit**

```bash
git add nexus-server/src/agent_loop.rs nexus-server/Cargo.toml
git commit -m "feat(server): execute parallel tool calls concurrently via tokio::spawn + join_all"
```

---

## Task 9: Checkpointing

**Files:**
- Modify: `nexus-server/src/db.rs` — add checkpoints table
- Modify: `nexus-server/src/agent_loop.rs` — save/restore checkpoints

### Purpose
Save agent loop state after each tool batch for crash recovery. On server restart, resume in-flight loops.

- [ ] **Step 1: Add checkpoints table**

In `nexus-server/src/db.rs`, add to `init_db()`:

```sql
CREATE TABLE IF NOT EXISTS agent_checkpoints (
    session_id TEXT PRIMARY KEY REFERENCES sessions(session_id),
    user_id TEXT NOT NULL,
    messages JSONB NOT NULL,
    pending_tool_calls JSONB,
    iteration INTEGER DEFAULT 0,
    channel TEXT NOT NULL,
    chat_id TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);
```

Add DB helpers:
- `save_checkpoint(session_id, user_id, messages, tool_calls, iteration, channel, chat_id)`
- `load_checkpoint(session_id)` → Option
- `delete_checkpoint(session_id)`

- [ ] **Step 2: Save checkpoint after each tool batch**

In `execute_tool_calls_loop()`, after all tool results are collected and before calling LLM again:

```rust
// Save checkpoint
if let Err(e) = crate::db::save_checkpoint(
    &state.db, session_id, user_id,
    &current_messages, &current_tool_calls, iteration,
    event_channel, event_chat_id,
).await {
    tracing::warn!("failed to save checkpoint: {}", e);
}
```

On successful completion (reply returned), delete checkpoint:
```rust
let _ = crate::db::delete_checkpoint(&state.db, session_id).await;
```

- [ ] **Step 3: Resume checkpoints on startup**

In `main.rs`, after initializing AppState, check for orphaned checkpoints:

```rust
// Resume in-flight agent loops from checkpoints
if let Ok(checkpoints) = db::list_all_checkpoints(&pool).await {
    for cp in checkpoints {
        info!("Resuming agent loop for session {}", cp.session_id);
        let event = InboundEvent {
            channel: cp.channel,
            sender_id: cp.user_id,
            chat_id: cp.chat_id,
            content: "[System] Resuming interrupted agent task...".into(),
            session_id: cp.session_id,
            timestamp: Some(chrono::Utc::now()),
            media: vec![],
            metadata: {
                let mut m = std::collections::HashMap::new();
                m.insert("resume_checkpoint".into(), serde_json::json!(true));
                m
            },
        };
        state_arc.bus.publish_inbound(event).await;
    }
}
```

- [ ] **Step 4: Build and test**

```bash
cargo build --package nexus-server
```

- [ ] **Step 5: Commit**

```bash
git add nexus-server/src/db.rs nexus-server/src/agent_loop.rs nexus-server/src/main.rs
git commit -m "feat(server): add agent loop checkpointing for crash recovery"
```

---

## Dependency Order

```
Task 1 (edit_file)          ──── standalone, do first
Task 2 (server tool trait)  ──── foundation for 4,5,6
Task 3 (schema merge)       ──── needed for task 4
Task 4 (server rmcp)        ──── depends on 2,3
Task 5 (message tool)       ──── depends on 2
Task 6 (cron)               ──── depends on 2,5
Task 7 (hooks)              ──── independent of 2-6
Task 8 (concurrent exec)    ──── independent
Task 9 (checkpointing)      ──── should be last (touches agent_loop heavily)
```

Recommended execution order: 1 → 2 → 3 → 7 → 5 → 4 → 6 → 8 → 9
