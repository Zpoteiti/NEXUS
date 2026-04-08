# M1: nexus-common + nexus-client Implementation Plan

**For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the shared protocol layer (nexus-common) and implement the execution node (nexus-client) from scratch, establishing the foundation for distributed tool execution.

**Architecture:**
- nexus-common: Clean protocol types (ServerToClient/ClientToServer enums), shared constants, error handling, and utilities (MCP schema normalization, MIME detection)
- nexus-client: Standalone Rust binary that connects via WebSocket to server, authenticates with device token, receives config via push, executes 7 built-in tools + MCP servers, handles reconnection with exponential backoff

**Tech Stack:** Rust 1.85+, tokio, tokio-tungstenite, serde, tracing, glob, regex, rmcp (MCP client SDK)

**Spec:** `docs/superpowers/specs/2026-04-08-m1-common-client-design.md`

---

## Section 1: Workspace Setup + nexus-common (From Scratch)

> **Note:** nexus-common was deleted. All files below are created fresh per spec.

**Files:**
- Create: `NEXUS/Cargo.toml` (workspace root)
- Create: `NEXUS/nexus-common/Cargo.toml`
- Create: `NEXUS/nexus-common/src/lib.rs`
- Create: `NEXUS/nexus-common/src/consts.rs`
- Create: `NEXUS/nexus-common/src/protocol.rs`
- Create: `NEXUS/nexus-common/src/error.rs`
- Create: `NEXUS/nexus-common/src/mcp_utils.rs`
- Create: `NEXUS/nexus-common/src/mime.rs`

---

### Task 1: Create Cargo Workspace + nexus-common Crate

**Files:**
- Create: `NEXUS/Cargo.toml`
- Create: `NEXUS/nexus-common/Cargo.toml`
- Create: `NEXUS/nexus-common/src/lib.rs`

- [ ] **Step 1: Create workspace root Cargo.toml**

Create `NEXUS/Cargo.toml`:

```toml
[workspace]
members = ["nexus-common", "nexus-client"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2024"
authors = ["NEXUS Team"]
license = "MIT"

[profile.release]
opt-level = 3
lto = true
```

- [ ] **Step 2: Create nexus-common/Cargo.toml**

```toml
[package]
name = "nexus-common"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
axum = { version = "0.8", optional = true }

[features]
default = []
axum = ["dep:axum"]

[lints.rust]
unsafe_code = "forbid"
```

- [ ] **Step 3: Create nexus-common/src/lib.rs**

```rust
pub mod consts;
pub mod error;
pub mod mcp_utils;
pub mod mime;
pub mod protocol;
```

- [ ] **Step 4: Verify crate structure**

Run: `cd /home/yucheng/Documents/GitHub/NEXUS/nexus-common && cargo check 2>&1 | head -5`

Expected: Errors about missing modules — correct, we create them next.

---

### Task 2: Create consts.rs

**Files:**
- Create: `NEXUS/nexus-common/src/consts.rs`

- [ ] **Step 1: Write consts.rs with all constants from spec section 1.4**

```rust
/// Shared constants between Server and Client.
/// Prevents hardcoded magic numbers or strings on either side.

pub const PROTOCOL_VERSION: &str = "1.0";
pub const HEARTBEAT_INTERVAL_SEC: u64 = 15;
pub const DEFAULT_MCP_TOOL_TIMEOUT_SEC: u64 = 30;
pub const MAX_AGENT_ITERATIONS: u32 = 200;
pub const MAX_TOOL_OUTPUT_CHARS: usize = 10_000;
pub const TOOL_OUTPUT_HEAD_CHARS: usize = 5_000;
pub const TOOL_OUTPUT_TAIL_CHARS: usize = 5_000;

pub const EXIT_CODE_SUCCESS: i32 = 0;
pub const EXIT_CODE_ERROR: i32 = 1;
pub const EXIT_CODE_TIMEOUT: i32 = -1;
pub const EXIT_CODE_CANCELLED: i32 = -2;

pub const DEVICE_TOKEN_PREFIX: &str = "nexus_dev_";
pub const DEVICE_TOKEN_RANDOM_LEN: usize = 32;

pub const SERVER_DEVICE_NAME: &str = "server";

// File tool limits
pub const MAX_READ_FILE_CHARS: usize = 128_000;
pub const DEFAULT_READ_FILE_LIMIT: usize = 2000;
pub const DEFAULT_LIST_DIR_MAX: usize = 200;

// Shell timeout
pub const DEFAULT_SHELL_TIMEOUT_SEC: u64 = 60;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants_values() {
        assert_eq!(PROTOCOL_VERSION, "1.0");
        assert_eq!(HEARTBEAT_INTERVAL_SEC, 15);
        assert_eq!(MAX_TOOL_OUTPUT_CHARS, TOOL_OUTPUT_HEAD_CHARS + TOOL_OUTPUT_TAIL_CHARS);
        assert_eq!(EXIT_CODE_SUCCESS, 0);
        assert_eq!(DEVICE_TOKEN_PREFIX, "nexus_dev_");
        assert_eq!(DEVICE_TOKEN_RANDOM_LEN, 32);
    }

    #[test]
    fn test_file_tool_constants() {
        assert_eq!(MAX_READ_FILE_CHARS, 128_000);
        assert_eq!(DEFAULT_READ_FILE_LIMIT, 2000);
        assert_eq!(DEFAULT_LIST_DIR_MAX, 200);
    }

    #[test]
    fn test_shell_timeout_default() {
        assert_eq!(DEFAULT_SHELL_TIMEOUT_SEC, 60);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p nexus-common consts`

Expected: All 3 tests pass.

---

### Task 3: Create protocol.rs

**Files:**
- Create: `NEXUS/nexus-common/src/protocol.rs`

- [ ] **Step 1: Write protocol.rs per spec sections 1.2-1.3**

```rust
/// WebSocket protocol types for Server-Client communication.
/// Contains only serialization structs/enums — no business logic.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceStatus {
    #[serde(rename = "online")]
    Online,
    #[serde(rename = "offline")]
    Offline,
}

/// Two-tier filesystem policy. Sandbox = workspace only (default). Unrestricted = full access.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode")]
pub enum FsPolicy {
    #[serde(rename = "sandbox")]
    Sandbox,
    #[serde(rename = "unrestricted")]
    Unrestricted,
}

impl Default for FsPolicy {
    fn default() -> Self {
        FsPolicy::Sandbox
    }
}

/// MCP server configuration entry, stored per-device in the DB.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpServerEntry {
    pub name: String,
    #[serde(default)]
    pub transport_type: Option<String>,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Option<std::collections::HashMap<String, String>>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: Option<std::collections::HashMap<String, String>>,
    #[serde(default)]
    pub tool_timeout: Option<u64>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool { true }

/// Commands sent from server to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ServerToClient {
    ExecuteToolRequest(ExecuteToolRequest),
    RequireLogin {
        message: String,
    },
    LoginSuccess {
        user_id: String,
        device_name: String,
        fs_policy: FsPolicy,
        mcp_servers: Vec<McpServerEntry>,
        workspace_path: String,
        shell_timeout: u64,
        ssrf_whitelist: Vec<String>,
    },
    LoginFailed {
        reason: String,
    },
    HeartbeatAck,
    ConfigUpdate {
        fs_policy: Option<FsPolicy>,
        mcp_servers: Option<Vec<McpServerEntry>>,
        workspace_path: Option<String>,
        shell_timeout: Option<u64>,
        ssrf_whitelist: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolRequest {
    pub request_id: String,
    pub tool_name: String,
    pub arguments: Value,
}

/// Events reported from client to server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ClientToServer {
    ToolExecutionResult(ToolExecutionResult),
    SubmitToken {
        token: String,
        protocol_version: String,
    },
    RegisterTools {
        schemas: Vec<Value>,
    },
    Heartbeat {
        status: DeviceStatus,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecutionResult {
    pub request_id: String,
    /// 0=success, 1=failed, -1=timeout, -2=cancelled
    pub exit_code: i32,
    pub output: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fspolicy_default_is_sandbox() {
        assert_eq!(FsPolicy::default(), FsPolicy::Sandbox);
    }

    #[test]
    fn test_fspolicy_serialize_deserialize() {
        let sandbox = FsPolicy::Sandbox;
        let json = serde_json::to_string(&sandbox).unwrap();
        assert_eq!(json, r#"{"mode":"sandbox"}"#);
        let round: FsPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(round, sandbox);

        let unrestricted = FsPolicy::Unrestricted;
        let json = serde_json::to_string(&unrestricted).unwrap();
        assert_eq!(json, r#"{"mode":"unrestricted"}"#);
    }

    #[test]
    fn test_login_success_round_trip() {
        let msg = ServerToClient::LoginSuccess {
            user_id: "u1".into(), device_name: "d1".into(),
            fs_policy: FsPolicy::Sandbox, mcp_servers: vec![],
            workspace_path: "/home/dev".into(), shell_timeout: 60,
            ssrf_whitelist: vec!["10.0.0.0/8".into()],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let de: ServerToClient = serde_json::from_str(&json).unwrap();
        match de {
            ServerToClient::LoginSuccess { workspace_path, shell_timeout, ssrf_whitelist, .. } => {
                assert_eq!(workspace_path, "/home/dev");
                assert_eq!(shell_timeout, 60);
                assert_eq!(ssrf_whitelist, vec!["10.0.0.0/8"]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_config_update_partial() {
        let msg = ServerToClient::ConfigUpdate {
            fs_policy: Some(FsPolicy::Unrestricted), mcp_servers: None,
            workspace_path: None, shell_timeout: Some(120), ssrf_whitelist: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let _: ServerToClient = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn test_heartbeat_ack_lightweight() {
        let msg = ServerToClient::HeartbeatAck;
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"HeartbeatAck"}"#);
    }

    #[test]
    fn test_heartbeat_no_hash() {
        let msg = ClientToServer::Heartbeat { status: DeviceStatus::Online };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("hash"));
    }

    #[test]
    fn test_submit_token_round_trip() {
        let msg = ClientToServer::SubmitToken {
            token: "nexus_dev_abc".into(), protocol_version: "1.0".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let _: ClientToServer = serde_json::from_str(&json).unwrap();
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p nexus-common protocol`

Expected: All 7 tests pass.

---

### Task 4: Create error.rs

**Files:**
- Create: `NEXUS/nexus-common/src/error.rs`

- [ ] **Step 1: Write error.rs per old crate pattern (23 ErrorCode variants + ApiError + NexusError)**

