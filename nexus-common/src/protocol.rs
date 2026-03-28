/// 职责边界：
/// 1. 仅包含 Server 和 Client 之间 WebSocket 通信的序列化结构体 (Structs/Enums)。
/// 2. 绝对不能包含任何业务逻辑。
///
/// 参考 nanobot：
/// - 这里的结构体设计应该替代 `nanobot/bus/queue.py` 中的内部消息传递机制，将其网络化。
/// - Tool 请求的 Payload 结构可以参考 `nanobot/agent/tools/base.py` 中的 `ToolCallRequest`。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Skill 全量信息，用于 Client → Server 注册。
///
/// - always=false: content = None（服务端只有摘要，正文由 Agent 自行 read_file）
/// - always=true:  content = Some(正文)（服务端存储，用于注入 system prompt）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFull {
    pub name: String,
    pub description: String,
    pub always: bool,
    /// always=true 时为 Some(正文)，always=false 时为 None
    pub content: Option<String>,
}

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

    /// 新增：AgentLoop 通过 bus 推送的响应
    AgentResponse {
        channel: String,
        chat_id: String,
        content: String,
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
        /// 工具 Schema 列表（内置工具 + MCP 工具）
        schemas: Vec<Value>,
        /// Skill 全量列表。
        /// always=false: content=None（服务端只存摘要）
        /// always=true:  content=Some(正文)（服务端存储，用于注入 system prompt）
        skills: Vec<SkillFull>,
    },

    /// 新增：心跳包，带着当前工具的 Hash，防止 Server 和 Client 状态脱节。
    /// hash 是对【内置工具 + MCP 工具 Schema 列表 + 所有 Skill 的 name/description/content/always】计算的统一哈希。
    /// Server 可通过比对上次记录的 hash 来判断工具集是否发生变更。
    Heartbeat {
        device_id: String,
        device_name: String,
        hash: String,
        /// 合法值："online" | "busy"。M1 阶段 Client 只发送 "online"。
        status: String,
    },

    /// 新增：模拟 Channel 消息，触发 AgentLoop 处理
    UserMessage {
        session_id: String,
        channel: String,
        content: String,
    },
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
