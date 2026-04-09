//! Server configuration from env vars + DB-stored LLM config.

use serde::{Deserialize, Serialize};

/// Configuration loaded from environment variables at startup.
/// All required fields panic if missing — fail fast.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub database_url: String,
    pub admin_token: String,
    pub jwt_secret: String,
    pub server_port: u16,
    pub gateway_ws_url: String,
    pub gateway_token: String,
    pub skills_dir: String,
}

impl ServerConfig {
    pub fn from_env() -> Self {
        Self {
            database_url: env_required("DATABASE_URL"),
            admin_token: env_required("ADMIN_TOKEN"),
            jwt_secret: env_required("JWT_SECRET"),
            server_port: env_required("SERVER_PORT")
                .parse()
                .expect("SERVER_PORT must be a number"),
            gateway_ws_url: env_required("NEXUS_GATEWAY_WS_URL"),
            gateway_token: env_required("NEXUS_GATEWAY_TOKEN"),
            skills_dir: std::env::var("NEXUS_SKILLS_DIR").unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                format!("{home}/.nexus/skills")
            }),
        }
    }
}

fn env_required(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{key} must be set"))
}

/// LLM provider config stored in system_config table. Hot-reloadable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub api_base: String,
    pub model: String,
    pub api_key: String,
    pub context_window: u32,
}
