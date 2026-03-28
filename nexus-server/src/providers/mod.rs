use serde_json::Value;

pub mod mock;
pub use mock::{chat_completion, ChatCompletionRequest, ChatCompletionResponse};

/// LLM 标准返回结构（跨 provider 统一）
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCallRequest>,
    pub finish_reason: String,
}

/// 单次工具调用请求（由 LLM 生成）
#[derive(Debug, Clone)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}