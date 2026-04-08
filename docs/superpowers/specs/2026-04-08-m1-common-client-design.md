# M1: nexus-common + nexus-client Design Spec

## Overview

M1 rebuilds the shared protocol layer (nexus-common) and the execution node (nexus-client) from scratch. The client connects to the server via WebSocket, authenticates with a device token, receives configuration via push, executes tools on behalf of the agent, and registers MCP tool servers.

**Build order:** nexus-common first (dependency of everything), then nexus-client bottom-up: skeleton → config → filesystem tools → shell + sandbox → MCP client → tool registration.

---

## 1. nexus-common

Shared crate containing protocol types, constants, error codes, and utilities. No business logic.

### 1.1 Modules

| Module | Purpose |
|---|---|
| `protocol.rs` | `ServerToClient` / `ClientToServer` enums, config structs |
| `consts.rs` | Shared constants (intervals, limits, exit codes, token format) |
| `error.rs` | `ErrorCode`, `ApiError`, `NexusError` |
| `mcp_utils.rs` | MCP schema normalization for OpenAI compatibility |
| `mime.rs` | MIME detection (extension + magic bytes) |

### 1.2 Protocol Messages

**ServerToClient:**

```rust
enum ServerToClient {
    RequireLogin { message: String },
    LoginSuccess {
        user_id: String,
        device_name: String,
        fs_policy: FsPolicy,
        mcp_servers: Vec<McpServerEntry>,
        workspace_path: String,
        shell_timeout: u64,
        ssrf_whitelist: Vec<String>,
    },
    LoginFailed { reason: String },
    HeartbeatAck,
    ExecuteToolRequest {
        request_id: String,
        tool_name: String,
        arguments: Value,
    },
    ConfigUpdate {
        fs_policy: Option<FsPolicy>,
        mcp_servers: Option<Vec<McpServerEntry>>,
        workspace_path: Option<String>,
        shell_timeout: Option<u64>,
        ssrf_whitelist: Option<Vec<String>>,
    },
}
```

**ClientToServer:**

```rust
enum ClientToServer {
    SubmitToken {
        token: String,
        protocol_version: String,
    },
    Heartbeat {
        status: DeviceStatus,
    },
    RegisterTools {
        schemas: Vec<Value>,
    },
    ToolExecutionResult {
        request_id: String,
        exit_code: i32,
        output: String,
    },
}
```

### 1.3 Shared Types

```rust
enum DeviceStatus { Online, Offline }

enum FsPolicy {
    Sandbox,        // workspace only (default)
    Unrestricted,   // full filesystem
}

struct McpServerEntry {
    name: String,
    transport_type: Option<String>,  // "stdio" | "sse" | "streamableHttp"
    command: String,
    args: Vec<String>,
    env: Option<HashMap<String, String>>,
    url: Option<String>,
    headers: Option<HashMap<String, String>>,
    tool_timeout: Option<u64>,
    enabled: bool,  // default true
}
```

### 1.4 Constants

| Constant | Value | Purpose |
|---|---|---|
| `PROTOCOL_VERSION` | `"1.0"` | Handshake version check |
| `HEARTBEAT_INTERVAL_SEC` | `15` | Client heartbeat interval |
| `DEFAULT_MCP_TOOL_TIMEOUT_SEC` | `30` | MCP tool call timeout |
| `MAX_AGENT_ITERATIONS` | `200` | Agent loop cap (server) |
| `MAX_TOOL_OUTPUT_CHARS` | `10_000` | Shell output truncation |
| `TOOL_OUTPUT_HEAD_CHARS` | `5_000` | First N chars kept |
| `TOOL_OUTPUT_TAIL_CHARS` | `5_000` | Last N chars kept |
| `EXIT_CODE_SUCCESS` | `0` | |
| `EXIT_CODE_ERROR` | `1` | |
| `EXIT_CODE_TIMEOUT` | `-1` | |
| `EXIT_CODE_CANCELLED` | `-2` | |
| `DEVICE_TOKEN_PREFIX` | `"nexus_dev_"` | Token validation |
| `DEVICE_TOKEN_RANDOM_LEN` | `32` | Token length check |
| `MAX_READ_FILE_CHARS` | `128_000` | read_file output cap |
| `DEFAULT_READ_FILE_LIMIT` | `2000` | read_file default line limit |
| `DEFAULT_LIST_DIR_MAX` | `200` | list_dir default max entries |
| `DEFAULT_SHELL_TIMEOUT_SEC` | `60` | Shell timeout when not configured |
| `SERVER_DEVICE_NAME` | `"server"` | Virtual device for server tools |

