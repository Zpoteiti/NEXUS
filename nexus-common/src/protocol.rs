/// 职责边界：
/// 1. 仅包含 Server 和 Client 之间 WebSocket 通信的序列化结构体 (Structs/Enums)。
/// 2. 绝对不能包含任何业务逻辑。
///
/// 参考 nanobot：
/// - 这里的结构体设计应该替代 `nanobot/bus/queue.py` 中的内部消息传递机制，将其网络化。
/// - Tool 请求的 Payload 结构可以参考 `nanobot/agent/tools/base.py` 中的 `ToolCallRequest`。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 通用的 WebSocket 消息信封
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NexusMessage<T> {
    pub message_id: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub payload: T,
}

/// 服务端下发给客户端的指令
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ServerToClient {
    ExecuteToolRequest(ExecuteToolRequest),
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
    
    /// 新增：客户端连上 MCP 或启动时，主动向 Server 上报当前可用的所有工具 Schema
    RegisterTools {
        device_id: String,
        schemas: Vec<Value>, // 存放 JSON Schema
    },
    
    /// 新增：心跳包，带着当前工具的 Hash，防止 Server 和 Client 状态脱节。
    /// tools_hash 是对【内置工具 + MCP 工具 + Skill 工具】三类工具 Schema 列表
    /// 合并后整体计算的哈希值（例如对 Vec<Value> 序列化后做 SHA256）。
    /// Server 可通过比对上次记录的 hash 来判断工具集是否发生变更。
    Heartbeat {
        device_id: String,
        tools_hash: String,
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
    pub exit_code: i32,
    pub output: String,
}