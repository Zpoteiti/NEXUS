/// 职责边界：
/// 1. 负责在本地启动并连接第三方 MCP Server（通过 stdio 子进程交互）。
/// 2. 初始化时发送 `tools/list` 获取外部工具，并在命名前加上 `mcp_` 前缀（命名隔离）。
/// 3. 将包装好的 Schema 组合起来，通过 WebSocket 发送 `RegisterTools` 给 Server。
/// 4. 收到执行请求时，将请求透传给对应的 MCP Server 并返回结果。
///
/// 参考 nanobot：
/// - 完全复刻 `nanobot/agent/mcp/` 以及包装器 `MCPToolWrapper` 的逻辑。
/// - 这里是 Nexus 架构真正的威力所在：Client 变成了一个无限扩展的插件底座。

// TODO: 实现 connect_mcp_server(command, args)
// TODO: 实现 fetch_and_register_tools()
// TODO: 实现 call_mcp_tool(tool_name, arguments)

/// 职责边界：
/// 1. 管理与外部 MCP Server 的 stdio 子进程生命周期。
/// 2. 负责握手 (Initialize) 并调用 `tools/list` 获取 Schema。
/// 3. 【关键映射】给外部工具强制加上 `mcp_{server_name}_` 前缀，提取 `description` 和完整的 `inputSchema`。
/// 4. 调用 `call_tool`，并将返回的复杂 Block (ImageContent 等) 粗暴降级为字符串 (Stringified)，合并成纯文本返回。
///
/// 参考 nanobot：
/// - 完全复刻 `nanobot/agent/mcp/` 的行为，特别是内容降级逻辑。

// TODO: pub struct McpSession { ... }
// TODO: pub async fn list_and_wrap_tools(&self) -> Vec<Value>
// TODO: pub async fn call_mcp_tool(&self, name: &str, args: Value) -> Result<String, String>