### 1.5 Changes from Old Code

| Action | Detail |
|---|---|
| Remove | `FsPolicy::Whitelist` variant |
| Remove | `FileUploadRequest`, `FileUploadResponse` |
| Remove | `FileDownloadRequest`, `FileDownloadResponse` |
| Remove | `HeartbeatAck` fields (fs_policy, mcp_servers) |
| Remove | `Heartbeat.hash` field |
| Add | `ServerToClient::ConfigUpdate` variant |
| Add | `LoginSuccess` fields: workspace_path, shell_timeout, ssrf_whitelist |
| Keep | `mcp_utils.rs`, `mime.rs`, `error.rs`, `consts.rs` as-is |

### 1.6 Cargo.toml

```toml
[package]
name = "nexus-common"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
axum = { version = "0.8", optional = true }

[features]
default = []
axum = ["dep:axum"]
```

---

## 2. nexus-client

Execution node that connects to the server, receives tool calls, and returns results.

### 2.1 Crate Structure

```
nexus-client/
├── Cargo.toml
├── src/
│   ├── main.rs           # Entry point, env loading, reconnect loop
│   ├── connection.rs     # WebSocket connect, auth handshake, message dispatch
│   ├── heartbeat.rs      # 15s heartbeat task
│   ├── config.rs         # Runtime config (FsPolicy, workspace, timeouts, SSRF)
│   ├── tools/
│   │   ├── mod.rs        # Tool trait, registry, dispatch, schema builder
│   │   ├── shell.rs      # Shell execution + guardrails + SSRF + bwrap
│   │   ├── read_file.rs
│   │   ├── write_file.rs
│   │   ├── edit_file.rs
│   │   ├── list_dir.rs
│   │   ├── glob.rs
│   │   ├── grep.rs
│   │   └── helpers.rs    # Path sanitization, timeout wrapper, output truncation
│   ├── sandbox.rs        # Bwrap wrapper (Linux only)
│   ├── env.rs            # Safe environment variables
│   ├── guardrails.rs     # Dangerous pattern deny-list + SSRF checker
│   └── mcp/
│       ├── mod.rs        # MCP manager: lifecycle, config diff, reinit
│       └── client.rs     # Single MCP server session (stdio, initialize, tools/list)
```

### 2.2 Dependencies

| Crate | Purpose |
|---|---|
| `nexus-common` | Protocol types, constants, errors |
| `tokio` | Async runtime, process, timers, fs |
| `tokio-tungstenite` | WebSocket client |
| `serde` / `serde_json` | Serialization |
| `tracing` / `tracing-subscriber` | Structured logging |
| `glob` | Glob pattern matching |
| `regex` | Grep patterns + guardrails deny-list |
| `rmcp` | MCP client SDK |

### 2.3 Environment Variables

| Var | Required | Purpose |
|---|---|---|
| `NEXUS_SERVER_WS_URL` (alias `NEXUS_WS_URL`) | Yes | WebSocket URL |
| `NEXUS_AUTH_TOKEN` (alias `NEXUS_DEVICE_TOKEN`) | Yes | Device token (`nexus_dev_` + 32 hex) |
| `RUST_LOG` | No | Tracing filter |

All other config (workspace, FsPolicy, MCP, timeouts, SSRF whitelist) received from server.

---

## 3. Connection & Reconnect

### 3.1 Startup

```
main()
  → load NEXUS_SERVER_WS_URL + NEXUS_AUTH_TOKEN from env
  → fail fast if either missing
  → enter reconnect_loop()
```

### 3.2 Reconnect Loop

Exponential backoff: 1s → 2s → 4s → ... → cap 30s. Reset to 1s on successful `LoginSuccess`.

### 3.3 Connection Flow

```
connect()
  → WebSocket upgrade to NEXUS_SERVER_WS_URL
  → receive RequireLogin { message }
  → send SubmitToken { token, protocol_version: "1.0" }
  → receive LoginSuccess or LoginFailed
  → on LoginFailed: log reason, return to reconnect loop
  → on LoginSuccess:
      → store config (fs_policy, workspace, mcp_servers, shell_timeout, ssrf_whitelist)
      → initialize MCP servers
      → build tool schemas (built-in + MCP)
      → send RegisterTools { schemas }
      → spawn heartbeat task
      → enter message_loop()
```