```rust
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    AuthFailed, AuthTokenExpired, Unauthorized, Forbidden,
    NotFound, Conflict, ValidationFailed, InvalidParams,
    ExecutionFailed, ExecutionTimeout, DeviceNotFound, DeviceOffline,
    ProtocolMismatch, InternalError, ToolBlocked, ToolTimeout,
    ToolNotFound, ToolInvalidParams, McpConnectionFailed, McpCallFailed,
    ConnectionFailed, HandshakeFailed, ChannelError,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AuthFailed => "AUTH_FAILED",
            Self::AuthTokenExpired => "AUTH_TOKEN_EXPIRED",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Forbidden => "FORBIDDEN",
            Self::NotFound => "NOT_FOUND",
            Self::Conflict => "CONFLICT",
            Self::ValidationFailed => "VALIDATION_FAILED",
            Self::InvalidParams => "INVALID_PARAMS",
            Self::ExecutionFailed => "EXECUTION_FAILED",
            Self::ExecutionTimeout => "EXECUTION_TIMEOUT",
            Self::DeviceNotFound => "DEVICE_NOT_FOUND",
            Self::DeviceOffline => "DEVICE_OFFLINE",
            Self::ProtocolMismatch => "PROTOCOL_MISMATCH",
            Self::InternalError => "INTERNAL_ERROR",
            Self::ToolBlocked => "TOOL_BLOCKED",
            Self::ToolTimeout => "TOOL_TIMEOUT",
            Self::ToolNotFound => "TOOL_NOT_FOUND",
            Self::ToolInvalidParams => "TOOL_INVALID_PARAMS",
            Self::McpConnectionFailed => "MCP_CONNECTION_FAILED",
            Self::McpCallFailed => "MCP_CALL_FAILED",
            Self::ConnectionFailed => "CONNECTION_FAILED",
            Self::HandshakeFailed => "HANDSHAKE_FAILED",
            Self::ChannelError => "CHANNEL_ERROR",
        }
    }

    pub fn http_status(&self) -> u16 {
        match self {
            Self::AuthFailed | Self::AuthTokenExpired | Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NotFound | Self::DeviceNotFound | Self::ToolNotFound => 404,
            Self::Conflict => 409,
            Self::ValidationFailed | Self::InvalidParams
            | Self::ToolInvalidParams | Self::ProtocolMismatch => 400,
            Self::ExecutionTimeout | Self::ToolTimeout => 504,
            Self::DeviceOffline => 503,
            Self::McpConnectionFailed | Self::McpCallFailed
            | Self::ConnectionFailed | Self::HandshakeFailed => 502,
            Self::ExecutionFailed | Self::InternalError
            | Self::ToolBlocked | Self::ChannelError => 500,
        }
    }

    pub fn from_str(s: &str) -> Option<ErrorCode> {
        match s {
            "AUTH_FAILED" => Some(Self::AuthFailed),
            "AUTH_TOKEN_EXPIRED" => Some(Self::AuthTokenExpired),
            "UNAUTHORIZED" => Some(Self::Unauthorized),
            "FORBIDDEN" => Some(Self::Forbidden),
            "NOT_FOUND" => Some(Self::NotFound),
            "CONFLICT" => Some(Self::Conflict),
            "VALIDATION_FAILED" => Some(Self::ValidationFailed),
            "INVALID_PARAMS" => Some(Self::InvalidParams),
            "EXECUTION_FAILED" => Some(Self::ExecutionFailed),
            "EXECUTION_TIMEOUT" => Some(Self::ExecutionTimeout),
            "DEVICE_NOT_FOUND" => Some(Self::DeviceNotFound),
            "DEVICE_OFFLINE" => Some(Self::DeviceOffline),
            "PROTOCOL_MISMATCH" => Some(Self::ProtocolMismatch),
            "INTERNAL_ERROR" => Some(Self::InternalError),
            "TOOL_BLOCKED" => Some(Self::ToolBlocked),
            "TOOL_TIMEOUT" => Some(Self::ToolTimeout),
            "TOOL_NOT_FOUND" => Some(Self::ToolNotFound),
            "TOOL_INVALID_PARAMS" => Some(Self::ToolInvalidParams),
            "MCP_CONNECTION_FAILED" => Some(Self::McpConnectionFailed),
            "MCP_CALL_FAILED" => Some(Self::McpCallFailed),
            "CONNECTION_FAILED" => Some(Self::ConnectionFailed),
            "HANDSHAKE_FAILED" => Some(Self::HandshakeFailed),
            "CHANNEL_ERROR" => Some(Self::ChannelError),
            _ => None,
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code: code.as_str().to_string(), message: message.into() }
    }

    pub fn http_status_code(&self) -> u16 {
        ErrorCode::from_str(&self.code).map(|c| c.http_status()).unwrap_or(500)
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

#[derive(Debug, Clone)]
pub struct NexusError {
    pub code: ErrorCode,
    pub message: String,
}

impl NexusError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl fmt::Display for NexusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for NexusError {}

impl From<NexusError> for ApiError {
    fn from(e: NexusError) -> Self { ApiError::new(e.code, e.message) }
}

#[cfg(feature = "axum")]
impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = axum::http::StatusCode::from_u16(self.http_status_code())
            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::to_string(&self).unwrap_or_default();
        (status, [("content-type", "application/json")], body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_round_trip() {
        let code = ErrorCode::AuthFailed;
        assert_eq!(ErrorCode::from_str(code.as_str()), Some(code));
    }

    #[test]
    fn test_all_codes_have_valid_http_status() {
        let codes = [
            ErrorCode::AuthFailed, ErrorCode::NotFound, ErrorCode::InternalError,
            ErrorCode::DeviceOffline, ErrorCode::ToolTimeout, ErrorCode::ChannelError,
        ];
        for code in codes {
            let s = code.http_status();
            assert!(s >= 400 && s < 600, "Bad status for {code}: {s}");
        }
    }

    #[test]
    fn test_api_error_display() {
        let err = ApiError::new(ErrorCode::NotFound, "missing");
        assert_eq!(err.to_string(), "[NOT_FOUND] missing");
        assert_eq!(err.http_status_code(), 404);
    }

    #[test]
    fn test_nexus_error_into_api_error() {
        let ne = NexusError::new(ErrorCode::InternalError, "oops");
        let ae: ApiError = ne.into();
        assert_eq!(ae.code, "INTERNAL_ERROR");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p nexus-common error`

Expected: All 4 tests pass.

---

### Task 5: Create mcp_utils.rs

**Files:**
- Create: `NEXUS/nexus-common/src/mcp_utils.rs`

- [ ] **Step 1: Write mcp_utils.rs — MCP schema normalization for OpenAI compatibility**

```rust
use serde_json::{Map, Value};

/// Extract the non-null branch from a oneOf/anyOf array that includes a null type.
pub fn extract_nullable_branch(options: &[Value]) -> Option<(Value, bool)> {
    let non_null: Vec<&Value> = options.iter()
        .filter(|v| v.get("type").and_then(Value::as_str) != Some("null"))
        .collect();
    let has_null = options.iter()
        .any(|v| v.get("type").and_then(Value::as_str) == Some("null"));
    if non_null.len() == 1 && has_null {
        Some((non_null[0].clone(), true))
    } else {
        None
    }
}

/// Normalize an MCP tool schema to be compatible with OpenAI function calling.
pub fn normalize_schema_for_openai(schema: &Value) -> Value {
    let Some(obj) = schema.as_object() else { return schema.clone() };
    let mut result = Map::new();

    // Handle type: ["string", "null"] -> type: "string", nullable: true
    if let Some(type_val) = obj.get("type") {
        if let Some(arr) = type_val.as_array() {
            let non_null: Vec<&Value> = arr.iter().filter(|v| v.as_str() != Some("null")).collect();
            let has_null = arr.iter().any(|v| v.as_str() == Some("null"));
            if non_null.len() == 1 {
                result.insert("type".into(), non_null[0].clone());
                if has_null { result.insert("nullable".into(), Value::Bool(true)); }
            } else {
                result.insert("type".into(), type_val.clone());
            }
        } else {
            result.insert("type".into(), type_val.clone());
        }
    }

    // Handle oneOf/anyOf with single non-null branch
    for key in &["oneOf", "anyOf"] {
        if let Some(Value::Array(options)) = obj.get(*key) {
            if let Some((branch, is_nullable)) = extract_nullable_branch(options) {
                let normalized = normalize_schema_for_openai(&branch);
                if let Some(branch_obj) = normalized.as_object() {
                    for (k, v) in branch_obj { result.insert(k.clone(), v.clone()); }
                }
                if is_nullable { result.insert("nullable".into(), Value::Bool(true)); }
                for (k, v) in obj {
                    if k != *key && !result.contains_key(k) { result.insert(k.clone(), v.clone()); }
                }
                return Value::Object(result);
            }
        }
    }

    // Recursively normalize properties
    if let Some(Value::Object(props)) = obj.get("properties") {
        let mut np = Map::new();
        for (k, v) in props { np.insert(k.clone(), normalize_schema_for_openai(v)); }
        result.insert("properties".into(), Value::Object(np));
    }

    // Recursively normalize items
    if let Some(items) = obj.get("items") {
        result.insert("items".into(), normalize_schema_for_openai(items));
    }

    // Ensure object types have properties and required
    if result.get("type").and_then(Value::as_str) == Some("object") {
        if !result.contains_key("properties") { result.insert("properties".into(), Value::Object(Map::new())); }
        if !result.contains_key("required") { result.insert("required".into(), Value::Array(vec![])); }
    }

    // Copy remaining keys
    for (k, v) in obj { if !result.contains_key(k) { result.insert(k.clone(), v.clone()); } }

    Value::Object(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_normalize_nullable_type() {
        let s = json!({"type": ["string", "null"]});
        let r = normalize_schema_for_openai(&s);
        assert_eq!(r["type"], "string");
        assert_eq!(r["nullable"], true);
    }

    #[test]
    fn test_normalize_oneof_nullable() {
        let s = json!({"oneOf": [{"type": "string"}, {"type": "null"}]});
        let r = normalize_schema_for_openai(&s);
        assert_eq!(r["type"], "string");
        assert_eq!(r["nullable"], true);
    }

    #[test]
    fn test_normalize_anyof_nullable() {
        let s = json!({"anyOf": [{"type": "integer"}, {"type": "null"}]});
        let r = normalize_schema_for_openai(&s);
        assert_eq!(r["type"], "integer");
        assert_eq!(r["nullable"], true);
    }

    #[test]
    fn test_normalize_nested_properties() {
        let s = json!({"type": "object", "properties": {"name": {"type": ["string", "null"]}}});
        let r = normalize_schema_for_openai(&s);
        assert_eq!(r["properties"]["name"]["type"], "string");
        assert_eq!(r["properties"]["name"]["nullable"], true);
    }

    #[test]
    fn test_normalize_object_has_required() {
        let s = json!({"type": "object"});
        let r = normalize_schema_for_openai(&s);
        assert!(r.get("properties").is_some());
        assert!(r.get("required").is_some());
    }

    #[test]
    fn test_normalize_passthrough_simple() {
        let s = json!({"type": "string", "description": "A name"});
        let r = normalize_schema_for_openai(&s);
        assert_eq!(r["type"], "string");
        assert!(r.get("nullable").is_none());
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p nexus-common mcp_utils`

Expected: All 6 tests pass.

---

### Task 6: Create mime.rs

**Files:**
- Create: `NEXUS/nexus-common/src/mime.rs`

- [ ] **Step 1: Write mime.rs — MIME detection by extension + magic bytes**

```rust
pub fn detect_mime_from_extension(filename: &str) -> Option<&'static str> {
    let lower = filename.to_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        return Some("application/gzip");
    }
    let ext = lower.rsplit('.').next()?;
    match ext {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "pdf" => Some("application/pdf"),
        "txt" => Some("text/plain"),
        "json" => Some("application/json"),
        "csv" => Some("text/csv"),
        "zip" => Some("application/zip"),
        "mp3" => Some("audio/mpeg"),
        "mp4" => Some("video/mp4"),
        _ => None,
    }
}

pub fn detect_mime_from_bytes(data: &[u8]) -> Option<&'static str> {
    if data.len() < 4 { return None; }
    match &data[..4] {
        [0x89, b'P', b'N', b'G'] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, _] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8'] => Some("image/gif"),
        [b'R', b'I', b'F', b'F'] if data.len() >= 12 && &data[8..12] == b"WEBP" => Some("image/webp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn ext_png() { assert_eq!(detect_mime_from_extension("photo.PNG"), Some("image/png")); }
    #[test] fn ext_jpeg() { assert_eq!(detect_mime_from_extension("a.jpeg"), Some("image/jpeg")); }
    #[test] fn ext_gif() { assert_eq!(detect_mime_from_extension("x.gif"), Some("image/gif")); }
    #[test] fn ext_webp() { assert_eq!(detect_mime_from_extension("x.webp"), Some("image/webp")); }
    #[test] fn ext_bmp() { assert_eq!(detect_mime_from_extension("x.bmp"), Some("image/bmp")); }
    #[test] fn ext_pdf() { assert_eq!(detect_mime_from_extension("doc.pdf"), Some("application/pdf")); }
    #[test] fn ext_unknown() { assert_eq!(detect_mime_from_extension("file.xyz"), None); }
    #[test] fn ext_tar_gz() { assert_eq!(detect_mime_from_extension("a.tar.gz"), Some("application/gzip")); }
    #[test] fn bytes_png() { assert_eq!(detect_mime_from_bytes(&[0x89, b'P', b'N', b'G']), Some("image/png")); }
    #[test] fn bytes_jpeg() { assert_eq!(detect_mime_from_bytes(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg")); }
    #[test] fn bytes_gif() { assert_eq!(detect_mime_from_bytes(b"GIF89a"), Some("image/gif")); }
    #[test] fn bytes_webp() { assert_eq!(detect_mime_from_bytes(b"RIFF\x00\x00\x00\x00WEBP"), Some("image/webp")); }
    #[test] fn bytes_unknown() { assert_eq!(detect_mime_from_bytes(b"hello world!!"), None); }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p nexus-common mime`

Expected: All 13 tests pass.

---

### Task 7: Verify full nexus-common build

- [ ] **Step 1: Run all tests**

Run: `cargo test -p nexus-common`

