/// 职责边界：
/// 1. 仅包含 Server 和 Client 之间 WebSocket 通信的序列化结构体 (Structs/Enums)。
/// 2. 绝对不能包含任何业务逻辑。
///
/// 参考 nanobot：
/// - 这里的结构体设计应该替代 `nanobot/bus/queue.py` 中的内部消息传递机制，将其网络化。
/// - Tool 请求的 Payload 结构可以参考 `nanobot/agent/tools/base.py` 中的 `ToolCallRequest`。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 服务端下发给客户端的指令
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ServerToClient {
    ExecuteToolRequest(ExecuteToolRequest),
    RequireLogin {
        message: String,
    },
    LoginSuccess {
        user_id: String,
        device_id: String,
    },
    LoginFailed {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolRequest {
    pub request_id: String,
    pub tool_name: String,
    pub arguments: Value,
}

/// 客户端上报给服务端的事件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ClientToServer {
    ToolStdoutStream(ToolStdoutStream),
    ToolExecutionResult(ToolExecutionResult),
    SubmitToken {
        token: String,
        device_id: String,
        device_name: String,
        /// 客户端声明的协议版本，Server 应校验是否等于 consts::PROTOCOL_VERSION，不匹配则拒绝登录。
        protocol_version: String,
    },
    
    /// 新增：客户端连上 MCP 或启动时，主动向 Server 上报当前可用的所有工具 Schema
    RegisterTools {
        device_id: String,
        device_name: String,
        schemas: Vec<Value>, // 存放 JSON Schema
    },

    /// 新增：心跳包，带着当前工具的 Hash，防止 Server 和 Client 状态脱节。
    /// tools_hash 是对【内置工具 + MCP 工具 + Skill 工具】三类工具 Schema 列表
    /// 合并后整体计算的哈希值（例如对 Vec<Value> 序列化后做 SHA256）。
    /// Server 可通过比对上次记录的 hash 来判断工具集是否发生变更。
    Heartbeat {
        device_id: String,
        device_name: String,
        tools_hash: String,
        /// 合法值："online" | "busy"。M1 阶段 Client 只发送 "online"。
        status: String,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStdoutStream {
    pub request_id: String,
    pub chunk_data: String,
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