### 3.4 Message Loop

```
message_loop()
  → match incoming ServerToClient:
      ExecuteToolRequest → dispatch to tool handler → send ToolExecutionResult
      ConfigUpdate → merge into config → maybe reinit MCP servers
      HeartbeatAck → reset missed heartbeat counter
  → on disconnect/error → cancel heartbeat → cancel pending tools → return to reconnect
```

### 3.5 Heartbeat

- Send `Heartbeat { status: Online }` every 15 seconds
- Track missed acks (4 missed = 60s = connection dead → force reconnect)
- Lightweight: no config payload, just status ping

---

## 4. Runtime Config

```rust
struct ClientConfig {
    workspace: PathBuf,
    fs_policy: FsPolicy,
    shell_timeout: u64,
    ssrf_whitelist: Vec<String>,  // CIDR ranges
    mcp_servers: Vec<McpServerEntry>,
}
```

Stored in `Arc<RwLock<ClientConfig>>`. Updated atomically on `ConfigUpdate`. Read by tools via shared reference.

Single-user client — no DashMap needed.

---

## 5. Tool System

### 5.1 Tool Trait

```rust
trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;  // JSON Schema
    async fn execute(&self, args: Value, config: &ClientConfig) -> ToolResult;
}

struct ToolResult {
    exit_code: i32,
    output: String,
}
```

### 5.2 Dispatch

Tool name lookup in a `HashMap<String, Box<dyn Tool>>`. MCP tools prefixed with `mcp_{server}_{tool}` — prefix routes to MCP executor.

### 5.3 Error Convention

All tool errors return descriptive strings with a hint suffix:

```
"Error: file not found: /path/to/file\n\n[Analyze the error and try a different approach.]"
```

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | NotFound, InvalidParams, ExecutionFailed |
| -1 | Timeout |
| -2 | Blocked by guardrails |

---

## 6. Built-in Tools

### 6.1 read_file

**Params:** `path` (required), `offset` (default 1, 1-indexed), `limit` (default 2000)

**Behavior:**
- Format: `"{line_number}| {content}"` per line
- Images (PNG, JPEG, GIF, WebP by magic bytes): return `"[Image: {path}, {size}KB]"`
- Binary (non-UTF-8, non-image): error
- Total output capped at 128,000 characters
- Pagination hint appended: `"Showing lines 1-2000 of 50000. Use offset to read more."`
- Path validated against FsPolicy

### 6.2 write_file

**Params:** `path` (required), `content` (required)

**Behavior:**
- Parent dirs created with `create_dir_all`
- Atomic write via `tokio::fs::write`
- Returns success with char count
- Path validated against FsPolicy (write access)

### 6.3 edit_file

**Params:** `file_path` (required), `old_string` (required, non-empty), `new_string` (required)

**Behavior:**
- Exact match first: count occurrences
  - 0 matches: try fuzzy match (line-stripped sliding window), show closest match diff
  - 1 match: replace, write back
  - >1 matches: error with count, no edit
- Fuzzy matching (from nanobot): normalize both sides to LF, strip whitespace per line, find window of same line count with matching stripped content
- Path validated against FsPolicy (write access)

### 6.4 list_dir

**Params:** `path` (required), `recursive` (default false), `max_entries` (default 200)

**Behavior:**
- Non-recursive: `[DIR] name` / `[FILE] name`
- Recursive: relative paths, dirs with trailing `/`
- Sorted alphabetically
- Auto-ignored: `.git`, `node_modules`, `__pycache__`, `.venv`, `venv`, `dist`, `build`, `.tox`, `.mypy_cache`, `.pytest_cache`, `.ruff_cache`, `.coverage`, `htmlcov`
- Truncation message when exceeding max_entries

### 6.5 glob

**Params:** `pattern` (required), `path` (optional, default workspace)

**Behavior:**
- Match files using glob patterns
- Results sorted by modification time (newest first)
- Auto-ignores noise directories
- Path validated against FsPolicy (read access)

### 6.6 grep

**Params:** `pattern` (required, regex), `path` (optional, default workspace), `include` (optional, glob filter), `context` (optional, default 0)

