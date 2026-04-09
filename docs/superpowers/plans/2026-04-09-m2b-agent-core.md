# M2b: Agent Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the agent brain: message bus, LLM provider, context builder, ReAct agent loop, all 8 server tools, tools registry, context compression, and large message conversion.

**Architecture:** Message bus routes InboundEvents to per-session agent loops. Each loop calls the LLM, dispatches tool calls (server-local or client-remote), persists results, and iterates until the LLM returns text. Context compression via LLM summarization when approaching token limits.

**Tech Stack:** reqwest (LLM + web_fetch + skill install), tokio mpsc/oneshot (bus + tool routing), regex + ipnet (SSRF), chrono-tz + cron (scheduling)

**Spec:** `docs/superpowers/specs/2026-04-09-m2-server-design.md` (sections 6, 9-15)

**Depends on:** M2a (DB, auth, API, WebSocket all working)

---

## File Map

| File | Responsibility |
|---|---|
| `nexus-server/src/bus.rs` | Expand: session routing, rate limiting, outbound dispatch |
| `nexus-server/src/providers/openai.rs` | OpenAI chat completions, retry, think-tag stripping |
| `nexus-server/src/context.rs` | Build full prompt: system + soul + memory + skills + devices + history |
| `nexus-server/src/tools_registry.rs` | Merge schemas, inject device_name enum, route tool calls to devices |
| `nexus-server/src/agent_loop.rs` | Per-session ReAct loop: LLM → tools → iterate |
| `nexus-server/src/memory.rs` | Context compression via LLM summarization |
| `nexus-server/src/server_tools/mod.rs` | Server tool registry and dispatch |
| `nexus-server/src/server_tools/memory.rs` | save_memory, edit_memory |
| `nexus-server/src/server_tools/message.rs` | message (with media + from_device) |
| `nexus-server/src/server_tools/file_transfer.rs` | file_transfer (cross-device relay) |
| `nexus-server/src/server_tools/cron_tool.rs` | cron (unified: add/list/remove) |
| `nexus-server/src/server_tools/skills.rs` | read_skill, install_skill |
| `nexus-server/src/server_tools/web_fetch.rs` | web_fetch (SSRF-protected) |
| `nexus-server/src/cron.rs` | Cron poller: 10s poll, inject due jobs into bus |
| `nexus-server/src/db/cron.rs` | Cron jobs DB CRUD |
| `nexus-server/src/db/skills.rs` | Skills DB CRUD |
| `nexus-server/src/auth/admin.rs` | Admin endpoints (LLM config, rate limit, default soul, server MCP) |
| `nexus-server/src/auth/cron_api.rs` | Cron job API endpoints |
| `nexus-server/src/auth/skills_api.rs` | Skills API endpoints |

---

### Task 1: LLM Provider

**Files:**
- Create: `nexus-server/src/providers/openai.rs`
- Create: `nexus-server/src/providers/mod.rs`
- Modify: `nexus-server/src/state.rs` (add reqwest::Client)
- Modify: `nexus-server/src/main.rs` (add mod, create client)

- [ ] **Step 1: Create providers/openai.rs**

The OpenAI-compatible chat completions provider. Handles:
- Request building (messages + tools + model)
- Response parsing (text vs tool_calls)
- `<think>` tag stripping for reasoning models
- Retry with exponential backoff (429, 5xx)
- Image stripping on non-transient errors

Key types:
```rust
// LLM message format
pub struct ChatMessage {
    pub role: String,                    // "system", "user", "assistant", "tool"
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
}

pub struct ToolCall {
    pub id: String,
    pub r#type: String,                  // "function"
    pub function: FunctionCall,
}

pub struct FunctionCall {
    pub name: String,
    pub arguments: String,              // JSON string
}

// LLM response
pub enum LlmResponse {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}
```

Implementation: single `call_llm(client, config, messages, tools) -> Result<LlmResponse>` function. Uses `reqwest::Client` passed in. Retries up to 3 times with 1s/2s/4s backoff for 429 and 5xx. Strips `<think>...</think>` from content via regex. On 4xx (not 429): strips image content blocks and retries once.

- [ ] **Step 2: Add reqwest::Client to AppState**

