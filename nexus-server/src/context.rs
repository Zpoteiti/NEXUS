/// 职责边界：
/// 1. 负责在每次调用 LLM 前，拼接出完整的 Prompt（System Prompt + History + RAG Memory）。
///
/// 参考 nanobot：
/// - 【核心参考】仔细阅读 `nanobot/agent/context.py` 中的 `ContextBuilder`。
/// - nanobot 是从 `templates/SOUL.md` 读人设，在这里我们改为从 state.rs 获取在线设备，并结合 db.rs 获取历史记录，动态生成给 async-openai 的 Messages 数组。

// TODO: 实现 build_llm_messages(session_id, user_input, online_devices)