**Behavior:**
- Search file contents with regex
- Return matches with file paths and line numbers
- Filter by file type via `include`
- Auto-ignores binary files and noise directories
- Path validated against FsPolicy (read access)

### 6.7 shell

**Params:** `command` (required), `timeout_sec` (optional), `working_dir` (optional, default workspace)

**Behavior:**
- Unix: `bash -l -c "{command}"` (login shell for proper PATH)
- Windows: `cmd /C "{command}"`
- Environment always isolated (see Section 8)
- In Sandbox mode: guardrails check first, then bwrap if available
- Output: stdout + stderr (prefixed `STDERR:\n`) + `\nExit code: {code}`
- Output truncation at 10K chars: first 5K + `"\n... ({total} chars truncated) ...\n"` + last 5K
- On timeout: kill process, return timeout error with duration

---

## 7. Path Sanitization

Shared helper in `tools/helpers.rs`.

```
sanitize_path(path: &str, config: &ClientConfig, write: bool) -> Result<PathBuf>
```

1. Expand `~` to home directory
2. If relative → join to workspace root
3. `canonicalize()` to resolve symlinks (for new files: canonicalize parent)
4. **Sandbox mode:** check path starts with workspace. Exception: `/dev/null` and `/tmp/nexus*`
5. **Unrestricted mode:** allow all paths
6. Return canonical absolute path

---

## 8. Environment Isolation

Always active, both Sandbox and Unrestricted modes. Defined in `env.rs`.

```rust
fn safe_env() -> Vec<(&'static str, String)> {
    vec![
        ("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"),
        ("HOME", std::env::var("HOME").unwrap_or_default()),
        ("LANG", "en_US.UTF-8"),
        ("TERM", "xterm-256color"),
    ]
}
// Windows: PATH = "C:\Windows\system32;C:\Windows;C:\Windows\System32\Wbem"
```

Applied via `Command::new("bash").env_clear()` then inject only safe vars. Prevents leaking `AWS_SECRET_ACCESS_KEY`, `GITHUB_TOKEN`, `DATABASE_URL`, `NEXUS_AUTH_TOKEN`, etc.

---

## 9. Guardrails (Sandbox Mode Only)

Defined in `guardrails.rs`. Two checks run before shell execution.

### 9.1 Dangerous Command Deny-List

Regex patterns compiled once at startup (`LazyLock`):

| Pattern | Blocks |
|---|---|
| `\brm\s+-[rf]{1,2}\b` | rm -rf, rm -r, rm -f |
| `\bdel\s+/[fq]\b` | Windows del /f, del /q |
| `\bformat\s+[a-z]:` | Drive formatting |
| `\bdd\s+if=\b` | Direct device read |
| `:()\s*\{.*?\};:` | Fork bombs |
| `\b(shutdown\|reboot\|poweroff\|init\s+0\|init\s+6)\b` | System shutdown |
| `>\s*/dev/sd[a-z]` | Direct disk writes |
| `\b(mkfifo\|mknod)\s+/dev/` | Device file creation |

Match = `ToolResult { exit_code: -2, output: "Blocked: ..." }`.

### 9.2 SSRF Protection

1. Extract URLs matching `https?://[^\s'"]+` from command
2. For each URL:
   - IP literal → check against blocked ranges
   - Domain → async DNS resolve → check all resolved IPs
   - DNS failure → conservatively block
3. Per-device SSRF whitelist (CIDR ranges) bypasses block list

**Blocked IP ranges:**

| Range | Purpose |
|---|---|
| `0.0.0.0/8` | Current network |
| `10.0.0.0/8` | Private (RFC 1918) |
| `100.64.0.0/10` | Shared/CGN |
| `127.0.0.0/8` | Loopback |
| `169.254.0.0/16` | Link-local (cloud metadata) |
| `172.16.0.0/12` | Private (RFC 1918) |
| `192.168.0.0/16` | Private (RFC 1918) |
| `::1/128` | IPv6 loopback |
| `fc00::/7` | IPv6 unique local |
| `fe80::/10` | IPv6 link-local |

### 9.3 Path Traversal (Shell Commands)

