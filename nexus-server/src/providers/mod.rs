/// 职责边界：
/// 1. 定义 `LlmProvider` Trait，使用 `async-openai` 库调用本地 LM Studio。
/// 2. 【核心难点】处理大模型经常输出破损 JSON Arguments 的问题。
///
/// 参考 nanobot：
/// - 对应 `nanobot/providers/litellm_provider.py`。
/// - 参考它的 `json_repair` 逻辑。在 Rust 中，我们可以在反序列化失败时，
///   尝试手动补齐括号或使用宽松的 JSON 解析库容错。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// LLM 标准返回结构（跨 provider 统一）。
#[derive(Debug, Clone)]
pub struct LlmResponse {
    /// LLM 的文本输出（当 finish_reason = "stop" 时有内容）
    pub content: Option<String>,
    /// LLM 决定调用的工具列表
    pub tool_calls: Vec<ToolCallRequest>,
    /// "stop" | "tool_calls" | "error"
    pub finish_reason: String,
}

/// 单次工具调用请求（由 LLM 生成）。
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallRequest {
    /// LLM 生成的 tool_call_id，用于 tool_result 对应
    pub id: String,
    /// 工具名称（对应 RegisterTools 中的 schema name）
    pub name: String,
    /// 工具参数（已经过 json_repair 处理）
    pub arguments: Value,
}

#[cfg(feature = "mock")]
pub mod mock {
    use super::*;
    use async_trait::async_trait;

    pub struct MockProvider;

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn chat(
            &self,
            _messages: Vec<Value>,
            _tools: Vec<Value>,
            _model: &str,
        ) -> LlmResponse {
            LlmResponse {
                content: Some("Mock response".to_string()),
                tool_calls: Vec::new(),
                finish_reason: "stop".to_string(),
            }
        }
    }
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(
        &self,
        messages: Vec<Value>,
        tools: Vec<Value>,
        model: &str,
    ) -> LlmResponse;
}
