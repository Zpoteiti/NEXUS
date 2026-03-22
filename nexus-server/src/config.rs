/// 职责边界：
/// 负责 Server 启动时从环境变量（.env 文件或宿主机 ENV）加载所有配置。
/// 优先级：环境变量 > 代码内默认值。
/// main.rs 在最开头调用 load_config()，将返回的 ServerConfig 传入各模块初始化函数。
///
/// 参考 nanobot：
/// - nanobot/config/schema.py  AgentDefaults（context_window_tokens, max_tokens, temperature, max_tool_iterations）
/// - nanobot/config/schema.py  GatewayConfig（host, port）
/// - nanobot/config/schema.py  ProviderConfig（api_key, api_base）
/// - nanobot/config/loader.py  load_config()（文件加载 + 静默降级）
///
/// Nexus 抛弃 config.json，纯环境变量，符合十二要素法则（12-factor app）。
/// 与 nanobot 的 ~100 行 Pydantic 模型相比，Server 端只保留运行必需的最小字段集。

use nexus_common::consts;

/// 全量服务器配置，由 load_config() 构造，在 main.rs 中传给各模块。
#[derive(Debug, Clone)]
pub struct ServerConfig {
    // ─── 必填字段（缺失时 load_config 会 panic）────────────────────────────────

    /// PostgreSQL 连接字符串。
    /// 例：postgres://user:pass@localhost/nexus
    /// 对应 nanobot：无（nanobot 使用 SQLite 文件，不需要连接串）
    pub database_url: String,

    /// Admin 注册时的校验 Token，auth.rs 的 /admin/register 端点使用。
    /// 对应 nanobot：无
    pub admin_token: String,

    // ─── LLM Provider（均可选，未配置时 Agent Loop 无法调用 LLM）─────────────

    /// LLM 服务的 API Key。
    /// None 表示未配置 LLM，Server 启动正常但 Agent Loop 无法运行。
    /// 对应 nanobot：schema.py ProviderConfig.api_key
    pub llm_api_key: Option<String>,

    // ─── LLM Provider（有默认值）──────────────────────────────────────────────

    /// LLM API 基础 URL。
    /// 默认：http://127.0.0.1:1234/v1（指向本地 LM Studio）
    /// 对应 nanobot：schema.py ProviderConfig.api_base
    pub llm_api_base: String,

    /// 模型名称。
    /// 默认：local-model
    /// 对应 nanobot：schema.py AgentDefaults.model
    pub llm_model_name: String,

    // ─── Agent 行为控制（有默认值）────────────────────────────────────────────

    /// Agent ReAct 循环最大迭代次数，超限后将最后一次 LLM 文本输出直接返回给用户。
    /// 默认：40（与 consts::MAX_AGENT_ITERATIONS 对齐）
    /// 对应 nanobot：schema.py AgentDefaults.max_tool_iterations
    pub max_agent_iterations: u32,

    /// LLM 上下文窗口大小（token 数）。
    /// memory.rs 用此值计算整合触发阈值：当 prompt tokens > context_window_tokens * 0.5 时触发整合。
    /// 默认：128000（对应主流部署规格；nanobot 默认为 65536）
    /// 对应 nanobot：schema.py AgentDefaults.context_window_tokens
    pub context_window_tokens: usize,

    /// LLM 单次响应最大 token 数。
    /// 默认：8192
    /// 对应 nanobot：schema.py AgentDefaults.max_tokens
    pub max_tokens: usize,

    /// LLM 采样温度。
    /// 默认：0.7（nanobot 默认为 0.1，Nexus 用更高值以获得更自然的对话输出）
    /// 对应 nanobot：schema.py AgentDefaults.temperature
    pub temperature: f32,

    // ─── Server 运行参数（有默认值）───────────────────────────────────────────

    /// HTTP/WS 服务监听端口。
    /// 默认：8080（nanobot gateway 默认为 18790）
    /// 对应 nanobot：schema.py GatewayConfig.port
    pub server_port: u16,

    /// 超过此秒数未收到心跳则从 AppState 中剔除该设备。
    /// 默认：60（= HEARTBEAT_INTERVAL_SEC * 4）
    /// 对应 nanobot：schema.py HeartbeatConfig.interval_s（语义不同；nanobot 心跳是状态广播，Nexus 是设备存活检测）
    pub heartbeat_timeout_sec: u64,
}

/// 从环境变量（+ .env 文件）加载完整服务器配置。
///
/// 加载顺序：
///   1. dotenvy::dotenv().ok() 加载工作目录下的 .env 文件（文件不存在则静默跳过）
///   2. std::env::var() 读取各字段
///   3. 必填字段缺失时 panic! 并打印明确提示
///   4. 有默认值的字段使用 unwrap_or_else 回退
pub fn load_config() -> ServerConfig {
    // 加载 .env 文件，文件不存在则静默跳过（符合生产部署惯例：ENV 直接由宿主机注入）
    dotenvy::dotenv().ok();

    // ─── 必填字段 ────────────────────────────────────────────────────────────
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| panic!("环境变量 DATABASE_URL 未设置，Server 无法启动。\n  示例：DATABASE_URL=postgres://user:pass@localhost/nexus"));

    let admin_token = std::env::var("ADMIN_TOKEN")
        .unwrap_or_else(|_| panic!("环境变量 ADMIN_TOKEN 未设置，Server 无法启动。\n  用途：/admin/register 端点的身份校验 Token"));

    // ─── LLM Provider（有默认值；未设置 LLM_API_KEY 时 Agent Loop 无法运行）──
    let llm_api_key = std::env::var("LLM_API_KEY").ok();

    // ─── LLM Provider（有默认值）─────────────────────────────────────────────
    let llm_api_base = std::env::var("LLM_API_BASE")
        .unwrap_or_else(|_| "http://127.0.0.1:1234/v1".to_string());

    let llm_model_name = std::env::var("LLM_MODEL_NAME")
        .unwrap_or_else(|_| "local-model".to_string());

    // ─── Agent 行为控制（有默认值）───────────────────────────────────────────
    let max_agent_iterations = std::env::var("MAX_AGENT_ITERATIONS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(consts::MAX_AGENT_ITERATIONS);

    let context_window_tokens = std::env::var("CONTEXT_WINDOW_TOKENS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(128_000);

    let max_tokens = std::env::var("MAX_TOKENS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(8192);

    let temperature = std::env::var("TEMPERATURE")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(0.7);

    // ─── Server 运行参数（有默认值）──────────────────────────────────────────
    let server_port = match std::env::var("SERVER_PORT") {
        Ok(val) => val.parse::<u16>().unwrap_or_else(|_| {
            panic!("环境变量 SERVER_PORT 格式错误：'{}'，必须是 1-65535 之间的整数", val)
        }),
        Err(_) => 8080,
    };

    let heartbeat_timeout_sec = std::env::var("HEARTBEAT_TIMEOUT_SEC")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(consts::HEARTBEAT_INTERVAL_SEC * 4);

    ServerConfig {
        database_url,
        admin_token,
        llm_api_key,
        llm_api_base,
        llm_model_name,
        max_agent_iterations,
        context_window_tokens,
        max_tokens,
        temperature,
        server_port,
        heartbeat_timeout_sec,
    }
}