- Block `../` and `..\` patterns in command string
- Extract absolute paths → validate within workspace
- Exceptions: `/dev/null`, `/tmp/nexus*` always allowed

---

## 10. Bubblewrap Sandbox (Linux Only, Sandbox Mode)

Defined in `sandbox.rs`. Checked once at startup via `LazyLock<bool>` (probe `bwrap --version`).

### 10.1 Mount Layout

| Mount | Type | Purpose |
|---|---|---|
| `/usr` | `--ro-bind` | System binaries |
| `/bin`, `/lib`, `/lib64` | `--ro-bind-try` | Additional system dirs |
| `/etc/alternatives` | `--ro-bind-try` | Debian alternatives |
| `/etc/ssl/certs` | `--ro-bind-try` | TLS certificates |
| `/etc/resolv.conf` | `--ro-bind-try` | DNS |
| `/etc/ld.so.cache` | `--ro-bind-try` | Linker cache |
| `/proc` | `--proc` | Minimal procfs |
| `/dev` | `--dev` | Minimal devfs |
| `/tmp` | `--tmpfs` | Isolated tmpfs |
| Workspace parent | `--tmpfs` | Masks home dir (hides ~/.ssh, ~/.nexus) |
| Workspace | `--dir` + `--bind` | Read-write on top of parent mask |

### 10.2 Flags

- `--new-session` — prevents signal injection
- `--die-with-parent` — sandbox dies if client exits

### 10.3 Command Wrapping

```
bwrap [mount flags] -- bash -l -c '{escaped_command}'
```

Arguments shell-escaped (single-quote special chars).

### 10.4 Graceful Degradation

If bwrap not installed → commands execute directly with guardrails + env isolation still active. Log warning once at startup.

---

## 11. MCP Client

### 11.1 Architecture

```
mcp/
├── mod.rs    # McpManager: owns all server sessions, handles lifecycle
└── client.rs # McpSession: single server connection (stdio)
```

### 11.2 Lifecycle

1. On `LoginSuccess`: receive `mcp_servers` config, start all enabled servers
2. On `ConfigUpdate` with `mcp_servers`: diff against current state
   - New/changed servers → start/restart
   - Removed servers → stop and unregister tools
3. Per server startup:
   - Spawn child process with configured command/args/env
   - MCP `initialize` handshake
   - `tools/list` to discover available tools
   - Register each tool with prefix: `mcp_{server_name}_{tool_name}`

### 11.3 Tool Call Forwarding

- Incoming `ExecuteToolRequest` with name starting `mcp_` → extract server name → forward to correct `McpSession`
- Apply `tool_timeout` (per-server, default 30s) via `tokio::time::timeout`
- Return result as `ToolExecutionResult`

### 11.4 Schema Normalization

Use `nexus-common::mcp_utils::normalize_schema_for_openai()` to convert MCP tool schemas before registration. Handles nullable types, oneOf/anyOf, ensures object types have properties/required.

### 11.5 Error Handling

- MCP server crash → log warning, mark tools unavailable, don't crash client
- Tool call to unavailable server → return error with exit_code 1
- On reconnect or ConfigUpdate → retry initialization

---

## 12. Tool Registration

After login and MCP initialization:

1. Collect schemas from all 7 built-in tools
2. Collect schemas from all MCP tools (prefixed, normalized)
3. Send `RegisterTools { schemas }` to server
4. On MCP config change: rebuild schemas, re-send `RegisterTools`

Schema format per tool (OpenAI function calling):

```json
{
    "type": "function",
    "function": {
        "name": "read_file",
        "description": "Read file contents with line numbers...",
        "parameters": {
            "type": "object",
            "properties": { ... },
            "required": ["path"]
        }
    }
}
```

---

## 13. Build Order

| Step | What | Depends On | Testable |
|---|---|---|---|
| 1 | nexus-common cleanup | — | `cargo test -p nexus-common` |
| 2 | Client skeleton (main, connection, heartbeat) | nexus-common | Connects to mock/real server |
| 3 | Config system | Step 2 | Unit tests on config merge |
| 4 | Filesystem tools (read, write, edit, list_dir, glob, grep) | Step 3 | Unit tests with temp dirs |
| 5 | Shell tool + guardrails + env isolation | Step 3 | Unit tests + integration |
| 6 | Bwrap sandbox | Step 5 | Linux integration tests |
| 7 | MCP client | Step 2 | Integration with test MCP server |
| 8 | Tool registration + full integration | Steps 4-7 | End-to-end with server |
