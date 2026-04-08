#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LlmConfig {
    pub api_base: String,        // Required: OpenAI-compatible endpoint URL
    pub api_key: String,
    pub model: String,
    pub context_window: usize,
}

use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub database_url: String,
    pub admin_token: String,
    pub server_port: u16,
    pub heartbeat_timeout_sec: u64,
    pub gateway_ws_url: String,
    pub gateway_token: String,
    pub jwt_secret: String,
    pub bcrypt_cost: u32,
    pub llm: Arc<RwLock<Option<LlmConfig>>>,
    pub skills_dir: String,
}

pub fn load_config() -> ServerConfig {
    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| panic!("DATABASE_URL env var is not set. Server cannot start.\n  Example: DATABASE_URL=postgres://user:pass@localhost/nexus"));

    let admin_token = std::env::var("ADMIN_TOKEN")
        .unwrap_or_else(|_| panic!("ADMIN_TOKEN env var is not set. Server cannot start.\n  Used for: /admin/register endpoint authentication"));

    let server_port = std::env::var("SERVER_PORT")
        .unwrap_or_else(|_| panic!("SERVER_PORT env var is not set. Server cannot start.\n  Example: SERVER_PORT=8080"))
        .parse::<u16>()
        .unwrap_or_else(|e| panic!("SERVER_PORT env var has invalid format: {e}. Must be an integer between 1-65535"));

    let heartbeat_timeout_sec = std::env::var("HEARTBEAT_TIMEOUT_SEC")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(60);

    let gateway_ws_url = std::env::var("NEXUS_GATEWAY_WS_URL")
        .unwrap_or_else(|_| panic!("NEXUS_GATEWAY_WS_URL env var is not set. Server cannot start.\n  Example: NEXUS_GATEWAY_WS_URL=ws://localhost:9090/ws/nexus"));

    let gateway_token = std::env::var("NEXUS_GATEWAY_TOKEN")
        .unwrap_or_else(|_| panic!("NEXUS_GATEWAY_TOKEN env var is not set. Server cannot start.\n  Used for: authenticating the server's WebSocket connection to the gateway"));

    let jwt_secret = std::env::var("JWT_SECRET")
        .unwrap_or_else(|_| panic!("JWT_SECRET env var is not set. Server cannot start.\n  Used for: JWT signing key, recommend at least 32 characters"));

    let bcrypt_cost = std::env::var("BCRYPT_COST")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(12);

    let skills_dir = std::env::var("NEXUS_SKILLS_DIR").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        format!("{}/.nexus/skills", home)
    });

    ServerConfig {
        database_url,
        admin_token,
        server_port,
        heartbeat_timeout_sec,
        gateway_ws_url,
        gateway_token,
        jwt_secret,
        bcrypt_cost,
        llm: Arc::new(RwLock::new(None)),
        skills_dir,
    }
}