Expected: ~33 tests pass (consts:3, protocol:7, error:4, mcp_utils:6, mime:13)

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p nexus-common -- -D warnings`

Expected: No warnings.

- [ ] **Step 3: Commit**

```bash
cd /home/yucheng/Documents/GitHub/NEXUS
git add Cargo.toml nexus-common/
git commit -m "feat(M1): create nexus-common crate — protocol types, constants, errors, MCP utils, MIME detection"
```

---

## Section 2: Client Crate Skeleton

**Files:**
- Create: `NEXUS/nexus-client/Cargo.toml`
- Create: `NEXUS/nexus-client/src/main.rs`
- Create: `NEXUS/nexus-client/src/env.rs`
- Create: `NEXUS/nexus-client/src/config.rs`
- Create: `NEXUS/nexus-client/src/connection.rs`
- Create: `NEXUS/nexus-client/src/heartbeat.rs`

---

### Task 8: Create nexus-client Cargo.toml

- [ ] **Step 1: Write Cargo.toml**

```toml
[package]
name = "nexus-client"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[dependencies]
nexus-common = { path = "../nexus-common" }
tokio = { version = "1", features = ["full"] }
tokio-tungstenite = { version = "0.26", features = ["native-tls"] }
futures-util = "0.3"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
glob = "0.3"
regex = "1"
ipnet = "2"
rmcp = { version = "0.1", features = ["client", "transport-child-process"] }

[dev-dependencies]
tempfile = "3"

[lints.rust]
unsafe_code = "forbid"
```

- [ ] **Step 2: Create stub main.rs**

```rust
fn main() { println!("nexus-client stub"); }
```

- [ ] **Step 3: Verify workspace compiles**

Run: `cargo check --workspace`

Expected: Both crates resolve. (Adjust `rmcp` version if needed — check crates.io.)

---

### Task 9: Create env.rs

- [ ] **Step 1: Write env.rs — safe environment variables for subprocess execution**

```rust
/// Safe environment variables for subprocess execution.
/// Always active — prevents leaking secrets (AWS_SECRET_ACCESS_KEY, DATABASE_URL, etc.)

pub fn safe_env() -> Vec<(&'static str, String)> {
    if cfg!(windows) {
        vec![
            ("PATH", r"C:\Windows\system32;C:\Windows;C:\Windows\System32\Wbem".to_string()),
            ("SYSTEMROOT", std::env::var("SYSTEMROOT").unwrap_or_else(|_| r"C:\Windows".to_string())),
        ]
    } else {
        vec![
            ("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string()),
            ("HOME", std::env::var("HOME").unwrap_or_default()),
            ("LANG", "en_US.UTF-8".to_string()),
            ("TERM", "xterm-256color".to_string()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_env_has_path() {
        let env = safe_env();
        assert!(env.iter().any(|(k, v)| *k == "PATH" && !v.is_empty()));
    }

    #[test]
    fn test_safe_env_no_secrets() {
        let keys: Vec<&str> = safe_env().iter().map(|(k, _)| *k).collect();
        for secret in &["AWS_SECRET_ACCESS_KEY", "DATABASE_URL", "NEXUS_AUTH_TOKEN", "GITHUB_TOKEN"] {
            assert!(!keys.contains(secret));
        }
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p nexus-client env`

Expected: 2 tests pass.

---

### Task 10: Create config.rs

- [ ] **Step 1: Write config.rs — runtime config + merge logic**

```rust
use nexus_common::protocol::{FsPolicy, McpServerEntry};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub workspace: PathBuf,
    pub fs_policy: FsPolicy,
    pub shell_timeout: u64,
    pub ssrf_whitelist: Vec<String>,
    pub mcp_servers: Vec<McpServerEntry>,
}

impl ClientConfig {
    pub fn from_login(
        workspace_path: String, fs_policy: FsPolicy, shell_timeout: u64,
        ssrf_whitelist: Vec<String>, mcp_servers: Vec<McpServerEntry>,
    ) -> Self {
        Self { workspace: PathBuf::from(workspace_path), fs_policy, shell_timeout, ssrf_whitelist, mcp_servers }
    }

    /// Merge a ConfigUpdate. Returns true if mcp_servers changed (caller must reinit MCP).
    pub fn merge_update(
        &mut self, fs_policy: Option<FsPolicy>, mcp_servers: Option<Vec<McpServerEntry>>,
        workspace_path: Option<String>, shell_timeout: Option<u64>, ssrf_whitelist: Option<Vec<String>>,
    ) -> bool {
        if let Some(v) = fs_policy { self.fs_policy = v; }
        if let Some(v) = workspace_path { self.workspace = PathBuf::from(v); }
        if let Some(v) = shell_timeout { self.shell_timeout = v; }
        if let Some(v) = ssrf_whitelist { self.ssrf_whitelist = v; }
        if let Some(v) = mcp_servers { self.mcp_servers = v; return true; }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ClientConfig {
        ClientConfig::from_login("/home/u/ws".into(), FsPolicy::Sandbox, 60, vec![], vec![])
    }

    #[test]
    fn test_from_login() {
        let c = cfg();
        assert_eq!(c.workspace, PathBuf::from("/home/u/ws"));
        assert_eq!(c.fs_policy, FsPolicy::Sandbox);
    }

    #[test]
    fn test_merge_partial() {
        let mut c = cfg();
        let mcp = c.merge_update(Some(FsPolicy::Unrestricted), None, None, Some(120), None);
        assert!(!mcp);
        assert_eq!(c.fs_policy, FsPolicy::Unrestricted);
        assert_eq!(c.shell_timeout, 120);
    }

    #[test]
    fn test_merge_mcp_returns_true() {
        let mut c = cfg();
        let mcp = c.merge_update(None, Some(vec![McpServerEntry {
            name: "t".into(), transport_type: None, command: "e".into(),
            args: vec![], env: None, url: None, headers: None, tool_timeout: None, enabled: true,
        }]), None, None, None);
        assert!(mcp);
        assert_eq!(c.mcp_servers.len(), 1);
    }

    #[test]
    fn test_merge_none_preserves() {
        let mut c = cfg();
        c.merge_update(None, None, None, None, None);
        assert_eq!(c.fs_policy, FsPolicy::Sandbox);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p nexus-client config`

Expected: 4 tests pass.

---

### Task 11: Create connection.rs

- [ ] **Step 1: Write connection.rs — WebSocket + auth handshake**

```rust
use crate::config::ClientConfig;
use futures_util::{SinkExt, StreamExt};
use nexus_common::consts::PROTOCOL_VERSION;
use nexus_common::protocol::{ClientToServer, ServerToClient};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info};

pub type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;
pub type WsStream = futures_util::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

pub async fn send_message(sink: &mut WsSink, msg: &ClientToServer) -> Result<(), String> {
    let json = serde_json::to_string(msg).map_err(|e| format!("serialize: {e}"))?;
    sink.send(Message::Text(json.into())).await.map_err(|e| format!("send: {e}"))
}

pub async fn recv_message(stream: &mut WsStream) -> Result<ServerToClient, String> {
    loop {
        match stream.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str::<ServerToClient>(&text)
                    .map_err(|e| format!("deserialize: {e}"));
            }
            Some(Ok(Message::Close(_))) => return Err("connection closed".into()),
            Some(Err(e)) => return Err(format!("ws error: {e}")),
            None => return Err("stream ended".into()),
            _ => continue,
        }
    }
}

pub async fn connect_and_auth(ws_url: &str, token: &str) -> Result<(WsSink, WsStream, ClientConfig), String> {
    info!("Connecting to {ws_url}...");
    let (ws, _) = connect_async(ws_url).await.map_err(|e| format!("connect failed: {e}"))?;
    let (mut sink, mut stream) = ws.split();

    // Receive RequireLogin
    match recv_message(&mut stream).await? {
        ServerToClient::RequireLogin { message } => info!("Server: {message}"),
        other => return Err(format!("Expected RequireLogin, got: {other:?}")),
    }

    // Send SubmitToken
    send_message(&mut sink, &ClientToServer::SubmitToken {
        token: token.to_string(), protocol_version: PROTOCOL_VERSION.to_string(),
    }).await?;
    debug!("Sent SubmitToken");

    // Receive LoginSuccess or LoginFailed
    match recv_message(&mut stream).await? {
        ServerToClient::LoginSuccess {
            user_id, device_name, fs_policy, mcp_servers, workspace_path, shell_timeout, ssrf_whitelist,
        } => {
            info!("Login success: user={user_id}, device={device_name}");
            Ok((sink, stream, ClientConfig::from_login(workspace_path, fs_policy, shell_timeout, ssrf_whitelist, mcp_servers)))
        }
        ServerToClient::LoginFailed { reason } => Err(format!("Login failed: {reason}")),
        other => Err(format!("Expected LoginSuccess/Failed, got: {other:?}")),
    }
}
```

No unit tests — requires real WebSocket server. Tested via integration in Section 7.

- [ ] **Step 2: Verify compiles**

Run: `cargo check -p nexus-client`

---

### Task 12: Create heartbeat.rs

- [ ] **Step 1: Write heartbeat.rs**

```rust
use crate::connection::{send_message, WsSink};
use nexus_common::consts::HEARTBEAT_INTERVAL_SEC;
use nexus_common::protocol::{ClientToServer, DeviceStatus};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, warn};

const MAX_MISSED_ACKS: u32 = 4;

pub struct HeartbeatHandle {
    task: tokio::task::JoinHandle<()>,
}

impl HeartbeatHandle {
    pub fn cancel(self) { self.task.abort(); }
}

pub fn spawn_heartbeat(sink: Arc<Mutex<WsSink>>, missed_acks: Arc<AtomicU32>) -> HeartbeatHandle {
    let task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(HEARTBEAT_INTERVAL_SEC));
        loop {
            interval.tick().await;
            let missed = missed_acks.fetch_add(1, Ordering::SeqCst);
            if missed >= MAX_MISSED_ACKS {
                warn!("Missed {missed} heartbeat acks — connection dead");
                break;
            }
            let msg = ClientToServer::Heartbeat { status: DeviceStatus::Online };
            let mut sink = sink.lock().await;
            if let Err(e) = send_message(&mut sink, &msg).await {
                warn!("Heartbeat send failed: {e}");
                break;
            }
            debug!("Heartbeat sent (missed={missed})");
        }
    });
    HeartbeatHandle { task }
}

pub fn ack_heartbeat(missed_acks: &AtomicU32) {
    missed_acks.store(0, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ack_resets_counter() {
        let counter = AtomicU32::new(3);
        ack_heartbeat(&counter);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p nexus-client heartbeat`

Expected: 1 test passes.

---

### Task 13: Create main.rs (Entry Point + Reconnect Loop)

- [ ] **Step 1: Write main.rs with reconnect loop + message loop**

```rust
mod config;
mod connection;
mod env;
mod heartbeat;

use connection::{recv_message, send_message, WsSink};
use heartbeat::{ack_heartbeat, spawn_heartbeat};
use nexus_common::consts::DEVICE_TOKEN_PREFIX;
use nexus_common::protocol::{ClientToServer, ServerToClient, ToolExecutionResult};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let ws_url = std::env::var("NEXUS_SERVER_WS_URL")
        .or_else(|_| std::env::var("NEXUS_WS_URL"))
        .expect("NEXUS_SERVER_WS_URL or NEXUS_WS_URL must be set");

    let token = std::env::var("NEXUS_AUTH_TOKEN")
        .or_else(|_| std::env::var("NEXUS_DEVICE_TOKEN"))
        .expect("NEXUS_AUTH_TOKEN or NEXUS_DEVICE_TOKEN must be set");

    if !token.starts_with(DEVICE_TOKEN_PREFIX) {
        error!("Token must start with '{DEVICE_TOKEN_PREFIX}'");
        std::process::exit(1);
    }

    info!("NEXUS Client starting...");
    reconnect_loop(&ws_url, &token).await;
}

async fn reconnect_loop(ws_url: &str, token: &str) {
    let mut backoff = 1u64;
    loop {
        match run_session(ws_url, token).await {
            Ok(()) => { info!("Session ended cleanly"); backoff = 1; }
            Err(e) => { warn!("Session error: {e}"); }
        }
        info!("Reconnecting in {backoff}s...");
        tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(30);
    }
}

async fn run_session(ws_url: &str, token: &str) -> Result<(), String> {
    let (sink, mut stream, initial_config) = connection::connect_and_auth(ws_url, token).await?;
    let config = Arc::new(RwLock::new(initial_config));
    let sink = Arc::new(Mutex::new(sink));
    let missed_acks = Arc::new(AtomicU32::new(0));

    // TODO (Section 6): Initialize MCP servers
    // TODO (Section 3): Build tool registry and send RegisterTools

    let hb = spawn_heartbeat(Arc::clone(&sink), Arc::clone(&missed_acks));
    let result = message_loop(&mut stream, &sink, &config, &missed_acks).await;
    hb.cancel();
    result
}

async fn message_loop(
    stream: &mut connection::WsStream,
    sink: &Arc<Mutex<WsSink>>,
    config: &Arc<RwLock<config::ClientConfig>>,
    missed_acks: &Arc<AtomicU32>,
) -> Result<(), String> {
    loop {
        let msg = recv_message(stream).await?;
        match msg {
            ServerToClient::HeartbeatAck => { ack_heartbeat(missed_acks); }
            ServerToClient::ExecuteToolRequest(req) => {
                // TODO (Section 7): dispatch to tool handler
                warn!("Tool execution not yet implemented: {}", req.tool_name);
                let result = ClientToServer::ToolExecutionResult(ToolExecutionResult {
                    request_id: req.request_id, exit_code: 1,
                    output: "Tool execution not yet implemented".into(),
                });
                let mut s = sink.lock().await;
                send_message(&mut s, &result).await?;
            }
            ServerToClient::ConfigUpdate { fs_policy, mcp_servers, workspace_path, shell_timeout, ssrf_whitelist } => {
                let mut cfg = config.write().await;
                let mcp_changed = cfg.merge_update(fs_policy, mcp_servers, workspace_path, shell_timeout, ssrf_whitelist);
                if mcp_changed { info!("MCP servers config changed — reinit needed"); }
            }
            other => { warn!("Unexpected message: {other:?}"); }
        }
    }
}
```

- [ ] **Step 2: Verify build**

Run: `cargo build -p nexus-client`

Expected: Compiles.

- [ ] **Step 3: Commit**

```bash
git add nexus-client/
git commit -m "feat(M1): nexus-client skeleton — connection, auth handshake, heartbeat, config, reconnect loop"
```

---

## Section 3: Tool Infrastructure

**Files:**
- Create: `NEXUS/nexus-client/src/tools/mod.rs`
- Create: `NEXUS/nexus-client/src/tools/helpers.rs`

---

### Task 14: Create tools/helpers.rs

- [ ] **Step 1: Write helpers.rs — path sanitization, output truncation, ignored dirs**

```rust
use crate::config::ClientConfig;
use nexus_common::consts::{MAX_TOOL_OUTPUT_CHARS, TOOL_OUTPUT_HEAD_CHARS, TOOL_OUTPUT_TAIL_CHARS};
use nexus_common::protocol::FsPolicy;
use std::path::{Path, PathBuf};

pub const IGNORED_DIRS: &[&str] = &[
    ".git", "node_modules", "__pycache__", ".venv", "venv",
    "dist", "build", ".tox", ".mypy_cache", ".pytest_cache",
    ".ruff_cache", ".coverage", "htmlcov",
];

pub fn tool_error(msg: &str) -> String {
    format!("Error: {msg}\n\n[Analyze the error and try a different approach.]")
}

pub fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_TOOL_OUTPUT_CHARS { return output.to_string(); }
    let head = &output[..TOOL_OUTPUT_HEAD_CHARS];
    let tail = &output[output.len() - TOOL_OUTPUT_TAIL_CHARS..];
    format!("{head}\n... ({} chars truncated) ...\n{tail}", output.len())
}

pub fn sanitize_path(path: &str, config: &ClientConfig, write: bool) -> Result<PathBuf, String> {
    let expanded = if path.starts_with("~/") || path == "~" {
        let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
        path.replacen('~', &home, 1)
    } else { path.to_string() };

    let abs = if Path::new(&expanded).is_absolute() { PathBuf::from(&expanded) }
    else { config.workspace.join(&expanded) };

    let canonical = if abs.exists() {
        abs.canonicalize().map_err(|e| format!("canonicalize: {e}"))?
    } else {
        let parent = abs.parent().ok_or("no parent")?;
        let name = abs.file_name().ok_or("no filename")?;
        if parent.exists() {
            parent.canonicalize().map_err(|e| format!("canonicalize parent: {e}"))?.join(name)
        } else if write { abs }
        else { return Err(tool_error(&format!("path not found: {path}"))); }
    };

    match config.fs_policy {
        FsPolicy::Sandbox => {
            let s = canonical.to_string_lossy();
            if s == "/dev/null" || s.starts_with("/tmp/nexus") { return Ok(canonical); }
            let ws = config.workspace.canonicalize().unwrap_or_else(|_| config.workspace.clone());
            if !canonical.starts_with(&ws) {
                return Err(tool_error(&format!("path outside workspace in Sandbox mode: {path}")));
            }
        }
        FsPolicy::Unrestricted => {}
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(ws: &str) -> ClientConfig {
        ClientConfig { workspace: PathBuf::from(ws), fs_policy: FsPolicy::Sandbox, shell_timeout: 60, ssrf_whitelist: vec![], mcp_servers: vec![] }
    }

    #[test] fn test_truncate_short() { assert_eq!(truncate_output("hi"), "hi"); }

    #[test] fn test_truncate_long() {
        let long = "x".repeat(20_000);
        let r = truncate_output(&long);
        assert!(r.len() < long.len());
        assert!(r.contains("truncated"));
    }

    #[test] fn test_tool_error_format() { assert!(tool_error("oops").starts_with("Error: ")); }

    #[test] fn test_sanitize_relative() {
        let c = sandbox("/tmp");
        assert!(sanitize_path("test.txt", &c, true).unwrap().starts_with("/tmp"));
    }

    #[test] fn test_sandbox_blocks_outside() {
        let c = sandbox("/tmp/workspace");
        assert!(sanitize_path("/etc/passwd", &c, false).is_err());
    }

    #[test] fn test_sandbox_allows_dev_null() {
        let c = sandbox("/tmp/workspace");
        assert!(sanitize_path("/dev/null", &c, false).is_ok());
    }

    #[test] fn test_sandbox_allows_tmp_nexus() {
        let c = sandbox("/tmp/workspace");
        assert!(sanitize_path("/tmp/nexus_cache", &c, true).is_ok());
    }

    #[test] fn test_unrestricted_allows_all() {
        let c = ClientConfig { workspace: PathBuf::from("/tmp"), fs_policy: FsPolicy::Unrestricted, shell_timeout: 60, ssrf_whitelist: vec![], mcp_servers: vec![] };
        assert!(sanitize_path("/etc/passwd", &c, false).is_ok());
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p nexus-client helpers`

Expected: 8 tests pass.

---

### Task 15: Create tools/mod.rs

- [ ] **Step 1: Write tools/mod.rs — Tool trait, ToolResult, registry, dispatch**

```rust
pub mod helpers;

