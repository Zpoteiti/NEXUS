#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub database_url: String,
    pub admin_token: String,
    pub server_port: u16,
    pub heartbeat_timeout_sec: u64,
}