Add `pub http_client: reqwest::Client` to `AppState`. Create once at startup in main.rs.

Add `reqwest` to Cargo.toml dependencies (move from dev-deps):
```toml
reqwest = { version = "0.12", features = ["json"] }
```

- [ ] **Step 3: Build and verify**

Run: `cargo build -p nexus-server`

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(server): OpenAI-compatible LLM provider with retry + think-tag stripping"
```

---

### Task 2: Message Bus — Full Implementation

**Files:**
- Modify: `nexus-server/src/bus.rs`

- [ ] **Step 1: Implement bus routing and rate limiting**

Expand `bus.rs` with:
- `publish_inbound(state, event)`: check rate limit → find or create SessionHandle → send to inbox_tx. If new session, spawn agent loop.
- Rate limit check: load `rate_limit_config`, check user's bucket in `rate_limiter` DashMap. Cron events (with `cron_job_id`) bypass.
- Token bucket: on each message, check `(remaining, last_refill)`. If stale (>60s), refill to limit. Decrement. If 0, reject.

Keep `InboundEvent` and `OutboundEvent` structs as-is. Add `publish_inbound` function.

- [ ] **Step 2: Build and verify**

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(server): message bus with session routing and rate limiting"
```

---

### Task 3: Context Builder

**Files:**
- Create: `nexus-server/src/context.rs`

- [ ] **Step 1: Implement context building**

Build the full prompt for each LLM call per spec section 10:
1. System prompt (soul or default)
2. Memory section
3. Always-on skills (full content)
4. On-demand skills (name + description)
5. Device status (online/offline + tool list)
6. Runtime info (current time)
7. Sender identity (ChannelIdentity — channel-agnostic)
8. Message history (reconstruct from DB rows, merge tool_calls)
9. Current user message (with untrusted wrapper for non-owner)

Key function: `build_context(state, user, session_id, event, history, skills) -> (Vec<ChatMessage>, Vec<ToolSchema>)`

Also builds tool schemas: merge server tools (no device_name) + client tools (with device_name enum) + server MCP tools (with device_name including "server").

History reconstruction: consecutive assistant rows with `tool_name` → single assistant message with `tool_calls` array. Tool rows → tool message with `tool_call_id`.

`ChannelIdentity` struct with `build_system_section()` method.

Token estimation: `chars / 4`.

- [ ] **Step 2: Build and verify**

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(server): context builder — system prompt, history, tools, identity"
```

---

### Task 4: Tools Registry — Schema Merge + Device Routing

**Files:**
- Create: `nexus-server/src/tools_registry.rs`

- [ ] **Step 1: Implement tools registry**

Two main functions:
1. `build_tool_schemas(state, user_id) -> Vec<Value>`: Merge server tool schemas + device tool schemas + server MCP schemas. For client/MCP tools, inject `device_name` enum parameter with available devices. Cache per-user in `tool_schema_cache`.

2. `route_tool_call(state, user_id, tool_name, arguments) -> Result<ToolExecutionResult>`:
   - Check server tools first → dispatch locally
   - Parse `device_name` from arguments
   - If `"server"` → dispatch to server MCP
   - Else → find device → send `ExecuteToolRequest` via WS → await oneshot (120s timeout)

Device routing: create oneshot channel, insert into `pending[device_key][request_id]`, send request via device's WsSink, await receiver with timeout.

- [ ] **Step 2: Build and verify**

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(server): tools registry — schema merge, device_name injection, routing"
```

---

### Task 5: Server Tools — Memory + Web Fetch

**Files:**
- Create: `nexus-server/src/server_tools/mod.rs`
- Create: `nexus-server/src/server_tools/memory.rs`
- Create: `nexus-server/src/server_tools/web_fetch.rs`

- [ ] **Step 1: Create server_tools/mod.rs**

Server tool registry: maps tool names to handler functions. Each returns `(i32, String)` (exit_code, output).

Dispatch function: `execute_server_tool(state, user_id, tool_name, arguments, session_context) -> ToolExecutionResult`

Tool schemas for all 8 server tools (JSON Schema format).

- [ ] **Step 2: Create server_tools/memory.rs**

`save_memory`: replace `users.memory_text`, enforce 4K cap.
`edit_memory`: append/prepend/replace operations on memory_text.

