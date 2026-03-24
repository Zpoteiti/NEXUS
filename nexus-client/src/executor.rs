/// 职责边界：
/// 1. 真正的“手脚”。接收 Server 传来的指令，执行本地的 Shell 命令、文件读写等。
/// 2. 必须包含基础的安全护栏 (Guardrails)。
///
/// 参考 nanobot：
/// - 【核心参考】仔细阅读 `nanobot/agent/tools/` 目录下的具体工具实现。
/// - 安全策略必须 1:1 移植 `nanobot/security/network.py` 以及 `nanobot/agent/tools/shell.py` 中的 `_guard_command` 逻辑（拦截 rm -rf、路径穿越等高危操作）。
/// - 将执行后的 stdout/stderr 封装回我们 nexus_common 定义的 ToolExecutionResult 中。
///
/// 【工具参数校验与标准错误 Hint 合约】
/// executor.rs 在将工具调用路由给具体执行模块之前，需完成参数的基础校验：
/// - 若 tool_name 对应的 schema 要求必填参数缺失，不执行，直接返回
///   ToolExecutionResult { exit_code: 1, output: "Missing required parameter: {field_name}" }
/// - 输出格式必须对 LLM 友好，明确说明缺少什么，以便 LLM 自我纠正后重新生成参数。
/// - 执行超时时返回：
///   ToolExecutionResult { exit_code: -1, output: "(Tool timed out after {N}s)" }
/// 参考 nanobot：nanobot/agent/tools/base.py cast_params()/validate_params()（L69-189）
///              nanobot/agent/tools/registry.py execute()（L38-59）。

// TODO: 实现 execute_tool(request: ExecuteToolRequest) -> ToolExecutionResult
// TODO: 实现 command_guardrails 函数，完全复刻 nanobot 的安全策略

/// 职责边界：
/// 1. 接收 `protocol::ExecuteToolRequest`。
/// 2. 扮演“路由器”角色：
///    - 如果 tool_name == "shell"，则调用 guardrails 检查，通过后交由 tools::shell 执行。
///    - 如果 tool_name 以 "mcp_" 开头，则去掉前缀，转发给对应的 mcp_client 会话。
/// 3. 将任何模块返回的 Ok(String) 或 Err(String) 统一包装为 `protocol::ToolExecutionResult` 向上层返回。

// TODO: pub async fn handle_tool_call(req: ExecuteToolRequest) -> ToolExecutionResult