/// 职责边界：
/// 1. 实现核心的 `run_agent_loop` 函数，控制 ReAct (思考-行动) 的 while 循环。
/// 2. 当 Provider 返回 ToolCall 时，挂起当前对话，通过 ws 模块向 Client 下发 `ExecuteToolRequest`。
/// 3. 【核心机制】收到 Client 执行错误或参数校验失败时，不抛出系统异常，而是包装成 "Tool Result (Error Hint)" 喂回给大模型，触发其“自我纠正”。
///
/// 参考 nanobot：
/// - 完全复刻 `nanobot/agent/loop.py` 中的 `_run_agent_loop` 状态机。
/// - 注意：Rust 中等待 Client 执行结果需要用到 `tokio::sync::oneshot` 或通过共享状态进行挂起唤醒。

// TODO: 实现 run_agent_loop(session_id, user_prompt)