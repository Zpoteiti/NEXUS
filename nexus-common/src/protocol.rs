/// Responsibility boundary:
/// 1. Contains only serialization structs/enums for Server-Client WebSocket communication.
/// 2. Must never contain any business logic.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Device status reported in heartbeat messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceStatus {
    #[serde(rename = "online")]
    Online,
    #[serde(rename = "offline")]
    Offline,
}

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
    FileUploadRequest(FileUploadRequest),
    RequireLogin {
        message: String,
    },
    LoginSuccess {
        user_id: String,
        device_name: String,
        fs_policy: FsPolicy,
        mcp_servers: Vec<McpServerEntry>,
    },
    LoginFailed {
        reason: String,
    },
    HeartbeatAck {
        fs_policy: FsPolicy,
        mcp_servers: Vec<McpServerEntry>,
    },
    FileDownloadRequest {
        request_id: String,
        file_name: String,
        content_base64: String,
        destination_path: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolRequest {
    pub request_id: String,
    pub tool_name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileUploadRequest {
    pub request_id: String,
    pub file_path: String,
}

/// Events reported from client to server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ClientToServer {
    ToolExecutionResult(ToolExecutionResult),
    FileUploadResponse(FileUploadResponse),
    SubmitToken {
        token: String,
        protocol_version: String,
    },

    RegisterTools {
        schemas: Vec<Value>,
    },

    Heartbeat {
        hash: String,
        status: DeviceStatus,
    },
    FileDownloadResponse(FileDownloadResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileUploadResponse {
    pub request_id: String,
    pub file_name: String,
    pub content_base64: String,
    pub mime_type: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDownloadResponse {
    pub request_id: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecutionResult {
    pub request_id: String,
    /// exit_code semantic conventions (Client and Server must follow this spec):
    ///   0  -- execution succeeded
    ///   1  -- execution failed (stderr or business error, details in output)
    ///  -1  -- execution timed out (killed by tokio::time::timeout)
    ///  -2  -- cancelled (device disconnected or cancel_pending_requests_for_device triggered)
    ///  -3  -- validation failed (executor.rs guardrails or schema validation blocked, not executed)
    pub exit_code: i32,
    pub output: String,
}
