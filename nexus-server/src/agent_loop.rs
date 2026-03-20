/// 职责边界：
/// 1. 实现核心的 `run_agent_loop` 函数，控制 ReAct（思考-行动）的 while 循环。
/// 2. 【挂起/唤醒机制】当 Provider 返回 ToolCall 时：
///    a. agent_loop 生成唯一的 request_id（UUID）。
///    b. 创建 oneshot::channel，将 oneshot::Sender<ToolExecutionResult> 存入
///       AppState 的挂起等待表（HashMap<RequestId, oneshot::Sender>）。
///    c. 通过 AppState 的在线设备路由表找到目标设备的 ws_tx，
///       发送 ServerToClient::ExecuteToolRequest（含 request_id 和工具参数）。
///    d. agent_loop .await oneshot::Receiver，挂起当前循环，让出线程。
///    e. ws.rs 收到 Client 返回的 ToolExecutionResult 后，
///       从 AppState 挂起等待表中取出对应的 oneshot::Sender，
///       调用 .send(result) 唤醒 agent_loop，循环继续执行。
/// 3. 【自我纠正机制】收到执行错误或参数校验失败时，
///    不抛出系统异常，而是将错误信息包装为 "Tool Result (Error Hint)"，
///    以 tool_result 角色消息喂回 LLM，触发其自我纠正并重新生成参数。
/// 4. 【LLM 错误响应不入历史】
///    当 LLM 返回 finish_reason = "error" 时，该响应不写入 session 历史，
///    不调用 db::save_message()，避免错误消息污染后续上下文，
///    防止 LLM 陷入"读到错误→继续报错"的持续 400 循环。
///
/// 参考 nanobot：
/// - 完全复刻 nanobot/agent/loop.py 中的 _run_agent_loop 状态机逻辑。
/// - nanobot/agent/loop.py _run_agent_loop 中 finish_reason=="error" 分支（约 L234-239）。
/// - Rust 中等待 Client 执行结果需要用 tokio::sync::oneshot 配合 AppState 挂起等待表。

// TODO: 实现 pub async fn run_agent_loop(state: AppState, session_id: SessionId, user_prompt: String)
