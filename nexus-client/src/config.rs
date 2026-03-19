/// 职责边界：
/// 1. 负责在 Client 启动时，加载运行所需的最少依赖配置。
/// 2. 优先级机制：环境变量 (.env) > 默认值。
/// 3. 必须包含的字段：
///    - `server_ws_url` (默认: ws://127.0.0.1:8080/ws)
///    - `device_id` (如果 env 没配，通过 Rust 标准库自动获取当前机器的主机名 hostname)
///
/// 参考启发：
/// - 抛弃 nanobot 臃肿的 config.json，采用极客最爱的十二要素法则 (Environment Variables)。

// TODO: pub struct ClientConfig { pub server_ws_url: String, pub device_id: String }
// TODO: pub fn load_config() -> ClientConfig