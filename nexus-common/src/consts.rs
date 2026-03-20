/// 职责边界：
/// 1. 存放 Server 和 Client 共享的全局常量。
/// 2. 杜绝在两端代码中硬编码 (Hardcode) 魔法数字或字符串。

// TODO: pub const PROTOCOL_VERSION: &str = "1.0";
//   WebSocket 握手时 Client 上报，Server 拒绝不兼容的协议版本。

// TODO: pub const HEARTBEAT_INTERVAL_SEC: u64 = 15;
//   Client 端心跳发送间隔（秒）。
//   Server 端的剔除阈值 HEARTBEAT_TIMEOUT_SEC 在 ServerConfig 中配置（默认 60s），
//   约为本间隔的 4 倍，允许网络抖动导致的偶发心跳丢失。

// TODO: pub const DEFAULT_MCP_TOOL_TIMEOUT_SEC: u64 = 30;
//   单次 MCP 工具调用的超时时间（秒）。
//   超时后 executor.rs 返回 "(MCP tool call timed out after 30s)"，
//   由 agent_loop.rs 将错误喂回 LLM 触发自我纠正。
//   参考 nanobot：nanobot/agent/tools/mcp.py  tool_timeout 默认值（L23）。

// TODO: pub const MAX_AGENT_ITERATIONS: u32 = 40;
//   Agent ReAct 循环（思考-行动）的最大迭代次数。
//   超限后 agent_loop.rs 将最后一次 LLM 文本输出直接返回给用户，不抛 panic。
//   参考 nanobot：nanobot/agent/loop.py  max_iterations 参数（默认 40）。

// TODO: pub const MAX_HISTORY_MESSAGES: usize = 500;
//   单次 LLM 调用携带的最大历史消息条数（从 last_consolidated 游标之后算起）。
//   context::build_message_history() 取末尾 N 条，并从第一条 role=user 消息开始，
//   避免以孤儿 tool_result 开头。
//   参考 nanobot：nanobot/session/manager.py  get_history(max_messages=500) L69。

// TODO: pub const MAX_TOOL_OUTPUT_CHARS: usize = 10_000;
//   工具执行输出的截断阈值（字符数）。
//   超出时采用双端保留策略：前 5000 字符 + "...(X chars truncated)..." + 后 5000 字符。
//   参考 nanobot：nanobot/agent/tools/shell.py  输出截断逻辑（L78）。

// TODO: pub const EXIT_CODE_SUCCESS: i32 = 0;
// TODO: pub const EXIT_CODE_ERROR: i32 = 1;
// TODO: pub const EXIT_CODE_TIMEOUT: i32 = -1;
// TODO: pub const EXIT_CODE_CANCELLED: i32 = -2;
// TODO: pub const EXIT_CODE_VALIDATION_FAILED: i32 = -3;