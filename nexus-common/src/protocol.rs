/// 职责边界：
/// 1. 仅包含 Server 和 Client 之间 WebSocket 通信的序列化结构体 (Structs/Enums)。
/// 2. 绝对不能包含任何业务逻辑。
///
/// 参考 nanobot：
/// - 这里的结构体设计应该替代 `nanobot/bus/queue.py` 中的内部消息传递机制，将其网络化。
/// - Tool 请求的 Payload 结构可以参考 `nanobot/agent/tools/base.py` 中的 `ToolCallRequest`。

use serde::{Deserialize, Serialize};
use serde_json::Value;

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

/// 服务端下发给客户端的指令
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

/// 客户端上报给服务端的事件
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
        status: String,
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
    /// exit_code 语义约定（Client 和 Server 必须遵守此规范）：
    ///   0  — 执行成功
    ///   1  — 执行失败（stderr 或业务错误，output 中包含错误详情）
    ///  -1  — 执行超时（被 tokio::time::timeout kill）
    ///  -2  — 被取消（设备断线或 cancel_pending_requests_for_device 触发）
    ///  -3  — 参数校验失败（executor.rs guardrails 或 schema 校验拦截，未执行）
    /// 参考 nanobot：nanobot/agent/tools/shell.py ExecTool.run() 返回值语义
    pub exit_code: i32,
    pub output: String,
}