use crate::config::ClientConfig;
use nexus_common::consts::{EXIT_CODE_ERROR, EXIT_CODE_SUCCESS};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub exit_code: i32,
    pub output: String,
}

impl ToolResult {
    pub fn success(output: impl Into<String>) -> Self { Self { exit_code: EXIT_CODE_SUCCESS, output: output.into() } }
    pub fn error(output: impl Into<String>) -> Self { Self { exit_code: EXIT_CODE_ERROR, output: output.into() } }
    pub fn blocked(output: impl Into<String>) -> Self { Self { exit_code: nexus_common::consts::EXIT_CODE_CANCELLED, output: output.into() } }
    pub fn timeout(output: impl Into<String>) -> Self { Self { exit_code: nexus_common::consts::EXIT_CODE_TIMEOUT, output: output.into() } }
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;
    fn execute(&self, args: Value, config: &ClientConfig) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>>;
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self { Self { tools: HashMap::new() } }

    pub fn register(&mut self, tool: Box<dyn Tool>) { self.tools.insert(tool.name().to_string(), tool); }

    pub async fn dispatch(&self, name: &str, args: Value, config: &ClientConfig) -> ToolResult {
        match self.tools.get(name) {
            Some(tool) => tool.execute(args, config).await,
            None => ToolResult::error(helpers::tool_error(&format!("tool not found: {name}"))),
        }
    }

    pub fn schemas(&self) -> Vec<Value> {
        self.tools.values().map(|t| serde_json::json!({
            "type": "function",
            "function": { "name": t.name(), "description": t.description(), "parameters": t.parameters() }
        })).collect()
    }

    pub fn tool_count(&self) -> usize { self.tools.len() }
}

pub fn register_builtin_tools(_registry: &mut ToolRegistry) {
    // TODO (Section 4-5): register each tool after creation
    // registry.register(Box::new(read_file::ReadFileTool));
    // ... etc
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct DummyTool;
    impl Tool for DummyTool {
        fn name(&self) -> &str { "dummy" }
        fn description(&self) -> &str { "test" }
        fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{},"required":[]}) }
        fn execute(&self, _: Value, _: &ClientConfig) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
            Box::pin(async { ToolResult::success("ok") })
        }
    }

    fn cfg() -> ClientConfig {
        ClientConfig { workspace: PathBuf::from("/tmp"), fs_policy: nexus_common::protocol::FsPolicy::Sandbox, shell_timeout: 60, ssrf_whitelist: vec![], mcp_servers: vec![] }
    }

    #[test] fn test_register_count() {
        let mut r = ToolRegistry::new();
        r.register(Box::new(DummyTool));
        assert_eq!(r.tool_count(), 1);
    }

    #[tokio::test] async fn test_dispatch_found() {
        let mut r = ToolRegistry::new();
        r.register(Box::new(DummyTool));
        let res = r.dispatch("dummy", Value::Null, &cfg()).await;
        assert_eq!(res.exit_code, 0);
    }

    #[tokio::test] async fn test_dispatch_not_found() {
        let r = ToolRegistry::new();
        let res = r.dispatch("missing", Value::Null, &cfg()).await;
        assert_eq!(res.exit_code, 1);
    }

    #[test] fn test_schemas_format() {
        let mut r = ToolRegistry::new();
        r.register(Box::new(DummyTool));
        let s = r.schemas();
        assert_eq!(s[0]["function"]["name"], "dummy");
    }
}
```

- [ ] **Step 2: Add `mod tools;` to main.rs**

- [ ] **Step 3: Run tests**

Run: `cargo test -p nexus-client tools::tests`

Expected: 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add nexus-client/
git commit -m "feat(M1): tool infrastructure — Tool trait, registry, dispatch, path sanitization, output truncation"
```

---

## Section 4: Filesystem Tools

Each tool follows the same pattern: implement `Tool` trait, write tests with `tempfile::tempdir()`, register in `tools/mod.rs`.

**Files:**
- Create: `NEXUS/nexus-client/src/tools/read_file.rs`
- Create: `NEXUS/nexus-client/src/tools/write_file.rs`
- Create: `NEXUS/nexus-client/src/tools/edit_file.rs`
- Create: `NEXUS/nexus-client/src/tools/list_dir.rs`
- Create: `NEXUS/nexus-client/src/tools/glob.rs`
- Create: `NEXUS/nexus-client/src/tools/grep.rs`

---

### Task 16: Create read_file.rs

- [ ] **Step 1: Write read_file.rs**

Per spec section 6.1: line-numbered output (`"{line_num}| {content}"`), image detection (magic bytes), binary rejection, 128K char cap, pagination hints, FsPolicy validation.

