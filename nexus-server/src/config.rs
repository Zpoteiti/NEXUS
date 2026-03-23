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
        .unwrap_or(60);

    ServerConfig {
        database_url,
        admin_token,
        server_port,
        heartbeat_timeout_sec,
    }
}
