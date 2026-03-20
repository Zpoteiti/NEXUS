/// 职责边界：
/// 1. 负责在 Client 启动时，加载运行所需的最少依赖配置。
/// 2. 优先级机制：环境变量 > ~/.nexus/client.toml 配置文件 > 默认值。
/// 3. 必须包含的字段：
///    - `server_ws_url`  — Server WebSocket 地址，默认：ws://127.0.0.1:8080/ws
///    - `device_id`      — 设备标识，优先读 NEXUS_DEVICE_ID，缺失则取主机名（hostname）
///    - `auth_token`     — 登录凭据（见下方说明）
///    - `mcp_servers`    — 外部 MCP Server 列表（见下方说明）
///    - `skills_dir`     — 本地 Skill 扫描根目录（见下方说明）
///
/// 参考启发：
/// - 抛弃 nanobot 臃肿的 config.json，采用十二要素法则（Environment Variables 优先）。
/// - 参考 nanobot/config/schema.py  MCPServerConfig 的字段设计。

// TODO: pub struct McpServerConfig {
//     pub name: String,               // 用于工具名前缀，例如 "github" → "mcp_github_*"
//     pub command: String,            // 启动命令，例如 "npx"
//     pub args: Vec<String>,          // 命令参数，例如 ["-y", "@modelcontextprotocol/server-github"]
//     pub env: Option<HashMap<String, String>>, // 子进程额外的环境变量（如 GITHUB_TOKEN）
//     pub enabled: bool,              // false 时跳过启动，方便临时禁用
// }
//   从 ~/.nexus/client.toml 的 [[mcp_servers]] 数组读取；
//   也可通过 NEXUS_MCP_SERVERS_JSON 环境变量传入 JSON 数组（优先级更高）。
//   参考 nanobot：nanobot/config/schema.py  MCPServerConfig（tools.mcp_servers 字典）。

// TODO: pub struct ClientConfig {
//     pub server_ws_url: String,      // 默认：ws://127.0.0.1:8080/ws
//     pub device_id: String,          // 默认：hostname()
//     pub auth_token: Option<String>, // 优先读 NEXUS_AUTH_TOKEN 环境变量；
//                                     // 若为 None，ws.rs 握手时由用户交互输入或走 email+password 流程
//     pub mcp_servers: Vec<McpServerConfig>, // 默认：空列表（不连接外部 MCP）
//     pub skills_dir: PathBuf,        // 默认：~/.nexus/skills/
//                                     // skills::scan_skills() 以此为扫描根目录
// }

// TODO: pub fn load_config() -> ClientConfig
//   加载顺序：
//   1. 调用 dotenvy::dotenv().ok() 加载 .env（静默跳过不存在的情况）
//   2. 读取 ~/.nexus/client.toml（若存在）作为基础配置
//   3. 用环境变量覆盖对应字段（NEXUS_SERVER_WS_URL、NEXUS_DEVICE_ID、NEXUS_AUTH_TOKEN 等）
//   4. device_id 缺失时调用 hostname::get() 获取主机名作为默认值