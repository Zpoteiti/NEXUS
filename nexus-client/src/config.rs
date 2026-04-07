use std::collections::HashMap;
use std::env;
use nexus_common::consts::{DEVICE_TOKEN_PREFIX, DEVICE_TOKEN_RANDOM_LEN};

/// MCP Server transport type.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum TransportType {
    /// Communicate with child process via stdin/stdout (local MCP server).
    #[default]
    Stdio,
    /// HTTP SSE transport.
    Sse,
    /// Streamable HTTP transport.
    StreamableHttp,
}

#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub name: String,
    /// Transport type: stdio | sse | streamableHttp
    pub transport_type: TransportType,
    pub command: String,
    pub args: Vec<String>,
    pub env: Option<HashMap<String, String>>,
    /// Per-tool call timeout (seconds), default from nexus-common's DEFAULT_MCP_TOOL_TIMEOUT_SEC
    pub tool_timeout: Option<u64>,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub server_ws_url: String,
    pub auth_token: String,
}

/// Convert server McpServerEntry list to client McpServerConfig list.
pub fn mcp_entries_to_configs(entries: &[nexus_common::protocol::McpServerEntry]) -> Vec<McpServerConfig> {
    entries.iter().filter(|e| e.enabled).map(|e| {
        let transport_type = parse_transport_type(e.transport_type.as_deref());
        McpServerConfig {
            name: e.name.clone(),
            transport_type,
            command: e.command.clone(),
            args: e.args.clone(),
            env: e.env.clone(),
            tool_timeout: e.tool_timeout,
            enabled: e.enabled,
        }
    }).collect()
}

fn first_non_empty_env(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        env::var(key).ok().and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
    })
}

fn parse_transport_type(s: Option<&str>) -> TransportType {
    match s.unwrap_or("stdio") {
        "sse" => TransportType::Sse,
        "streamableHttp" | "streamable_http" => TransportType::StreamableHttp,
        _ => TransportType::Stdio,
    }
}

fn validate_server_ws_url(url: &str) {
    if !(url.starts_with("ws://") || url.starts_with("wss://")) {
        panic!("NEXUS_SERVER_WS_URL must start with ws:// or wss://");
    }
}

fn validate_auth_token(token: &str) {
    if !token.starts_with(DEVICE_TOKEN_PREFIX) {
        panic!("NEXUS_AUTH_TOKEN must start with '{}'", DEVICE_TOKEN_PREFIX);
    }
    let random = &token[DEVICE_TOKEN_PREFIX.len()..];
    if random.len() != DEVICE_TOKEN_RANDOM_LEN {
        panic!("NEXUS_AUTH_TOKEN random segment must be exactly {} characters", DEVICE_TOKEN_RANDOM_LEN);
    }
}

pub fn load_config() -> ClientConfig {
    dotenvy::dotenv().ok();

    let server_ws_url = first_non_empty_env(&["NEXUS_SERVER_WS_URL", "NEXUS_WS_URL"])
        .unwrap_or_else(|| panic!("NEXUS_SERVER_WS_URL is required (e.g. ws://127.0.0.1:8080/ws)"));
    validate_server_ws_url(&server_ws_url);

    let auth_token = first_non_empty_env(&["NEXUS_AUTH_TOKEN", "NEXUS_DEVICE_TOKEN"])
        .unwrap_or_else(|| panic!("NEXUS_AUTH_TOKEN is required"));
    validate_auth_token(&auth_token);

    ClientConfig {
        server_ws_url,
        auth_token,
    }
}
