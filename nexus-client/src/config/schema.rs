use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Option<HashMap<String, String>>,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub server_ws_url: String,
    pub device_id: String,
    pub device_name: String,
    pub auth_token: String,
    pub mcp_servers: Vec<McpServerConfig>,
    pub skills_dir: PathBuf,
}
