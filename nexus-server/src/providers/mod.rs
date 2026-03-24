/// 职责边界：
/// 1. 定义 `LlmProvider` Trait，包含 `chat_with_retry` 方法。
/// 2. 负责统一定义模型返回的内部标准结构：`LlmResponse` 和 `ToolCallRequest`。
///
/// 参考 nanobot：
/// - 对应 `nanobot/providers/base.py`。
/// - 将千奇百怪的 LLM API 统一抽象隔离，保护 agent_loop.rs 不受具体厂商数据格式的污染。
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【chat_with_retry 行为契约】
/// ─────────────────────────────────────────────────────────────────────────────
/// 所有实现 LlmProvider 的结构体（如 openai.rs 中的 OpenAiProvider）
/// 必须遵守以下重试与降级策略：
///
/// 1. 瞬时错误识别与指数退避重试：
///    以下情况视为瞬时错误，应触发重试：
///      - HTTP 状态码：429（Rate Limit）、500、502、503、504
///      - 错误消息中包含关键词："timeout"、"overloaded"、"temporarily unavailable"
///    退避延迟序列：(1s, 2s, 4s)，最多尝试 4 次（1 次初始 + 3 次重试）。
///    参考 nanobot：nanobot/providers/base.py  chat_with_retry() L226-275，
///                  _is_transient_error() L191-193。
///
/// 2. 非瞬时错误 + 请求含图片时的降级策略：
///    若错误为非瞬时（例如 400 Bad Request），且当前消息列表中包含图片内容，
///    则去掉所有图片内容后重试一次（降级到纯文本请求）。
///    重试后无论成功与否，不再继续重试。
///    参考 nanobot：nanobot/providers/base.py  L262-265。
///
/// 3. 全部失败后的返回规范：
///    所有重试耗尽后，返回带 finish_reason = "error" 的 LlmResponse，
///    绝不 panic 或向上层传播 Err，保证 agent_loop.rs 始终能收到一个 LlmResponse。
///
/// 4. 破损 JSON tool call arguments 的修复（json_repair）：
///    LLM 有时返回格式破损的 JSON 作为 tool call arguments（缺括号/引号等）。
///    反序列化失败时，应尝试以下修复策略（参考 nanobot/providers/litellm_provider.py）：
///      a. 尝试补齐末尾缺失的括号/引号（简单修复）
///      b. 使用宽松 JSON 解析库容错（若有）
///      c. 修复失败则将原始字符串包装为 { "raw": "<原始字符串>" }，
///         以 tool_result 形式喂回 LLM，触发自我纠正（agent_loop.rs 的纠错机制）。

// TODO: 定义标准内部结构体
//   pub struct LlmResponse {
//       pub content: Option<String>,         // LLM 的文本输出（可能为 None 当只有 tool calls 时）
//       pub tool_calls: Vec<ToolCallRequest>, // LLM 决定调用的工具列表（可能为空）
//       pub finish_reason: String,           // "stop" | "tool_calls" | "error"
//   }
//   pub struct ToolCallRequest {
//       pub id: String,                      // LLM 生成的 tool_call_id，用于 tool_result 对应
//       pub name: String,                    // 工具名称（对应 RegisterTools 中的 schema name）
//       pub arguments: serde_json::Value,    // 工具参数（已经过 json_repair 处理）
//   }

// TODO: 定义 LlmProvider Trait
//   #[async_trait]
//   pub trait LlmProvider: Send + Sync {
//       async fn chat_with_retry(
//           &self,
//           messages: Vec<Message>,
//           tools: Vec<Value>,
//           model: &str,
//       ) -> LlmResponse;
//       // 遵守上述行为契约，永不 panic，永不向上传播 Err。
//   }