```rust
use crate::config::ClientConfig;
use crate::tools::helpers::{sanitize_path, tool_error};
use crate::tools::{Tool, ToolResult};
use nexus_common::consts::{DEFAULT_READ_FILE_LIMIT, MAX_READ_FILE_CHARS};
use nexus_common::mime::detect_mime_from_bytes;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

pub struct ReadFileTool;

impl Tool for ReadFileTool {
    fn name(&self) -> &str { "read_file" }
    fn description(&self) -> &str { "Read file contents with line numbers. Images return metadata." }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{
            "path":{"type":"string","description":"File path"},
            "offset":{"type":"integer","description":"Start line (1-indexed)","default":1},
            "limit":{"type":"integer","description":"Max lines","default":2000}
        },"required":["path"]})
    }
    fn execute(&self, args: Value, config: &ClientConfig) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let config = config.clone();
        Box::pin(async move { exec(args, &config).await })
    }
}

async fn exec(args: Value, config: &ClientConfig) -> ToolResult {
    let path_str = match args.get("path").and_then(Value::as_str) {
        Some(p) => p, None => return ToolResult::error(tool_error("missing: path")),
    };
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(DEFAULT_READ_FILE_LIMIT as u64) as usize;

    let path = match sanitize_path(path_str, config, false) { Ok(p) => p, Err(e) => return ToolResult::error(e) };

    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b, Err(e) => return ToolResult::error(tool_error(&format!("file not found: {path_str}\n{e}"))),
    };

    if let Some(mime) = detect_mime_from_bytes(&bytes) {
        if mime.starts_with("image/") {
            return ToolResult::success(format!("[Image: {path_str}, {}KB]", bytes.len() / 1024));
        }
    }

    let content = match String::from_utf8(bytes) {
        Ok(s) => s, Err(_) => return ToolResult::error(tool_error(&format!("binary file: {path_str}"))),
    };

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let start = (offset - 1).min(total);
    let end = (start + limit).min(total);

    let mut output = String::new();
    for (i, line) in lines[start..end].iter().enumerate() {
        output.push_str(&format!("{}| {line}\n", start + i + 1));
    }

    if output.len() > MAX_READ_FILE_CHARS { output.truncate(MAX_READ_FILE_CHARS); output.push_str("\n... (truncated)"); }
    if end < total { output.push_str(&format!("\nShowing lines {}-{} of {total}. Use offset to read more.", start + 1, end)); }

    ToolResult::success(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_common::protocol::FsPolicy;

    fn cfg(dir: &std::path::Path) -> ClientConfig {
        ClientConfig { workspace: dir.to_path_buf(), fs_policy: FsPolicy::Unrestricted, shell_timeout: 60, ssrf_whitelist: vec![], mcp_servers: vec![] }
    }

    #[tokio::test] async fn test_basic() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("t.txt"), "a\nb\nc\n").unwrap();
        let r = exec(serde_json::json!({"path": d.path().join("t.txt").to_str().unwrap()}), &cfg(d.path())).await;
        assert_eq!(r.exit_code, 0);
        assert!(r.output.contains("1| a") && r.output.contains("3| c"));
    }

    #[tokio::test] async fn test_offset_limit() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("b.txt"), (1..=100).map(|i| format!("line{i}\n")).collect::<String>()).unwrap();
        let r = exec(serde_json::json!({"path": d.path().join("b.txt").to_str().unwrap(), "offset": 50, "limit": 5}), &cfg(d.path())).await;
        assert!(r.output.contains("50| line50") && r.output.contains("Showing lines 50-54"));
    }

    #[tokio::test] async fn test_image() {
        let d = tempfile::tempdir().unwrap();
        let mut data = vec![0x89, b'P', b'N', b'G'];
        data.extend_from_slice(&[0u8; 1000]);
        std::fs::write(d.path().join("i.png"), &data).unwrap();
        let r = exec(serde_json::json!({"path": d.path().join("i.png").to_str().unwrap()}), &cfg(d.path())).await;
        assert!(r.output.contains("[Image:"));
    }

    #[tokio::test] async fn test_not_found() {
        let d = tempfile::tempdir().unwrap();
        let r = exec(serde_json::json!({"path": "/no/such/file"}), &cfg(d.path())).await;
        assert_eq!(r.exit_code, 1);
    }
}
```

- [ ] **Step 2: Add `pub mod read_file;` to tools/mod.rs**

- [ ] **Step 3: Run tests**

Run: `cargo test -p nexus-client read_file`

Expected: 4 tests pass.

---

### Task 17: Create write_file.rs

- [ ] **Step 1: Write write_file.rs**

Per spec section 6.2: create parent dirs, atomic write, return char count.

```rust
use crate::config::ClientConfig;
use crate::tools::helpers::{sanitize_path, tool_error};
use crate::tools::{Tool, ToolResult};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

pub struct WriteFileTool;

impl Tool for WriteFileTool {
    fn name(&self) -> &str { "write_file" }
    fn description(&self) -> &str { "Write content to a file. Creates parent directories." }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{
            "path":{"type":"string"}, "content":{"type":"string"}
        },"required":["path","content"]})
    }
    fn execute(&self, args: Value, config: &ClientConfig) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let config = config.clone();
        Box::pin(async move { exec(args, &config).await })
    }
}

async fn exec(args: Value, config: &ClientConfig) -> ToolResult {
    let p = match args.get("path").and_then(Value::as_str) { Some(p) => p, None => return ToolResult::error(tool_error("missing: path")) };
    let content = match args.get("content").and_then(Value::as_str) { Some(c) => c, None => return ToolResult::error(tool_error("missing: content")) };

    let path = match sanitize_path(p, config, true) { Ok(p) => p, Err(e) => return ToolResult::error(e) };

    if let Some(parent) = path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await { return ToolResult::error(tool_error(&format!("mkdir: {e}"))); }
    }

    match tokio::fs::write(&path, content).await {
        Ok(()) => ToolResult::success(format!("Wrote {} chars to {p}", content.len())),
        Err(e) => ToolResult::error(tool_error(&format!("write: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_common::protocol::FsPolicy;

    fn cfg(d: &std::path::Path) -> ClientConfig {
        ClientConfig { workspace: d.to_path_buf(), fs_policy: FsPolicy::Unrestricted, shell_timeout: 60, ssrf_whitelist: vec![], mcp_servers: vec![] }
    }

    #[tokio::test] async fn test_write() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("o.txt");
        let r = exec(serde_json::json!({"path": f.to_str().unwrap(), "content": "hi"}), &cfg(d.path())).await;
        assert_eq!(r.exit_code, 0);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "hi");
    }

    #[tokio::test] async fn test_creates_dirs() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a/b/c.txt");
        exec(serde_json::json!({"path": f.to_str().unwrap(), "content": "deep"}), &cfg(d.path())).await;
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "deep");
    }
}
```

- [ ] **Step 2: Add `pub mod write_file;` to tools/mod.rs**

- [ ] **Step 3: Run tests**

Run: `cargo test -p nexus-client write_file`

Expected: 2 tests pass.

---

### Task 18: Create edit_file.rs (with Fuzzy Matching)

- [ ] **Step 1: Write edit_file.rs**

Per spec section 6.3: exact match first (0=error, 1=replace, >1=error), then fuzzy match (line-stripped sliding window from nanobot pattern).

```rust
use crate::config::ClientConfig;
use crate::tools::helpers::{sanitize_path, tool_error};
use crate::tools::{Tool, ToolResult};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

pub struct EditFileTool;

impl Tool for EditFileTool {
    fn name(&self) -> &str { "edit_file" }
    fn description(&self) -> &str { "Replace exact text in a file. On 0 matches, tries fuzzy whitespace-tolerant match." }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{
            "file_path":{"type":"string"}, "old_string":{"type":"string"}, "new_string":{"type":"string"}
        },"required":["file_path","old_string","new_string"]})
    }
    fn execute(&self, args: Value, config: &ClientConfig) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let config = config.clone();
        Box::pin(async move { exec(args, &config).await })
    }
}

fn find_match(content: &str, old_text: &str) -> (Option<String>, usize) {
    let content = content.replace("\r\n", "\n");
    let old_text = old_text.replace("\r\n", "\n");

    if content.contains(&old_text) {
        return (Some(old_text.clone()), content.matches(&old_text).count());
    }

    let old_lines: Vec<&str> = old_text.lines().collect();
    if old_lines.is_empty() { return (None, 0); }
    let stripped_old: Vec<String> = old_lines.iter().map(|l| l.trim().to_string()).collect();
    let content_lines: Vec<&str> = content.lines().collect();

    let mut candidates = Vec::new();
    for i in 0..=content_lines.len().saturating_sub(stripped_old.len()) {
        let window = &content_lines[i..i + stripped_old.len()];
        let stripped_win: Vec<String> = window.iter().map(|l| l.trim().to_string()).collect();
        if stripped_win == stripped_old {
            candidates.push(window.join("\n"));
        }
    }

    if candidates.is_empty() { (None, 0) }
    else { let c = candidates.len(); (Some(candidates.into_iter().next().unwrap()), c) }
}

async fn exec(args: Value, config: &ClientConfig) -> ToolResult {
    let p = match args.get("file_path").and_then(Value::as_str) { Some(p) => p, None => return ToolResult::error(tool_error("missing: file_path")) };
    let old = match args.get("old_string").and_then(Value::as_str) { Some(s) if !s.is_empty() => s, _ => return ToolResult::error(tool_error("old_string must be non-empty")) };
    let new = match args.get("new_string").and_then(Value::as_str) { Some(s) => s, None => return ToolResult::error(tool_error("missing: new_string")) };

    let path = match sanitize_path(p, config, true) { Ok(p) => p, Err(e) => return ToolResult::error(e) };
    let content = match tokio::fs::read_to_string(&path).await { Ok(c) => c, Err(e) => return ToolResult::error(tool_error(&format!("read: {e}"))) };

    let (matched, count) = find_match(&content, old);

    match count {
        0 => ToolResult::error(tool_error(&format!("old_string not found in {p}"))),
        1 => {
            let new_content = content.replacen(&matched.unwrap(), new, 1);
            match tokio::fs::write(&path, &new_content).await {
                Ok(()) => ToolResult::success(format!("Edited {p}")),
                Err(e) => ToolResult::error(tool_error(&format!("write: {e}"))),
            }
        }
        n => ToolResult::error(tool_error(&format!("{n} matches in {p} — must be exactly 1"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_common::protocol::FsPolicy;

    fn cfg(d: &std::path::Path) -> ClientConfig {
        ClientConfig { workspace: d.to_path_buf(), fs_policy: FsPolicy::Unrestricted, shell_timeout: 60, ssrf_whitelist: vec![], mcp_servers: vec![] }
    }

    #[test] fn test_find_exact() { let (m, c) = find_match("hello world", "world"); assert_eq!(c, 1); assert_eq!(m.unwrap(), "world"); }
    #[test] fn test_find_multiple() { assert_eq!(find_match("aa bb aa", "aa").1, 2); }
    #[test] fn test_find_fuzzy() {
        let (m, c) = find_match("    fn f() {\n        x();\n    }", "fn f() {\n    x();\n}");
        assert_eq!(c, 1);
        assert!(m.unwrap().contains("        x();"));
    }
    #[test] fn test_find_none() { assert_eq!(find_match("hello", "bye").1, 0); }

    #[tokio::test] async fn test_edit_exact() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("t.rs");
        std::fs::write(&f, "fn main() { old() }").unwrap();
        let r = exec(serde_json::json!({"file_path": f.to_str().unwrap(), "old_string": "old()", "new_string": "new()"}), &cfg(d.path())).await;
        assert_eq!(r.exit_code, 0);
        assert!(std::fs::read_to_string(&f).unwrap().contains("new()"));
    }

    #[tokio::test] async fn test_edit_no_match() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("t.txt");
        std::fs::write(&f, "hello").unwrap();
        let r = exec(serde_json::json!({"file_path": f.to_str().unwrap(), "old_string": "bye", "new_string": "x"}), &cfg(d.path())).await;
        assert_eq!(r.exit_code, 1);
    }

    #[tokio::test] async fn test_edit_multi_match() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("t.txt");
        std::fs::write(&f, "aa bb aa").unwrap();
        let r = exec(serde_json::json!({"file_path": f.to_str().unwrap(), "old_string": "aa", "new_string": "cc"}), &cfg(d.path())).await;
        assert_eq!(r.exit_code, 1);
    }
}
```

- [ ] **Step 2: Add `pub mod edit_file;` to tools/mod.rs**

- [ ] **Step 3: Run tests**

Run: `cargo test -p nexus-client edit_file`

Expected: 7 tests pass.

---

### Task 19: Create list_dir.rs

- [ ] **Step 1: Write list_dir.rs**

Per spec section 6.4: `[DIR]`/`[FILE]` prefixes, recursive mode, alphabetical sort, auto-ignore noise dirs, truncation.