- [ ] **Step 3: Create server_tools/web_fetch.rs**

SSRF protection (same blocked ranges as client), per-user whitelist from DB, global semaphore (50 concurrent), reqwest GET with 15s timeout / 10s connect / 5 redirects / 1MB max body, strip HTML tags, prepend untrusted content banner, truncate to 50K chars.

- [ ] **Step 4: Build and verify**

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(server): server tools — save/edit memory, web_fetch with SSRF protection"
```

---

### Task 6: Server Tools — Message + File Transfer

**Files:**
- Create: `nexus-server/src/server_tools/message.rs`
- Create: `nexus-server/src/server_tools/file_transfer.rs`

- [ ] **Step 1: Create server_tools/message.rs**

`message` tool: send content to a channel with optional media from a device.
- If media + from_device: send FileRequest to device, await FileResponse, save to server temp
- Publish OutboundEvent to target channel/chat_id

- [ ] **Step 2: Create server_tools/file_transfer.rs**

`file_transfer` tool: relay files between devices.
- from_device=client: FileRequest → FileResponse (base64)
- from_device="server": read from user's allowed dirs (uploads + skills), canonicalize + prefix check
- to_device=client: FileSend → FileSendAck
- to_device="server": save to user upload dir

- [ ] **Step 3: Build and verify**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(server): server tools — message (with media) + file_transfer (cross-device)"
```

---

### Task 7: Server Tools — Cron + Skills

**Files:**
- Create: `nexus-server/src/server_tools/cron_tool.rs`
- Create: `nexus-server/src/server_tools/skills.rs`
- Create: `nexus-server/src/db/cron.rs`
- Create: `nexus-server/src/db/skills.rs`
- Modify: `nexus-server/src/db/mod.rs` (add modules)

- [ ] **Step 1: Create db/cron.rs + db/skills.rs**

Cron CRUD: create_job, list_by_user, delete_job, update_after_run, find_due_jobs.
Skills CRUD: upsert_skill, list_by_user, find_by_name, delete_skill.

- [ ] **Step 2: Create server_tools/cron_tool.rs**

Unified cron tool: action=add/list/remove. Compute next_run_at for each scheduling mode. Validate timezone via chrono-tz. Nested prevention (check session_id starts with "cron:"). Channel/chat_id from session context.

Add `cron` and `chrono-tz` to Cargo.toml.

- [ ] **Step 3: Create server_tools/skills.rs**

`read_skill`: load SKILL.md from disk, append file_transfer hint if extra files exist.
`install_skill`: fetch from GitHub raw URL, parse YAML frontmatter, write to disk, upsert DB.

- [ ] **Step 4: Build and verify**

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(server): server tools — cron (add/list/remove) + skills (read/install)"
```

---

### Task 8: Agent Loop

**Files:**
- Create: `nexus-server/src/agent_loop.rs`

- [ ] **Step 1: Implement the ReAct agent loop**

`run_session(state, session_id, user_id, inbox_rx)`:
- Outer loop: await InboundEvent from inbox
- Acquire session lock
- Save user message to DB
- Inner loop (max 200 iterations):
  - Load uncompressed history
  - Build context (system prompt + history + tools)
  - Check compression threshold → compress if needed
  - Call LLM
  - If text → save to DB, publish OutboundEvent, break
  - If tool_calls → dedup check, save assistant message, execute each tool, save results, publish progress, continue
- Loop guards: max iterations, loop detection (3 identical = soft, 4th = hard)

Large message conversion: if user message > 4K chars, save full to file, inline first 4K + file reference.

- [ ] **Step 2: Build and verify**

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(server): ReAct agent loop — LLM calls, tool dispatch, iteration guards"
```

---

### Task 9: Context Compression

**Files:**
- Create: `nexus-server/src/memory.rs`

- [ ] **Step 1: Implement compression**

`check_and_compress(state, session_id, history, context_window) -> bool`:
- Estimate tokens: sum of all message chars / 4
- If context_window - total < 16K: compress
- Identify messages between system (index 0) and latest user message
- Send to LLM: "Summarize this conversation concisely, preserving key decisions, facts, and context." max_tokens=12K
- Mark compressed messages in DB
- Insert summary as assistant message
- Return true (caller reloads history)

