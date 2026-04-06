# nexus-common

Shared protocol, error types, and constants for the NEXUS system. This crate defines the contract between server and client.

---

## Module structure

```
nexus-common/src/
  lib.rs         -- module exports
  protocol.rs    -- WebSocket message types (ServerToClient, ClientToServer)
  error.rs       -- ErrorCode enum, NexusError, ApiError
  mcp_utils.rs   -- MCP schema normalization utilities
  consts.rs      -- shared constants
```

---

## protocol.rs

Server and Client communicate over a single WebSocket connection (`/ws`). All messages are JSON text frames.

### ServerToClient

| Variant | Purpose |
|---------|---------|
| `RequireLogin` | Prompt client to authenticate |
| `LoginSuccess` | Auth succeeded; includes `user_id`, `device_name`, `fs_policy`, `mcp_servers` |
| `LoginFailed` | Auth rejected with reason |
| `ExecuteToolRequest` | Agent loop requests tool execution (`request_id`, `tool_name`, `arguments`) |
| `FileUploadRequest` | Request file content from client (`request_id`, `file_path`) |
| `FileDownloadRequest` | Push file to client (`request_id`, `file_name`, `content_base64`, `destination_path`) |
| `HeartbeatAck` | Heartbeat response with updated `fs_policy` and `mcp_servers` |

### ClientToServer

| Variant | Purpose |
|---------|---------|
| `SubmitToken` | Authenticate with `token` and `protocol_version` |
| `RegisterTools` | Register tool schemas (`schemas: Vec<Value>`) |
| `Heartbeat` | Periodic heartbeat with `hash` (tools hash) and `status` |
| `ToolExecutionResult` | Tool execution result (`request_id`, `exit_code`, `output`) |
| `FileUploadResponse` | File content response (`request_id`, `file_name`, `content_base64`, `mime_type`, `error`) |
| `FileDownloadResponse` | File download acknowledgement (`request_id`, `error`) |

### Supporting types

- **`FsPolicy`** -- per-device filesystem access policy: `Sandbox` (default), `Whitelist { allowed_paths }`, `Unrestricted`
- **`McpServerEntry`** -- MCP server configuration (name, command, args, env, url, headers, timeout, enabled)

### Exit code conventions

| Value | Meaning |
|-------|---------|
| `0` | Success |
| `1` | Execution error |
| `-1` | Timeout |
| `-2` | Cancelled (device disconnected) |
| `-3` | Validation failed (guardrails blocked) |

---

## error.rs

### ErrorCode

Enum with 23 variants covering all error categories:

- **Auth:** `AuthFailed`, `AuthTokenExpired`, `Unauthorized`, `Forbidden`
- **General:** `NotFound`, `Conflict`, `ValidationFailed`, `InvalidParams`, `ExecutionFailed`, `ExecutionTimeout`
- **Device:** `DeviceNotFound`, `DeviceOffline`
- **Protocol:** `ProtocolMismatch`, `InternalError`
- **Tool:** `ToolBlocked`, `ToolTimeout`, `ToolNotFound`, `ToolInvalidParams`
- **MCP:** `McpConnectionFailed`, `McpCallFailed`
- **Connection:** `ConnectionFailed`, `HandshakeFailed`, `ChannelError`

Each variant maps to an HTTP status code via `http_status()` and a string representation via `as_str()`.

### NexusError

Internal error type for cross-crate use. Contains `ErrorCode` + message. Implements `std::error::Error`.

### ApiError

Standard JSON error response for HTTP handlers. Contains `code: String` + `message: String`. Implements axum `IntoResponse` (behind `axum` feature flag). Converts from `NexusError`.

---

## mcp_utils.rs

Utilities for converting MCP tool schemas into OpenAI-compatible function calling format. Handles nullable branch extraction from `oneOf`/`anyOf` patterns and schema normalization.

---

## consts.rs

| Constant | Value | Purpose |
|----------|-------|---------|
| `PROTOCOL_VERSION` | `"1.0"` | Handshake version check |
| `HEARTBEAT_INTERVAL_SEC` | `15` | Client heartbeat interval |
| `DEFAULT_MCP_TOOL_TIMEOUT_SEC` | `30` | MCP tool call timeout |
| `MAX_AGENT_ITERATIONS` | `200` | ReAct loop iteration limit |
| `MAX_HISTORY_MESSAGES` | `500` | Max history messages per LLM call |
| `MAX_TOOL_OUTPUT_CHARS` | `10000` | Tool output truncation threshold |
| `TOOL_OUTPUT_HEAD_CHARS` | `5000` | Truncation: head chars kept |
| `TOOL_OUTPUT_TAIL_CHARS` | `5000` | Truncation: tail chars kept |
| `EXIT_CODE_*` | `0, 1, -1, -2, -3` | Standard exit codes |
| `DEVICE_TOKEN_PREFIX` | `"nexus_dev_"` | Device token format prefix |
| `DEVICE_TOKEN_RANDOM_LEN` | `32` | Token random part length |

---

## Requirements

- Rust 1.85+ (edition 2024)
- Dependencies: `serde`, `serde_json` (core); `axum` (optional feature for `IntoResponse`)