```rust
use crate::config::ClientConfig;
use crate::tools::helpers::{sanitize_path, tool_error, IGNORED_DIRS};
use crate::tools::{Tool, ToolResult};
use nexus_common::consts::DEFAULT_LIST_DIR_MAX;
use serde_json::Value;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

pub struct ListDirTool;

impl Tool for ListDirTool {
    fn name(&self) -> &str { "list_dir" }
    fn description(&self) -> &str { "List directory contents." }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{
            "path":{"type":"string"}, "recursive":{"type":"boolean","default":false},
            "max_entries":{"type":"integer","default":200}
        },"required":["path"]})
    }
    fn execute(&self, args: Value, config: &ClientConfig) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let config = config.clone();
        Box::pin(async move { exec(args, &config).await })
    }
}

async fn exec(args: Value, config: &ClientConfig) -> ToolResult {
    let p = match args.get("path").and_then(Value::as_str) { Some(p) => p, None => return ToolResult::error(tool_error("missing: path")) };
    let recursive = args.get("recursive").and_then(Value::as_bool).unwrap_or(false);
    let max = args.get("max_entries").and_then(Value::as_u64).unwrap_or(DEFAULT_LIST_DIR_MAX as u64) as usize;

    let path = match sanitize_path(p, config, false) { Ok(p) => p, Err(e) => return ToolResult::error(e) };
    if !path.is_dir() { return ToolResult::error(tool_error(&format!("not a directory: {p}"))); }

    let mut entries = Vec::new();
    if recursive { collect_rec(&path, &path, &mut entries, max * 2); }
    else { collect_flat(&path, &mut entries, max * 2); }
    entries.sort();

    let mut out: String = entries.iter().take(max).cloned().collect::<Vec<_>>().join("\n");
    if entries.len() > max { out.push_str(&format!("\n... ({} total, showing {max})", entries.len())); }
    ToolResult::success(out)
}

fn collect_flat(dir: &Path, entries: &mut Vec<String>, max: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if entries.len() >= max { break; }
        let n = e.file_name().to_string_lossy().to_string();
        if IGNORED_DIRS.contains(&n.as_str()) { continue; }
        entries.push(if e.file_type().map(|f| f.is_dir()).unwrap_or(false) { format!("[DIR]  {n}") } else { format!("[FILE] {n}") });
    }
}

fn collect_rec(base: &Path, dir: &Path, entries: &mut Vec<String>, max: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if entries.len() >= max { break; }
        let n = e.file_name().to_string_lossy().to_string();
        if IGNORED_DIRS.contains(&n.as_str()) { continue; }
        let rel = e.path().strip_prefix(base).unwrap_or(&e.path()).to_string_lossy().to_string();
        if e.file_type().map(|f| f.is_dir()).unwrap_or(false) {
            entries.push(format!("{rel}/"));
            collect_rec(base, &e.path(), entries, max);
        } else { entries.push(rel); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_common::protocol::FsPolicy;

    fn cfg(d: &std::path::Path) -> ClientConfig {
        ClientConfig { workspace: d.to_path_buf(), fs_policy: FsPolicy::Unrestricted, shell_timeout: 60, ssrf_whitelist: vec![], mcp_servers: vec![] }
    }

    #[tokio::test] async fn test_flat() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.txt"), "").unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        let r = exec(serde_json::json!({"path": d.path().to_str().unwrap()}), &cfg(d.path())).await;
        assert!(r.output.contains("[FILE] a.txt") && r.output.contains("[DIR]  sub"));
    }

    #[tokio::test] async fn test_recursive() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("s")).unwrap();
        std::fs::write(d.path().join("s/f.txt"), "").unwrap();
        let r = exec(serde_json::json!({"path": d.path().to_str().unwrap(), "recursive": true}), &cfg(d.path())).await;
        assert!(r.output.contains("s/") && r.output.contains("s/f.txt"));
    }

    #[tokio::test] async fn test_ignores_git() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join(".git")).unwrap();
        std::fs::create_dir(d.path().join("src")).unwrap();
        let r = exec(serde_json::json!({"path": d.path().to_str().unwrap()}), &cfg(d.path())).await;
        assert!(!r.output.contains(".git") && r.output.contains("src"));
    }
}
```

- [ ] **Step 2: Add `pub mod list_dir;` to tools/mod.rs**

- [ ] **Step 3: Run tests**

Run: `cargo test -p nexus-client list_dir`

Expected: 3 tests pass.

---

### Task 20: Create glob.rs

- [ ] **Step 1: Write glob.rs**

Per spec section 6.5: match files, sort by mtime newest first, auto-ignore noise dirs.

```rust
use crate::config::ClientConfig;
use crate::tools::helpers::{sanitize_path, tool_error, IGNORED_DIRS};
use crate::tools::{Tool, ToolResult};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

pub struct GlobTool;

impl Tool for GlobTool {
    fn name(&self) -> &str { "glob" }
    fn description(&self) -> &str { "Find files matching a glob pattern, sorted by mtime (newest first)." }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{
            "pattern":{"type":"string"}, "path":{"type":"string","description":"Base dir (default: workspace)"}
        },"required":["pattern"]})
    }
    fn execute(&self, args: Value, config: &ClientConfig) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let config = config.clone();
        Box::pin(async move { exec(args, &config).await })
    }
}

async fn exec(args: Value, config: &ClientConfig) -> ToolResult {
    let pat = match args.get("pattern").and_then(Value::as_str) { Some(p) => p, None => return ToolResult::error(tool_error("missing: pattern")) };
    let base = if let Some(p) = args.get("path").and_then(Value::as_str) {
        match sanitize_path(p, config, false) { Ok(p) => p, Err(e) => return ToolResult::error(e) }
    } else { config.workspace.clone() };

    let full = base.join(pat).to_string_lossy().to_string();
    let paths = match glob::glob(&full) { Ok(p) => p, Err(e) => return ToolResult::error(tool_error(&format!("bad pattern: {e}"))) };

    let mut entries: Vec<(std::time::SystemTime, String)> = Vec::new();
    for entry in paths.flatten() {
        if entry.components().any(|c| IGNORED_DIRS.contains(&c.as_os_str().to_string_lossy().as_ref())) { continue; }
        let mt = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let rel = entry.strip_prefix(&base).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| entry.to_string_lossy().to_string());
        entries.push((mt, rel));
    }
    entries.sort_by(|a, b| b.0.cmp(&a.0));

    if entries.is_empty() { return ToolResult::success("No files matched."); }
    ToolResult::success(entries.iter().map(|(_, p)| p.as_str()).collect::<Vec<_>>().join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_common::protocol::FsPolicy;

    fn cfg(d: &std::path::Path) -> ClientConfig {
        ClientConfig { workspace: d.to_path_buf(), fs_policy: FsPolicy::Unrestricted, shell_timeout: 60, ssrf_whitelist: vec![], mcp_servers: vec![] }
    }

    #[tokio::test] async fn test_glob_rs() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.rs"), "").unwrap();
        std::fs::write(d.path().join("b.txt"), "").unwrap();
        let r = exec(serde_json::json!({"pattern": "*.rs"}), &cfg(d.path())).await;
        assert!(r.output.contains("a.rs") && !r.output.contains("b.txt"));
    }

    #[tokio::test] async fn test_no_matches() {
        let d = tempfile::tempdir().unwrap();
        let r = exec(serde_json::json!({"pattern": "*.xyz"}), &cfg(d.path())).await;
        assert!(r.output.contains("No files"));
    }
}
```

- [ ] **Step 2: Add `pub mod glob;` to tools/mod.rs**

- [ ] **Step 3: Run tests**

Run: `cargo test -p nexus-client glob`

Expected: 2 tests pass.

---

### Task 21: Create grep.rs

- [ ] **Step 1: Write grep.rs**

Per spec section 6.6: regex search, file:line format, include filter, context lines, auto-ignore.

```rust
use crate::config::ClientConfig;
use crate::tools::helpers::{sanitize_path, tool_error, IGNORED_DIRS};
use crate::tools::{Tool, ToolResult};
use regex::Regex;
use serde_json::Value;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

pub struct GrepTool;

impl Tool for GrepTool {
    fn name(&self) -> &str { "grep" }
    fn description(&self) -> &str { "Search file contents with regex." }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{
            "pattern":{"type":"string"}, "path":{"type":"string"},
            "include":{"type":"string","description":"Glob filter"}, "context":{"type":"integer","default":0}
        },"required":["pattern"]})
    }
    fn execute(&self, args: Value, config: &ClientConfig) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let config = config.clone();
        Box::pin(async move { exec(args, &config).await })
    }
}

async fn exec(args: Value, config: &ClientConfig) -> ToolResult {
    let pat = match args.get("pattern").and_then(Value::as_str) { Some(p) => p, None => return ToolResult::error(tool_error("missing: pattern")) };
    let ctx = args.get("context").and_then(Value::as_u64).unwrap_or(0) as usize;
    let include = args.get("include").and_then(Value::as_str);
    let re = match Regex::new(pat) { Ok(r) => r, Err(e) => return ToolResult::error(tool_error(&format!("bad regex: {e}"))) };
    let base = if let Some(p) = args.get("path").and_then(Value::as_str) {
        match sanitize_path(p, config, false) { Ok(p) => p, Err(e) => return ToolResult::error(e) }
    } else { config.workspace.clone() };
    let incl = include.and_then(|p| glob::Pattern::new(p).ok());
    let mut results = Vec::new();
    search_dir(&base, &base, &re, &incl, ctx, &mut results);
    if results.is_empty() { ToolResult::success("No matches found.") } else { ToolResult::success(results.join("\n")) }
}

fn search_dir(base: &Path, dir: &Path, re: &Regex, incl: &Option<glob::Pattern>, ctx: usize, res: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if IGNORED_DIRS.contains(&n.as_str()) { continue; }
        let p = e.path();
        if p.is_dir() { search_dir(base, &p, re, incl, ctx, res); }
        else if p.is_file() {
            if let Some(ref g) = incl { if !g.matches(&n) { continue; } }
            search_file(base, &p, re, ctx, res);
        }
    }
}

fn search_file(base: &Path, path: &Path, re: &Regex, ctx: usize, res: &mut Vec<String>) {
    let Ok(content) = std::fs::read_to_string(path) else { return };
    let lines: Vec<&str> = content.lines().collect();
    let rel = path.strip_prefix(base).unwrap_or(path).to_string_lossy();
    for (i, line) in lines.iter().enumerate() {
        if re.is_match(line) {
            let s = i.saturating_sub(ctx);
            let e = (i + ctx + 1).min(lines.len());
            for j in s..e {
                let pfx = if j == i { ">" } else { " " };
                res.push(format!("{rel}:{}{pfx} {}", j + 1, lines[j]));
            }
            if ctx > 0 { res.push("--".into()); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_common::protocol::FsPolicy;

    fn cfg(d: &std::path::Path) -> ClientConfig {
        ClientConfig { workspace: d.to_path_buf(), fs_policy: FsPolicy::Unrestricted, shell_timeout: 60, ssrf_whitelist: vec![], mcp_servers: vec![] }
    }

    #[tokio::test] async fn test_basic() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("h.txt"), "hello\nbye\nhello again\n").unwrap();
        let r = exec(serde_json::json!({"pattern": "hello"}), &cfg(d.path())).await;
        assert!(r.output.contains("h.txt:1") && r.output.contains("h.txt:3"));
    }

    #[tokio::test] async fn test_include_filter() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.rs"), "fn main").unwrap();
        std::fs::write(d.path().join("b.txt"), "fn main").unwrap();
        let r = exec(serde_json::json!({"pattern": "fn", "include": "*.rs"}), &cfg(d.path())).await;
        assert!(r.output.contains("a.rs") && !r.output.contains("b.txt"));
    }

    #[tokio::test] async fn test_bad_regex() {
        let d = tempfile::tempdir().unwrap();
        let r = exec(serde_json::json!({"pattern": "[invalid"}), &cfg(d.path())).await;
        assert_eq!(r.exit_code, 1);
    }
}
```

- [ ] **Step 2: Add `pub mod grep;` to tools/mod.rs**

- [ ] **Step 3: Run all filesystem tool tests + commit**

Run:
```bash
cargo test -p nexus-client
```

Expected: All tests pass.

```bash
git add nexus-client/
git commit -m "feat(M1): 6 filesystem tools — read_file, write_file, edit_file, list_dir, glob, grep"
```

---

## Section 5: Shell + Guardrails + Sandbox

**Files:**
- Create: `NEXUS/nexus-client/src/guardrails.rs`
- Create: `NEXUS/nexus-client/src/sandbox.rs`
- Create: `NEXUS/nexus-client/src/tools/shell.rs`

---

### Task 22: Create guardrails.rs

- [ ] **Step 1: Write guardrails.rs — deny-list + SSRF + path traversal**

Per spec section 9: regex deny-list (LazyLock), SSRF IP range blocking with per-device whitelist, path traversal detection.

