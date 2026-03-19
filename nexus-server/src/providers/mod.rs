/// 职责边界：
/// 1. 定义 `LlmProvider` Trait，包含 `chat_with_retry` 方法。
/// 2. 负责统一定义模型返回的内部标准结构：`LlmResponse` 和 `ToolCallRequest`。
///
/// 参考 nanobot：
/// - 对应 `nanobot/providers/base.py`。
/// - 将千奇百怪的 LLM API 统一抽象隔离，保护 agent_loop.rs 不受具体厂商数据格式的污染。

// TODO: 定义 LlmProvider Trait
// TODO: 定义标准的 ToolCallRequest 结构体