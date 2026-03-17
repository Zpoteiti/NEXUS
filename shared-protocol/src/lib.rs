use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

pub fn load_toml_config_or_default<T>(path: impl AsRef<Path>) -> Result<T, ConfigLoadError>
where
    T: DeserializeOwned + Default,
{
    let path = path.as_ref();
    if !path.exists() {
        return Ok(T::default());
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|source| ConfigLoadError::Read { path: path.display().to_string(), source })?;
    toml::from_str::<T>(&raw)
        .map_err(|source| ConfigLoadError::Parse { path: path.display().to_string(), source })
}

#[derive(Debug, Error)]
pub enum ConfigLoadError {
    #[error("failed to read config from {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse toml config from {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeLimits {
    pub max_connections: usize,
    pub request_timeout_ms: u64,
    pub max_inflight_requests: usize,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self { max_connections: 500, request_timeout_ms: 30_000, max_inflight_requests: 2_000 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthConfig {
    pub token_issuer: String,
    pub audience: String,
    pub node_auth_token: String,
    pub admin_username: String,
    pub admin_password: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            token_issuer: "nexus".to_owned(),
            audience: "nexus-nodes".to_owned(),
            node_auth_token: "dev-token".to_owned(),
            admin_username: "change-me-admin-user".to_owned(),
            admin_password: "change-me-admin-password".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub postgres_dsn: String,
    pub vlm_endpoint: String,
    pub limits: RuntimeLimits,
    pub auth: AuthConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:7878".to_owned(),
            postgres_dsn: "postgres://user:pass@127.0.0.1:5432/nexus".to_owned(),
            vlm_endpoint: "http://100.80.10.33:1234/v1/chat/completions".to_owned(),
            limits: RuntimeLimits::default(),
            auth: AuthConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientNodeConfig {
    pub node_id: String,
    pub tenant_id: String,
    pub user_id: String,
    pub server_endpoint: String,
    pub auth_token: String,
    pub limits: RuntimeLimits,
}

impl Default for ClientNodeConfig {
    fn default() -> Self {
        Self {
            node_id: "local-node-1".to_owned(),
            tenant_id: "tenant-dev".to_owned(),
            user_id: "user-dev".to_owned(),
            server_endpoint: "ws://127.0.0.1:7878/ws".to_owned(),
            auth_token: "dev-token".to_owned(),
            limits: RuntimeLimits::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeRegistration {
    pub node_id: String,
    pub tenant_id: String,
    pub user_id: String,
    pub auth_token: String,
}

impl NodeRegistration {
    pub fn new(
        node_id: impl Into<String>,
        tenant_id: impl Into<String>,
        user_id: impl Into<String>,
        auth_token: impl Into<String>,
    ) -> Self {
        Self {
            node_id: node_id.into(),
            tenant_id: tenant_id.into(),
            user_id: user_id.into(),
            auth_token: auth_token.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tenant {
    pub tenant_id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserAccount {
    pub tenant_id: String,
    pub user_id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoginUser {
    pub username: String,
    pub password_hash: String,
    pub tenant_id: String,
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoginSession {
    pub session_id: String,
    pub username: String,
    pub tenant_id: String,
    pub user_id: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserDevice {
    pub tenant_id: String,
    pub user_id: String,
    pub node_id: String,
    pub alias: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserChannelBinding {
    pub tenant_id: String,
    pub user_id: String,
    pub channel_name: String,
    pub external_user: String,
    pub credentials_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRecord {
    pub tenant_id: String,
    pub user_id: String,
    pub session_id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRecord {
    pub tenant_id: String,
    pub user_id: String,
    pub session_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageRecord {
    pub tenant_id: String,
    pub user_id: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeConnection {
    pub tenant_id: String,
    pub user_id: String,
    pub node_id: String,
    pub connected_at_ms: u64,
    pub last_seen_ms: u64,
    pub inflight_requests: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolKind {
    Shell,
    Browser,
    Filesystem,
    Calculator,
    CustomMcp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolRequest {
    pub request_id: String,
    pub tenant_id: String,
    pub user_id: String,
    pub node_id: String,
    pub tool: ToolKind,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolResult {
    pub request_id: String,
    pub ok: bool,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeHello {
    pub registration: NodeRegistration,
    pub custom_tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageSummary {
    pub tenant_id: String,
    pub requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClientToServer {
    Hello(NodeHello),
    Pong { node_id: String },
    ToolResult(ToolResult),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ServerToClient {
    Ack { node_id: String },
    Ping,
    ToolRequest(ToolRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderHealth {
    pub endpoint: String,
    pub reachable: bool,
    pub status_code: u16,
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("invalid node registration")]
    InvalidRegistration,
    #[error("authentication failed")]
    AuthFailed,
    #[error("message decode failed")]
    DecodeFailed,
}

#[cfg(test)]
mod tests {
    use super::load_toml_config_or_default;
    use serde::Deserialize;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Debug, Default, Deserialize, PartialEq, Eq)]
    struct DemoConfig {
        value: String,
    }

    #[test]
    fn load_config_returns_default_when_file_missing() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("nexus-missing-{nonce}.toml"));
        let cfg = load_toml_config_or_default::<DemoConfig>(&path).expect("load default config");
        assert_eq!(cfg, DemoConfig::default());
    }

    #[test]
    fn load_config_parses_existing_toml_file() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("nexus-config-{nonce}.toml"));
        std::fs::write(&path, "value = \"ok\"\n").expect("write config file");
        let cfg = load_toml_config_or_default::<DemoConfig>(&path).expect("load config from toml");
        assert_eq!(cfg.value, "ok");
        std::fs::remove_file(path).expect("cleanup temp file");
    }
}