```rust
use regex::Regex;
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::LazyLock;

static DENY_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| vec![
    (Regex::new(r"\brm\s+-[rf]{1,2}\b").unwrap(), "rm -rf/rm -r/rm -f"),
    (Regex::new(r"\bdel\s+/[fq]\b").unwrap(), "del /f or /q"),
    (Regex::new(r"\bformat\s+[a-z]:").unwrap(), "drive format"),
    (Regex::new(r"\bdd\s+if=\b").unwrap(), "dd"),
    (Regex::new(r":\(\)\s*\{.*?\}\s*;\s*:").unwrap(), "fork bomb"),
    (Regex::new(r"\b(shutdown|reboot|poweroff|init\s+0|init\s+6)\b").unwrap(), "shutdown/reboot"),
    (Regex::new(r">\s*/dev/sd[a-z]").unwrap(), "disk write"),
    (Regex::new(r"\b(mkfifo|mknod)\s+/dev/").unwrap(), "device creation"),
]);

static BLOCKED_RANGES: LazyLock<Vec<ipnet::IpNet>> = LazyLock::new(|| {
    ["0.0.0.0/8","10.0.0.0/8","100.64.0.0/10","127.0.0.0/8",
     "169.254.0.0/16","172.16.0.0/12","192.168.0.0/16",
     "::1/128","fc00::/7","fe80::/10"]
        .iter().map(|s| s.parse().unwrap()).collect()
});

static URL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://[^\s'\"]+").unwrap());

pub fn check_deny_list(cmd: &str) -> Option<String> {
    DENY_PATTERNS.iter().find(|(p, _)| p.is_match(cmd)).map(|(_, d)| format!("Blocked: {d}"))
}

pub fn check_path_traversal(cmd: &str) -> Option<String> {
    if cmd.contains("../") || cmd.contains("..\\") { Some("Blocked: path traversal".into()) } else { None }
}

fn is_blocked(ip: &IpAddr, whitelist: &[ipnet::IpNet]) -> bool {
    if whitelist.iter().any(|n| n.contains(ip)) { return false; }
    BLOCKED_RANGES.iter().any(|n| n.contains(ip))
}

pub fn check_ssrf(cmd: &str, whitelist: &[String]) -> Option<String> {
    let wl: Vec<ipnet::IpNet> = whitelist.iter().filter_map(|s| s.parse().ok()).collect();
    for m in URL_RE.find_iter(cmd) {
        let url = m.as_str();
        let host = url.trim_start_matches("http://").trim_start_matches("https://")
            .split('/').next().unwrap_or("").split(':').next().unwrap_or("");
        if host.is_empty() { continue; }
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_blocked(&ip, &wl) { return Some(format!("Blocked: SSRF — {url}")); }
            continue;
        }
        match format!("{host}:80").to_socket_addrs() {
            Ok(addrs) => { for a in addrs { if is_blocked(&a.ip(), &wl) { return Some(format!("Blocked: SSRF — {host} resolves to private IP")); } } }
            Err(_) => return Some(format!("Blocked: SSRF — DNS failed for {host}")),
        }
    }
    None
}

pub fn check_all(cmd: &str, ssrf_whitelist: &[String]) -> Option<String> {
    check_deny_list(cmd).or_else(|| check_path_traversal(cmd)).or_else(|| check_ssrf(cmd, ssrf_whitelist))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn deny_rm_rf() { assert!(check_deny_list("rm -rf /").is_some()); }
    #[test] fn deny_safe_rm() { assert!(check_deny_list("rm file.txt").is_none()); }
    #[test] fn deny_shutdown() { assert!(check_deny_list("shutdown -h now").is_some()); }
    #[test] fn deny_fork_bomb() { assert!(check_deny_list(":() { :|:& }; :").is_some()); }
    #[test] fn traversal_blocks() { assert!(check_path_traversal("cat ../../../etc/passwd").is_some()); }
    #[test] fn traversal_safe() { assert!(check_path_traversal("cat file.txt").is_none()); }
    #[test] fn ssrf_blocks_localhost() { assert!(check_ssrf("curl http://127.0.0.1/", &[]).is_some()); }
    #[test] fn ssrf_blocks_private() { assert!(check_ssrf("curl http://10.0.0.1/", &[]).is_some()); }
    #[test] fn ssrf_allows_public() { assert!(check_ssrf("curl https://api.github.com/", &[]).is_none()); }
    #[test] fn ssrf_whitelist_overrides() { assert!(check_ssrf("curl http://10.0.0.1/", &["10.0.0.0/8".into()]).is_none()); }
    #[test] fn ssrf_blocks_metadata() { assert!(check_ssrf("curl http://169.254.169.254/", &[]).is_some()); }
    #[test] fn check_all_safe() { assert!(check_all("ls -la", &[]).is_none()); }
}
```

- [ ] **Step 2: Add `ipnet = "2"` to nexus-client/Cargo.toml deps + `mod guardrails;` to main.rs**

- [ ] **Step 3: Run tests**

Run: `cargo test -p nexus-client guardrails`

Expected: 12 tests pass.

---

### Task 23: Create sandbox.rs

- [ ] **Step 1: Write sandbox.rs — bwrap wrapper (Linux only)**

Per spec section 10: LazyLock probe, mount layout, --new-session, --die-with-parent, graceful degradation.

```rust
use std::path::Path;
use std::sync::LazyLock;

pub static BWRAP_AVAILABLE: LazyLock<bool> = LazyLock::new(|| {
    if !cfg!(target_os = "linux") { tracing::info!("bwrap: non-Linux"); return false; }
    match std::process::Command::new("bwrap").arg("--version").output() {
        Ok(o) if o.status.success() => { tracing::info!("bwrap: {}", String::from_utf8_lossy(&o.stdout).trim()); true }
        _ => { tracing::warn!("bwrap not found — no sandbox container"); false }
    }
});

pub fn wrap_command(command: &str, workspace: &Path) -> Vec<String> {
    let ws = workspace.to_string_lossy().to_string();
    let parent = workspace.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or("/".into());
    vec![
        "bwrap".into(),
        "--ro-bind".into(), "/usr".into(), "/usr".into(),
        "--ro-bind-try".into(), "/bin".into(), "/bin".into(),
        "--ro-bind-try".into(), "/lib".into(), "/lib".into(),
        "--ro-bind-try".into(), "/lib64".into(), "/lib64".into(),
        "--ro-bind-try".into(), "/etc/alternatives".into(), "/etc/alternatives".into(),
        "--ro-bind-try".into(), "/etc/ssl/certs".into(), "/etc/ssl/certs".into(),
        "--ro-bind-try".into(), "/etc/resolv.conf".into(), "/etc/resolv.conf".into(),
        "--ro-bind-try".into(), "/etc/ld.so.cache".into(), "/etc/ld.so.cache".into(),
        "--proc".into(), "/proc".into(),
        "--dev".into(), "/dev".into(),
        "--tmpfs".into(), "/tmp".into(),
        "--tmpfs".into(), parent,
        "--dir".into(), ws.clone(),
        "--bind".into(), ws.clone(), ws,
        "--new-session".into(),
        "--die-with-parent".into(),
        "--".into(),
        "bash".into(), "-l".into(), "-c".into(),
        format!("'{}'", command.replace('\'', "'\\''")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test] fn test_structure() {
        let c = wrap_command("echo hi", &PathBuf::from("/home/u/ws"));
        assert_eq!(c[0], "bwrap");
        assert!(c.contains(&"--new-session".into()) && c.contains(&"--die-with-parent".into()));
    }

    #[test] fn test_workspace_bind() {
        let c = wrap_command("ls", &PathBuf::from("/home/u/ws"));
        let i = c.iter().position(|a| a == "--bind").unwrap();
        assert_eq!(c[i+1], "/home/u/ws");
    }
}
```

- [ ] **Step 2: Add `mod sandbox;` to main.rs**

- [ ] **Step 3: Run tests**

Run: `cargo test -p nexus-client sandbox`

Expected: 2 tests pass.

---

### Task 24: Create tools/shell.rs

- [ ] **Step 1: Write shell.rs**

Per spec section 6.7 + section 9-10: guardrails check (sandbox only), bwrap if available, env isolation always, timeout, output truncation, stderr prefix.

```rust
use crate::config::ClientConfig;
use crate::env::safe_env;
use crate::guardrails;
use crate::sandbox;
use crate::tools::helpers::{tool_error, truncate_output};
use crate::tools::{Tool, ToolResult};
use nexus_common::consts::DEFAULT_SHELL_TIMEOUT_SEC;
use nexus_common::protocol::FsPolicy;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use tokio::process::Command;

pub struct ShellTool;

impl Tool for ShellTool {
    fn name(&self) -> &str { "shell" }
    fn description(&self) -> &str { "Execute a shell command." }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{
            "command":{"type":"string"}, "timeout_sec":{"type":"integer"},
            "working_dir":{"type":"string"}
        },"required":["command"]})
    }
    fn execute(&self, args: Value, config: &ClientConfig) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let config = config.clone();
        Box::pin(async move { exec(args, &config).await })
    }
}

async fn exec(args: Value, config: &ClientConfig) -> ToolResult {
    let command = match args.get("command").and_then(Value::as_str) { Some(c) => c, None => return ToolResult::error(tool_error("missing: command")) };
    let timeout_sec = args.get("timeout_sec").and_then(Value::as_u64).unwrap_or(config.shell_timeout.max(DEFAULT_SHELL_TIMEOUT_SEC));
    let wd = args.get("working_dir").and_then(Value::as_str).map(std::path::PathBuf::from).unwrap_or_else(|| config.workspace.clone());

    if config.fs_policy == FsPolicy::Sandbox {
        if let Some(reason) = guardrails::check_all(command, &config.ssrf_whitelist) {
            return ToolResult::blocked(reason);
        }
    }

    let mut cmd = if config.fs_policy == FsPolicy::Sandbox && *sandbox::BWRAP_AVAILABLE {
        let a = sandbox::wrap_command(command, &config.workspace);
        let mut c = Command::new(&a[0]); c.args(&a[1..]); c
    } else if cfg!(windows) {
        let mut c = Command::new("cmd"); c.args(["/C", command]); c
    } else {
        let mut c = Command::new("bash"); c.args(["-l", "-c", command]); c
    };

    cmd.env_clear();
    for (k, v) in safe_env() { cmd.env(k, v); }
    cmd.current_dir(&wd);

    match tokio::time::timeout(std::time::Duration::from_secs(timeout_sec), cmd.output()).await {
        Ok(Ok(out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let code = out.status.code().unwrap_or(1);
            let mut text = stdout.to_string();
            if !stderr.is_empty() { if !text.is_empty() { text.push('\n'); } text.push_str("STDERR:\n"); text.push_str(&stderr); }
            text.push_str(&format!("\nExit code: {code}"));
            ToolResult { exit_code: code, output: truncate_output(&text) }
        }
        Ok(Err(e)) => ToolResult::error(tool_error(&format!("exec failed: {e}"))),
        Err(_) => ToolResult::timeout(format!("Timed out after {timeout_sec}s: {command}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ucfg(d: &std::path::Path) -> ClientConfig {
        ClientConfig { workspace: d.to_path_buf(), fs_policy: FsPolicy::Unrestricted, shell_timeout: 60, ssrf_whitelist: vec![], mcp_servers: vec![] }
    }
    fn scfg(d: &std::path::Path) -> ClientConfig {
        ClientConfig { workspace: d.to_path_buf(), fs_policy: FsPolicy::Sandbox, shell_timeout: 60, ssrf_whitelist: vec![], mcp_servers: vec![] }
    }

    #[tokio::test] async fn test_echo() {
        let d = tempfile::tempdir().unwrap();
        let r = exec(serde_json::json!({"command": "echo hello"}), &ucfg(d.path())).await;
        assert_eq!(r.exit_code, 0); assert!(r.output.contains("hello"));
    }

    #[tokio::test] async fn test_exit_code() {
        let d = tempfile::tempdir().unwrap();
        let r = exec(serde_json::json!({"command": "exit 42"}), &ucfg(d.path())).await;
        assert_eq!(r.exit_code, 42);
    }

    #[tokio::test] async fn test_stderr() {
        let d = tempfile::tempdir().unwrap();
        let r = exec(serde_json::json!({"command": "echo err >&2"}), &ucfg(d.path())).await;
        assert!(r.output.contains("STDERR:") && r.output.contains("err"));
    }

    #[tokio::test] async fn test_timeout() {
        let d = tempfile::tempdir().unwrap();
        let r = exec(serde_json::json!({"command": "sleep 10", "timeout_sec": 1}), &ucfg(d.path())).await;
        assert_eq!(r.exit_code, nexus_common::consts::EXIT_CODE_TIMEOUT);
    }

    #[tokio::test] async fn test_sandbox_blocks_rm() {
        let d = tempfile::tempdir().unwrap();
        let r = exec(serde_json::json!({"command": "rm -rf /"}), &scfg(d.path())).await;
        assert_eq!(r.exit_code, nexus_common::consts::EXIT_CODE_CANCELLED);
    }

    #[tokio::test] async fn test_env_isolation() {
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("TEST_SECRET_99", "leaked");
        let r = exec(serde_json::json!({"command": "echo $TEST_SECRET_99"}), &ucfg(d.path())).await;
        assert!(!r.output.contains("leaked"));
        std::env::remove_var("TEST_SECRET_99");
    }
}
```

- [ ] **Step 2: Add `pub mod shell;` to tools/mod.rs**

