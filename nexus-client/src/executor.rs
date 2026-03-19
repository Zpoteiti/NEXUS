/// 职责边界：
/// 1. 真正的“手脚”。接收 Server 传来的指令，执行本地的 Shell 命令、文件读写等。
/// 2. 必须包含基础的安全护栏 (Guardrails)。
///
/// 参考 nanobot：
/// - 【核心参考】仔细阅读 `nanobot/agent/tools/` 目录下的具体工具实现。
/// - 安全策略必须 1:1 移植 `nanobot/security/network.py` 以及 `nanobot/agent/tools/shell.py` 中的 `_guard_command` 逻辑（拦截 rm -rf、路径穿越等高危操作）。
/// - 将执行后的 stdout/stderr 封装回我们 nexus_common 定义的 ToolExecutionResult 中。

// TODO: 实现 execute_tool(request: ExecuteToolRequest) -> ToolExecutionResult
// TODO: 实现 command_guardrails 函数，完全复刻 nanobot 的安全策略

/// 职责边界：
/// 1. 接收 `protocol::ExecuteToolRequest`。
/// 2. 扮演“路由器”角色：
///    - 如果 tool_name == "shell"，则调用 guardrails 检查，通过后交由 tools::shell 执行。
///    - 如果 tool_name 以 "mcp_" 开头，则去掉前缀，转发给对应的 mcp_client 会话。
/// 3. 将任何模块返回的 Ok(String) 或 Err(String) 统一包装为 `protocol::ToolExecutionResult` 向上层返回。

// TODO: pub async fn handle_tool_call(req: ExecuteToolRequest) -> ToolExecutionResult