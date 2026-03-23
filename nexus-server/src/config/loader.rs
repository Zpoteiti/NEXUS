use crate::config::schema::ServerConfig;

pub fn load_config() -> ServerConfig {
    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| panic!("环境变量 DATABASE_URL 未设置，Server 无法启动。\n  示例：DATABASE_URL=postgres://user:pass@localhost/nexus"));

    let admin_token = std::env::var("ADMIN_TOKEN")
        .unwrap_or_else(|_| panic!("环境变量 ADMIN_TOKEN 未设置，Server 无法启动。\n  用途：/admin/register 端点的身份校验 Token"));

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