- [ ] **Step 3: Run tests + commit**

Run: `cargo test -p nexus-client shell`

Expected: 6 tests pass.

```bash
git add nexus-client/
git commit -m "feat(M1): shell tool + guardrails (deny-list, SSRF, traversal) + bwrap sandbox"
```

---

## Section 6: MCP Client

**Files:**
- Create: `NEXUS/nexus-client/src/mcp/mod.rs`
- Create: `NEXUS/nexus-client/src/mcp/client.rs`

---

### Task 25: Create mcp/client.rs

- [ ] **Step 1: Write mcp/client.rs — single MCP server session**

Per spec section 11: spawn child process, MCP initialize, tools/list, prefixed schemas, tool call forwarding with timeout.

```rust
use nexus_common::consts::DEFAULT_MCP_TOOL_TIMEOUT_SEC;
use nexus_common::mcp_utils::normalize_schema_for_openai;
use nexus_common::protocol::McpServerEntry;
use rmcp::model::Tool as McpTool;
use rmcp::service::RunningService;
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::RoleClient;
use serde_json::Value;
use tracing::{error, info};

pub struct McpSession {
    pub server_name: String,
    pub tool_timeout: u64,
    service: RunningService<RoleClient, TokioChildProcess>,
    tools: Vec<McpTool>,
}

impl McpSession {
    pub async fn start(entry: &McpServerEntry) -> Result<Self, String> {
        info!("Starting MCP server: {}", entry.name);
        let mut cmd = tokio::process::Command::new(&entry.command);
        cmd.args(&entry.args);
        if let Some(ref env) = entry.env { for (k, v) in env { cmd.env(k, v); } }
        cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());

        let child = TokioChildProcess::new(&mut cmd).map_err(|e| format!("spawn '{}': {e}", entry.name))?;
        let service = rmcp::ServiceExt::serve(child).await.map_err(|e| format!("init '{}': {e}", entry.name))?;
        let tools = service.list_tools(None).await.map_err(|e| format!("tools/list '{}': {e}", entry.name))?.tools;
        info!("MCP '{}': {} tools", entry.name, tools.len());

        Ok(Self { server_name: entry.name.clone(), tool_timeout: entry.tool_timeout.unwrap_or(DEFAULT_MCP_TOOL_TIMEOUT_SEC), service, tools })
    }

    pub fn tool_schemas(&self) -> Vec<Value> {
        self.tools.iter().map(|t| {
            let name = format!("mcp_{}_{}", self.server_name, t.name);
            let desc = t.description.as_deref().unwrap_or("MCP tool");
            let params = t.input_schema.as_ref()
                .map(|s| normalize_schema_for_openai(&serde_json::to_value(s).unwrap_or_default()))
                .unwrap_or_else(|| serde_json::json!({"type":"object","properties":{},"required":[]}));
            serde_json::json!({"type":"function","function":{"name":name,"description":desc,"parameters":params}})
        }).collect()
    }

    pub async fn call_tool(&self, tool_name: &str, args: Value) -> Result<String, String> {
        let timeout = std::time::Duration::from_secs(self.tool_timeout);
        let result = tokio::time::timeout(timeout, self.service.call_tool(tool_name, args))
            .await.map_err(|_| format!("MCP tool '{tool_name}' timed out after {}s", self.tool_timeout))?
            .map_err(|e| format!("MCP call failed: {e}"))?;
        Ok(result.content.iter().filter_map(|item| match item {
            rmcp::model::Content::Text(t) => Some(t.text.clone()),
            _ => None,
        }).collect::<Vec<_>>().join("\n"))
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tools.iter().map(|t| t.name.to_string()).collect()
    }
}
```

No unit tests — requires real MCP server binary. Integration testing is manual.

---

### Task 26: Create mcp/mod.rs

- [ ] **Step 1: Write mcp/mod.rs — MCP manager**

Per spec section 11.2: lifecycle, config diff, reinit, call routing.

```rust
pub mod client;

use client::McpSession;
use nexus_common::protocol::McpServerEntry;
use serde_json::Value;
use std::collections::HashMap;
use tracing::{error, info};

pub struct McpManager {
    sessions: HashMap<String, McpSession>,
}

impl McpManager {
    pub fn new() -> Self { Self { sessions: HashMap::new() } }

    pub async fn initialize(&mut self, servers: &[McpServerEntry]) {
        for entry in servers.iter().filter(|e| e.enabled) {
            match McpSession::start(entry).await {
                Ok(s) => { self.sessions.insert(entry.name.clone(), s); }
                Err(e) => { error!("MCP '{}': {e}", entry.name); }
            }
        }
    }

    pub async fn apply_config(&mut self, new: &[McpServerEntry]) {
        let current: Vec<String> = self.sessions.keys().cloned().collect();
        let new_names: Vec<&str> = new.iter().map(|s| s.name.as_str()).collect();
        for name in &current { if !new_names.contains(&name.as_str()) { self.sessions.remove(name); } }
        for entry in new {
            if !entry.enabled { self.sessions.remove(&entry.name); continue; }
            self.sessions.remove(&entry.name);
            match McpSession::start(entry).await {
                Ok(s) => { self.sessions.insert(entry.name.clone(), s); }
                Err(e) => { error!("MCP restart '{}': {e}", entry.name); }
            }
        }
    }

    pub fn all_tool_schemas(&self) -> Vec<Value> {
        self.sessions.values().flat_map(|s| s.tool_schemas()).collect()
    }

    pub async fn call_tool(&self, prefixed: &str, args: Value) -> Result<String, String> {
        let rest = prefixed.strip_prefix("mcp_").ok_or_else(|| format!("not MCP: {prefixed}"))?;
        for (name, session) in &self.sessions {
            if let Some(tool) = rest.strip_prefix(&format!("{name}_")) {
                return session.call_tool(tool, args).await;
            }
        }
        Err(format!("MCP server not found for: {prefixed}"))
    }

    pub fn is_mcp_tool(name: &str) -> bool { name.starts_with("mcp_") }
    pub fn session_count(&self) -> usize { self.sessions.len() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn test_is_mcp_tool() { assert!(McpManager::is_mcp_tool("mcp_gh_list")); assert!(!McpManager::is_mcp_tool("read_file")); }
    #[test] fn test_empty_manager() { let m = McpManager::new(); assert_eq!(m.session_count(), 0); }
}
```

- [ ] **Step 2: Add `mod mcp;` to main.rs**

- [ ] **Step 3: Run tests + commit**

Run: `cargo test -p nexus-client mcp`

Expected: 2 tests pass.

```bash
git add nexus-client/
git commit -m "feat(M1): MCP client — session lifecycle, tool schema collection, call routing"
```

---

## Section 7: Tool Registration + Integration Wiring

**Files:**
- Modify: `NEXUS/nexus-client/src/tools/mod.rs`
- Modify: `NEXUS/nexus-client/src/main.rs`

---

### Task 27: Wire Up Tool Registration

- [ ] **Step 1: Update register_builtin_tools in tools/mod.rs**

Replace the TODO body:

```rust
pub fn register_builtin_tools(registry: &mut ToolRegistry) {
    registry.register(Box::new(read_file::ReadFileTool));
    registry.register(Box::new(write_file::WriteFileTool));
    registry.register(Box::new(edit_file::EditFileTool));
    registry.register(Box::new(list_dir::ListDirTool));
    registry.register(Box::new(glob::GlobTool));
    registry.register(Box::new(grep::GrepTool));
    registry.register(Box::new(shell::ShellTool));
}
```

- [ ] **Step 2: Add test**

```rust
#[test] fn test_builtin_count() {
    let mut r = ToolRegistry::new();
    register_builtin_tools(&mut r);
    assert_eq!(r.tool_count(), 7);
}
```

- [ ] **Step 3: Run test**

Run: `cargo test -p nexus-client test_builtin_count`

Expected: PASS (7 tools).

---

### Task 28: Wire Up MCP + Dispatch in main.rs

- [ ] **Step 1: Update run_session to initialize MCP + register tools**

Replace the TODO section in `run_session`:

```rust
async fn run_session(ws_url: &str, token: &str) -> Result<(), String> {
    let (sink, mut stream, initial_config) = connection::connect_and_auth(ws_url, token).await?;
    let config = Arc::new(RwLock::new(initial_config));
    let sink = Arc::new(Mutex::new(sink));
    let missed_acks = Arc::new(AtomicU32::new(0));

    let mcp_manager = Arc::new(Mutex::new(mcp::McpManager::new()));
    { let cfg = config.read().await; mcp_manager.lock().await.initialize(&cfg.mcp_servers).await; }

    let mut registry = tools::ToolRegistry::new();
    tools::register_builtin_tools(&mut registry);
    let registry = Arc::new(registry);

    { // Send RegisterTools
        let mut schemas = registry.schemas();
        schemas.extend(mcp_manager.lock().await.all_tool_schemas());
        let mut s = sink.lock().await;
        send_message(&mut s, &nexus_common::protocol::ClientToServer::RegisterTools { schemas }).await?;
    }

    let hb = spawn_heartbeat(Arc::clone(&sink), Arc::clone(&missed_acks));
    let result = message_loop(&mut stream, &sink, &config, &missed_acks, &registry, &mcp_manager).await;
    hb.cancel();
    result
}
```

- [ ] **Step 2: Update message_loop to dispatch tools via registry + MCP**

```rust
async fn message_loop(
    stream: &mut connection::WsStream, sink: &Arc<Mutex<WsSink>>,
    config: &Arc<RwLock<config::ClientConfig>>, missed_acks: &Arc<AtomicU32>,
    registry: &Arc<tools::ToolRegistry>, mcp_manager: &Arc<Mutex<mcp::McpManager>>,
) -> Result<(), String> {
    loop {
        let msg = recv_message(stream).await?;
        match msg {
            ServerToClient::HeartbeatAck => { ack_heartbeat(missed_acks); }
            ServerToClient::ExecuteToolRequest(req) => {
                let sink = Arc::clone(sink);
                let config = Arc::clone(config);
                let registry = Arc::clone(registry);
                let mcp_mgr = Arc::clone(mcp_manager);
                tokio::spawn(async move {
                    let result = if mcp::McpManager::is_mcp_tool(&req.tool_name) {
                        match mcp_mgr.lock().await.call_tool(&req.tool_name, req.arguments).await {
                            Ok(out) => tools::ToolResult::success(out),
                            Err(e) => tools::ToolResult::error(e),
                        }
                    } else {
                        let cfg = config.read().await;
                        registry.dispatch(&req.tool_name, req.arguments, &cfg).await
                    };
                    let msg = ClientToServer::ToolExecutionResult(ToolExecutionResult {
                        request_id: req.request_id, exit_code: result.exit_code, output: result.output,
                    });
                    if let Err(e) = send_message(&mut *sink.lock().await, &msg).await {
                        tracing::warn!("send result failed: {e}");
                    }
                });
            }
            ServerToClient::ConfigUpdate { fs_policy, mcp_servers, workspace_path, shell_timeout, ssrf_whitelist } => {
                let mut cfg = config.write().await;
                let mcp_changed = cfg.merge_update(fs_policy, mcp_servers.clone(), workspace_path, shell_timeout, ssrf_whitelist);
                if mcp_changed {
                    if let Some(new_servers) = mcp_servers {
                        let mut mgr = mcp_manager.lock().await;
                        mgr.apply_config(&new_servers).await;
                        let mut schemas = registry.schemas();
                        schemas.extend(mgr.all_tool_schemas());
                        let _ = send_message(&mut *sink.lock().await,
                            &ClientToServer::RegisterTools { schemas }).await;
                    }
                }
            }
            other => { warn!("Unexpected: {other:?}"); }
        }
    }
}
```

- [ ] **Step 3: Build + test full workspace**

Run:
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

Expected: All pass.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(M1): wire up tool registration, MCP dispatch, and full message loop"
```

---

## Plan Complete

| Section | Tasks | What |
|---|---|---|
| 1 | 1-7 | Workspace + nexus-common from scratch |
| 2 | 8-13 | Client skeleton (connection, heartbeat, config, reconnect) |
| 3 | 14-15 | Tool infrastructure (trait, registry, helpers) |
| 4 | 16-21 | 6 filesystem tools |
| 5 | 22-24 | Shell + guardrails + bwrap sandbox |
| 6 | 25-26 | MCP client |
| 7 | 27-28 | Integration wiring |

**Test coverage:** ~70 tests across both crates.

**Execution options:**

1. **Subagent-Driven (recommended)** — Fresh subagent per task, review between tasks
2. **Inline Execution** — Execute tasks in this session with checkpoints

Which approach?
