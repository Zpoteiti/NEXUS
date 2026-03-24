/// 职责边界：
/// 1. 实现 `LlmProvider` Trait，使用 `async-openai` 库调用本地 LM Studio。
/// 2. 【核心难点】处理大模型经常输出破损 JSON Arguments 的问题。
///
/// 参考 nanobot：
/// - 对应 `nanobot/providers/litellm_provider.py`。
/// - 参考它的 `json_repair` 逻辑。在 Rust 中，我们可以在反序列化失败时，尝试手动补齐括号或使用宽松的 JSON 解析库容错。

// TODO: 实现 OpenAiProvider 结构体及其 chat_with_retry 方法