/// 职责边界：
/// 1. 存放 Server 和 Client 共享的全局常量。
/// 2. 杜绝在两端代码中硬编码 (Hardcode) 魔法数字或字符串。

// TODO: 定义协议版本号 (例如 pub const PROTOCOL_VERSION: &str = "1.0";)
// TODO: 定义默认的心跳间隔时间 (例如 pub const HEARTBEAT_INTERVAL_SEC: u64 = 15;)
// TODO: 定义标准的 MCP 本地接口超时时间