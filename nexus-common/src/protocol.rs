//! WebSocket protocol types for Server-Client communication.
//! Contains only serialization structs/enums — no business logic.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceStatus {
    #[serde(rename = "online")]
    Online,
    #[serde(rename = "offline")]
    Offline,
}

/// Two-tier filesystem policy. Sandbox = workspace only (default). Unrestricted = full access.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode")]
pub enum FsPolicy {
    #[serde(rename = "sandbox")]
    #[default]
    Sandbox,
    #[serde(rename = "unrestricted")]
    Unrestricted,
}

/// MCP server configuration entry, stored per-device in the DB.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpServerEntry {
    pub name: String,
    #[serde(default)]
    pub transport_type: Option<String>,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Option<std::collections::HashMap<String, String>>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: Option<std::collections::HashMap<String, String>>,
    #[serde(default)]
    pub tool_timeout: Option<u64>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Commands sent from server to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ServerToClient {
    ExecuteToolRequest(ExecuteToolRequest),
    RequireLogin {
        message: String,
    },
    LoginSuccess {
        user_id: String,
        device_name: String,
        fs_policy: FsPolicy,
        mcp_servers: Vec<McpServerEntry>,
        workspace_path: String,
        shell_timeout: u64,
        ssrf_whitelist: Vec<String>,
    },
    LoginFailed {
        reason: String,
    },
    HeartbeatAck,
    ConfigUpdate {
        fs_policy: Option<FsPolicy>,
        mcp_servers: Option<Vec<McpServerEntry>>,
        workspace_path: Option<String>,
        shell_timeout: Option<u64>,
        ssrf_whitelist: Option<Vec<String>>,
    },
    FileRequest {
        request_id: String,
        path: String,
    },
    FileSend {
        request_id: String,
        filename: String,
        content_base64: String,
        destination: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolRequest {
    pub request_id: String,
    pub tool_name: String,
    pub arguments: Value,
}

/// Events reported from client to server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ClientToServer {
    ToolExecutionResult(ToolExecutionResult),
    SubmitToken {
        token: String,
        protocol_version: String,
    },
    RegisterTools {
        schemas: Vec<Value>,
    },
    Heartbeat {
        status: DeviceStatus,
    },
    FileResponse {
        request_id: String,
        content_base64: String,
        mime_type: Option<String>,
        error: Option<String>,
    },
    FileSendAck {
        request_id: String,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecutionResult {
    pub request_id: String,
    /// 0=success, 1=failed, -1=timeout, -2=cancelled
    pub exit_code: i32,
    pub output: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fspolicy_default_is_sandbox() {
        assert_eq!(FsPolicy::default(), FsPolicy::Sandbox);
    }

    #[test]
    fn test_fspolicy_serialize_deserialize() {
        let sandbox = FsPolicy::Sandbox;
        let json = serde_json::to_string(&sandbox).unwrap();
        assert_eq!(json, r#"{"mode":"sandbox"}"#);
        let round: FsPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(round, sandbox);

        let unrestricted = FsPolicy::Unrestricted;
        let json = serde_json::to_string(&unrestricted).unwrap();
        assert_eq!(json, r#"{"mode":"unrestricted"}"#);
    }

    #[test]
    fn test_login_success_round_trip() {
        let msg = ServerToClient::LoginSuccess {
            user_id: "u1".into(),
            device_name: "d1".into(),
            fs_policy: FsPolicy::Sandbox,
            mcp_servers: vec![],
            workspace_path: "/home/dev".into(),
            shell_timeout: 60,
            ssrf_whitelist: vec!["10.0.0.0/8".into()],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let de: ServerToClient = serde_json::from_str(&json).unwrap();
        match de {
            ServerToClient::LoginSuccess {
                workspace_path,
                shell_timeout,
                ssrf_whitelist,
                ..
            } => {
                assert_eq!(workspace_path, "/home/dev");
                assert_eq!(shell_timeout, 60);
                assert_eq!(ssrf_whitelist, vec!["10.0.0.0/8"]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_config_update_partial() {
        let msg = ServerToClient::ConfigUpdate {
            fs_policy: Some(FsPolicy::Unrestricted),
            mcp_servers: None,
            workspace_path: None,
            shell_timeout: Some(120),
            ssrf_whitelist: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let _: ServerToClient = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn test_heartbeat_ack_lightweight() {
        let msg = ServerToClient::HeartbeatAck;
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"HeartbeatAck"}"#);
    }

    #[test]
    fn test_heartbeat_no_hash() {
        let msg = ClientToServer::Heartbeat {
            status: DeviceStatus::Online,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("hash"));
    }

    #[test]
    fn test_submit_token_round_trip() {
        let msg = ClientToServer::SubmitToken {
            token: "nexus_dev_abc".into(),
            protocol_version: "1.0".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let _: ClientToServer = serde_json::from_str(&json).unwrap();
    }
}