Summary messages are normal assistant messages — get compressed again in future rounds.

- [ ] **Step 2: Build and verify**

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(server): context compression via LLM summarization"
```

---

### Task 10: Cron Poller

**Files:**
- Create: `nexus-server/src/cron.rs`

- [ ] **Step 1: Implement cron poller**

Background task spawned at startup:
- Every 10s: query due jobs from DB
- For each: create InboundEvent with cron_job_id, publish to bus
- Update DB: last_run_at, run_count, compute next_run_at
- Handle delete_after_run and at-mode disable

Uses `cron` crate for expression parsing, `chrono-tz` for timezone-aware next occurrence.

- [ ] **Step 2: Build and verify**

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(server): cron poller — 10s poll, due job execution, scheduling modes"
```

---

### Task 11: Admin + Cron + Skills API Endpoints

**Files:**
- Create: `nexus-server/src/auth/admin.rs`
- Create: `nexus-server/src/auth/cron_api.rs`
- Create: `nexus-server/src/auth/skills_api.rs`
- Modify: `nexus-server/src/auth/mod.rs` (add modules)
- Modify: `nexus-server/src/main.rs` (merge routes, spawn cron poller)

- [ ] **Step 1: Create auth/admin.rs**

Admin-only endpoints (check `claims.is_admin`):
- GET/PUT default-soul
- GET/PUT rate-limit
- GET/PUT llm-config (hot-reload into Arc<RwLock>)
- GET admin/skills (list all users' skills)

- [ ] **Step 2: Create auth/cron_api.rs**

User cron job CRUD: GET list, POST create, PATCH update, DELETE remove.

- [ ] **Step 3: Create auth/skills_api.rs**

User skill CRUD: GET list, POST create (from content), POST install (from GitHub), DELETE.

- [ ] **Step 4: Wire everything into main.rs**

Add all new modules, merge routes, spawn cron poller, create reqwest client, load LLM config + default soul from DB on startup.

- [ ] **Step 5: Build and verify**

Run: `cargo build -p nexus-server`

- [ ] **Step 6: Commit**

```bash
git commit -m "feat(server): admin, cron, skills API endpoints + cron poller startup"
```

---

### Task 12: Integration Test — Full Agent Loop

- [ ] **Step 1: Reset database and start server**

- [ ] **Step 2: Configure LLM**

```bash
curl -s -X PUT http://localhost:8080/api/llm-config \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"api_base":"https://api.openai.com/v1","model":"gpt-4o-mini","api_key":"sk-...","context_window":128000}'
```

- [ ] **Step 3: Connect a nexus-client device**

- [ ] **Step 4: Send a message via gateway (or test endpoint) and verify agent loop executes**

This requires gateway (M3) or a test harness. For now, verify:
- Bus publish_inbound creates session and spawns agent loop
- Agent loop calls LLM, dispatches tool calls, returns result
- Context compression triggers on long conversations
- Server tools work (save_memory, web_fetch, cron)
- Rate limiting blocks excessive requests

- [ ] **Step 5: Commit any fixes**

---

## Summary

| Task | What | Key Files |
|---|---|---|
| 1 | LLM provider (OpenAI-compatible, retry, think-strip) | providers/openai.rs |
| 2 | Message bus (session routing, rate limiting) | bus.rs |
| 3 | Context builder (prompt assembly, identity, device status) | context.rs |
| 4 | Tools registry (schema merge, device routing) | tools_registry.rs |
| 5 | Server tools: memory + web_fetch | server_tools/memory.rs, web_fetch.rs |
| 6 | Server tools: message + file_transfer | server_tools/message.rs, file_transfer.rs |
| 7 | Server tools: cron + skills + DB CRUD | server_tools/cron_tool.rs, skills.rs, db/ |
| 8 | Agent loop (ReAct, tool dispatch, guards) | agent_loop.rs |
| 9 | Context compression (LLM summarization) | memory.rs |
| 10 | Cron poller (10s poll, due job execution) | cron.rs |
| 11 | Admin + cron + skills API endpoints | auth/admin.rs, cron_api.rs, skills_api.rs |
| 12 | Integration test | Manual |
