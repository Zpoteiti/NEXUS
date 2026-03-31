use std::collections::HashMap;
use std::env;
use std::path::PathBuf;

use nexus_common::consts::{DEVICE_TOKEN_PREFIX, DEVICE_TOKEN_RANDOM_LEN};
use serde::Deserialize;
use serde_json::Value;

/// MCP Server 传输类型
#[derive(Debug, Clone, Default, PartialEq)]
pub enum TransportType {
    /// 通过标准输入输出与子进程通信（本地 MCP Server）
    #[default]
    Stdio,
    /// HTTP SSE 传输
    Sse,
    /// Streamable HTTP 传输
    StreamableHttp,
}

#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub name: String,
    /// 传输类型：stdio | sse | streamableHttp
    pub transport_type: TransportType,
    pub command: String,
    pub args: Vec<String>,
    pub env: Option<HashMap<String, String>>,
    /// MCP Server 的 URL（仅 Sse/StreamableHttp 需要）
    pub url: Option<String>,
    /// HTTP 请求头（仅 Sse/StreamableHttp 需要）
    pub headers: Option<HashMap<String, String>>,
    /// 单个工具调用超时（秒），默认来自 nexus-common 的 DEFAULT_MCP_TOOL_TIMEOUT_SEC
    pub tool_timeout: Option<u64>,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub server_ws_url: String,
    pub auth_token: String,
    pub mcp_servers: Vec<McpServerConfig>,
    pub skills_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
struct McpServerJson {
    #[serde(default)]
    name: Option<String>,
    /// 传输类型：stdio | sse | streamableHttp（默认 stdio）
    #[serde(default)]
    transport_type: Option<String>,
    #[serde(default)]
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    headers: Option<HashMap<String, String>>,
    #[serde(default)]
    tool_timeout: Option<u64>,
    #[serde(default = "default_true")]
    enabled: bool,
}

const DEFAULT_SERVER_WS_URL: &str = "ws://127.0.0.1:8080/ws";

fn default_true() -> bool {
    true
}

fn first_non_empty_env(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        env::var(key).ok().and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
    })
}

fn detect_hostname() -> String {
    first_non_empty_env(&["NEXUS_HOSTNAME", "HOSTNAME", "COMPUTERNAME"])
        .unwrap_or_else(|| "unknown-device".to_string())
}

fn default_skills_dir() -> PathBuf {
    if let Some(home) = first_non_empty_env(&["HOME", "USERPROFILE"]) {
        PathBuf::from(home).join(".nexus").join("skills")
    } else {
        PathBuf::from(".nexus").join("skills")
    }
}

fn parse_mcp_servers_from_value(value: Value) -> Vec<McpServerConfig> {
    let normalized = if let Some(v) = value
        .get("tools")
        .and_then(|tools| tools.get("mcpServers").or_else(|| tools.get("mcp_servers")))
    {
        v.clone()
    } else {
        value
    };

    match normalized {
        Value::Array(items) => items
            .into_iter()
            .filter_map(|item| serde_json::from_value::<McpServerJson>(item).ok())
            .filter_map(|cfg| {
                let name = cfg.name?;
                let transport_type = parse_transport_type(cfg.transport_type.as_deref());
                (!cfg.command.trim().is_empty() || transport_type != TransportType::Stdio)
                    .then(|| McpServerConfig {
                        name,
                        transport_type,
                        command: cfg.command,
                        args: cfg.args,
                        env: (!cfg.env.is_empty()).then_some(cfg.env),
                        url: cfg.url,
                        headers: cfg.headers,
                        tool_timeout: cfg.tool_timeout,
                        enabled: cfg.enabled,
                    })
            })
            .collect(),
        Value::Object(entries) => entries
            .into_iter()
            .filter_map(|(name, item)| {
                serde_json::from_value::<McpServerJson>(item).ok().and_then(|cfg| {
                    let transport_type = parse_transport_type(cfg.transport_type.as_deref());
                    (!cfg.command.trim().is_empty() || transport_type != TransportType::Stdio)
                        .then(|| McpServerConfig {
                            name,
                            transport_type,
                            command: cfg.command,
                            args: cfg.args,
                            env: (!cfg.env.is_empty()).then_some(cfg.env),
                            url: cfg.url,
                            headers: cfg.headers,
                            tool_timeout: cfg.tool_timeout,
                            enabled: cfg.enabled,
                        })
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_transport_type(s: Option<&str>) -> TransportType {
    match s.unwrap_or("stdio") {
        "sse" => TransportType::Sse,
        "streamableHttp" | "streamable_http" => TransportType::StreamableHttp,
        _ => TransportType::Stdio,
    }
}

fn parse_mcp_servers() -> Vec<McpServerConfig> {
    let raw = first_non_empty_env(&["NEXUS_MCP_SERVERS_JSON", "NEXUS_MCP_SERVERS"]);
    raw.and_then(|json| serde_json::from_str::<Value>(&json).ok())
        .map(parse_mcp_servers_from_value)
        .unwrap_or_default()
}

fn validate_server_ws_url(url: &str) {
    if !(url.starts_with("ws://") || url.starts_with("wss://")) {
        panic!("NEXUS_SERVER_WS_URL 格式错误，必须以 ws:// 或 wss:// 开头");
    }
}

fn validate_auth_token(token: &str) {
    if !token.starts_with(DEVICE_TOKEN_PREFIX) {
        panic!("NEXUS_AUTH_TOKEN 格式错误，必须以 nexus_dev_ 开头");
    }
    let random = &token[DEVICE_TOKEN_PREFIX.len()..];
    if random.len() != DEVICE_TOKEN_RANDOM_LEN {
        panic!("NEXUS_AUTH_TOKEN 格式错误，随机段长度必须为 32");
    }
}

pub fn load_config() -> ClientConfig {
    dotenvy::dotenv().ok();

    let server_ws_url = first_non_empty_env(&["NEXUS_SERVER_WS_URL", "NEXUS_WS_URL"])
        .unwrap_or_else(|| DEFAULT_SERVER_WS_URL.to_string());
    validate_server_ws_url(&server_ws_url);

    let auth_token = first_non_empty_env(&["NEXUS_AUTH_TOKEN", "NEXUS_DEVICE_TOKEN"])
        .unwrap_or_else(|| panic!("环境变量 NEXUS_AUTH_TOKEN 未设置，Client 无法完成设备鉴权"));
    validate_auth_token(&auth_token);

    let mcp_servers = parse_mcp_servers();
    let skills_dir = first_non_empty_env(&["NEXUS_SKILLS_DIR", "NEXUS_SKILLS_PATH"])
        .map(PathBuf::from)
        .unwrap_or_else(default_skills_dir);

    ClientConfig {
        server_ws_url,
        auth_token,
        mcp_servers,
        skills_dir,
    }
}
