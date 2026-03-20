/// 职责边界：
/// 负责 Server 启动时从环境变量（.env 文件或宿主机 ENV）加载所有配置。
/// 优先级：环境变量 > 代码内默认值。
/// main.rs 在最开头调用 load_config()，将返回的 ServerConfig 传入各模块初始化函数。
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【配置字段一览】
/// ─────────────────────────────────────────────────────────────────────────────
///
/// 数据库 & 安全（必填，无默认值，缺失时 load_config() 应 panic 并打印提示）：
///   DATABASE_URL     — PostgreSQL 连接字符串，例如 postgres://user:pass@localhost/nexus
///   JWT_SECRET       — JWT 签名密钥，建议至少 32 字节随机字符串
///   ADMIN_TOKEN      — Admin 注册时的校验 Token（auth.rs 使用）
///
/// LLM Provider（必填 API_KEY，其余有默认值）：
///   LLM_API_KEY      — LLM 服务的 API Key（必填）
///   LLM_API_BASE     — LLM API 基础 URL，默认：http://127.0.0.1:1234/v1（指向本地 LM Studio）
///   LLM_MODEL_NAME   — 模型名称，默认：local-model
///
/// Server 运行参数（均有默认值）：
///   SERVER_PORT            — HTTP/WS 监听端口，默认：8080
///   HEARTBEAT_TIMEOUT_SEC  — 超过此秒数未收到心跳则从 AppState 剔除该设备，默认：60
///   MAX_AGENT_ITERATIONS   — Agent ReAct 循环最大迭代次数，默认：40
///                            超限后将最后一次 LLM 文本输出直接返回给用户
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【参考 nanobot】
/// ─────────────────────────────────────────────────────────────────────────────
/// nanobot/config/schema.py  — Pydantic Config 模型，支持 NANOBOT_ 前缀环境变量覆盖
/// nanobot/config/loader.py  — 从 config.json 加载并 migrate
/// Nexus 抛弃 config.json，纯环境变量，符合十二要素法则（12-factor app）。
/// 与 nanobot 的 ~100 行 Pydantic 模型相比，Server 端只保留运行必需的最小字段集。

// TODO: pub struct ServerConfig {
//     // 必填字段（缺失时 load_config 应 panic）
//     pub database_url: String,
//     pub jwt_secret: String,
//     pub admin_token: String,
//     pub llm_api_key: String,
//     // 有默认值的字段
//     pub llm_api_base: String,          // 默认：http://127.0.0.1:1234/v1
//     pub llm_model_name: String,        // 默认：local-model
//     pub server_port: u16,              // 默认：8080
//     pub heartbeat_timeout_sec: u64,    // 默认：60
//     pub max_agent_iterations: u32,     // 默认：40（与 consts::MAX_AGENT_ITERATIONS 对齐）
//
//     // LLM 行为控制参数（均有默认值）
//     pub context_window_tokens: usize,  // 默认：128000
//                                         // LLM 上下文窗口大小，用于 memory.rs 触发整合的阈值计算
//                                         // （当 prompt tokens > context_window_tokens * 0.5 时触发）
//     pub max_tokens: usize,             // 默认：8192，LLM 单次响应最大 token 数
//     pub temperature: f32,              // 默认：0.7，LLM 采样温度
// }

// TODO: pub fn load_config() -> ServerConfig
//   使用 dotenvy::dotenv().ok() 加载 .env 文件（不存在则静默跳过），
//   再用 std::env::var() 读取各字段；
//   必填字段缺失时调用 panic!("环境变量 XXX 未设置，Server 无法启动") 给出明确提示；
//   有默认值的字段用 unwrap_or_else(|_| "默认值".to_string()) 处理。
//
//   context_window_tokens 是记忆整合的核心触发参数，
//   必须与实际部署的 LLM 模型上下文窗口匹配，否则会导致整合过早或过晚触发。
//   参考 nanobot：nanobot/config/schema.py AgentDefaults.context_window_tokens
