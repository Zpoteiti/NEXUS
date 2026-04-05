//! Server-side MCP client manager.
//! Admins can configure shared MCP servers on the NEXUS server.
//! All users' agents automatically get these tools with device_name="server".

use nexus_common::error::{ErrorCode, NexusError};
use nexus_common::mcp_utils::normalize_schema_for_openai;
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::time::{timeout, Duration};
use tracing::{info, warn};

use nexus_common::protocol::McpServerEntry;

/// Server-side MCP session wrapping an rmcp client.
pub struct ServerMcpSession {
    server_name: String,
    client: rmcp::service::RunningService<rmcp::RoleClient, ()>,
    tool_name_map: HashMap<String, String>, // wrapped → original
    cached_schemas: Vec<Value>,              // cached tool schemas for LLM
    tool_timeout: u64,
}

impl ServerMcpSession {
    pub async fn connect(entry: &McpServerEntry) -> Result<Self, NexusError> {
        let tool_timeout = entry.tool_timeout.unwrap_or(30);

        let mut cmd = tokio::process::Command::new(&entry.command);
        cmd.args(&entry.args);
        if let Some(ref env) = entry.env {
            cmd.envs(env);
        }

        let transport = rmcp::transport::TokioChildProcess::new(cmd)
            .map_err(|e| NexusError::new(ErrorCode::McpConnectionFailed, format!("failed to spawn server MCP '{}': {}", entry.name, e)))?;

        let client = timeout(
            Duration::from_secs(30),
            rmcp::serve_client((), transport),
        )
        .await
        .map_err(|_| NexusError::new(ErrorCode::McpConnectionFailed, format!("server MCP '{}': init timeout", entry.name)))?
        .map_err(|e| NexusError::new(ErrorCode::McpConnectionFailed, format!("server MCP '{}': init failed: {}", entry.name, e)))?;

        Ok(Self {
            server_name: entry.name.clone(),
            client,
            tool_name_map: HashMap::new(),
            cached_schemas: Vec::new(),
            tool_timeout,
        })
    }

    pub async fn list_tools(&mut self) -> Result<Vec<Value>, NexusError> {
        let tools = self.client.list_all_tools().await
            .map_err(|e| NexusError::new(ErrorCode::McpCallFailed, format!("server MCP '{}': list_tools failed: {}", self.server_name, e)))?;

        let mut schemas = Vec::new();
        for tool in tools {
            let original_name = tool.name.to_string();
            let wrapped_name = format!("mcp_{}_{}", self.server_name, original_name);
            self.tool_name_map.insert(wrapped_name.clone(), original_name);

            let description = tool.description.map(|d| d.to_string()).unwrap_or_default();
            let input_schema: Value = serde_json::to_value(&*tool.input_schema)
                .unwrap_or_else(|_| json!({"type": "object", "properties": {}}));

            let normalized_schema = normalize_schema_for_openai(&input_schema);

            schemas.push(json!({
                "type": "function",
                "function": {
                    "name": wrapped_name,
                    "description": description,
                    "parameters": normalized_schema
                }
            }));
        }

        self.cached_schemas = schemas.clone();
        Ok(schemas)
    }

    pub async fn call_tool(&self, wrapped_name: &str, arguments: Value) -> Result<String, NexusError> {
        let original_name = self.tool_name_map.get(wrapped_name)
            .ok_or_else(|| NexusError::new(ErrorCode::ToolNotFound, format!("server MCP tool not found: {}", wrapped_name)))?
            .clone();

        let args_map = arguments.as_object().cloned().unwrap_or_default();
        let mut params = rmcp::model::CallToolRequestParams::default();
        params.name = original_name.into();
        params.arguments = Some(args_map);

        let result = timeout(
            Duration::from_secs(self.tool_timeout),
            self.client.call_tool(params),
        )
        .await
        .map_err(|_| NexusError::new(ErrorCode::ToolTimeout, format!("server MCP tool timeout after {}s", self.tool_timeout)))?
        .map_err(|e| NexusError::new(ErrorCode::McpCallFailed, format!("server MCP tool call failed: {}", e)))?;

        if result.is_error == Some(true) {
            let text = extract_text(&result.content);
            return Err(NexusError::new(ErrorCode::McpCallFailed, if text.is_empty() { "MCP tool returned error".into() } else { text }));
        }

        Ok(extract_text(&result.content))
    }
}

fn extract_text(content: &[rmcp::model::Content]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for item in content {
        if let rmcp::model::RawContent::Text(t) = &item.raw {
            writeln!(out, "{}", t.text).ok();
        }
    }
    out.trim().to_string()
}

/// Manager for all server-side MCP sessions.
pub struct ServerMcpManager {
    sessions: HashMap<String, ServerMcpSession>,
}

impl ServerMcpManager {
    pub fn new() -> Self {
        Self { sessions: HashMap::new() }
    }

    pub async fn initialize(&mut self, entries: &[McpServerEntry]) {
        // Clear old sessions
        self.sessions.clear();

        for entry in entries.iter().filter(|e| e.enabled) {
            match ServerMcpSession::connect(entry).await {
                Ok(mut session) => {
                    match session.list_tools().await {
                        Ok(schemas) => {
                            info!("server MCP '{}': discovered {} tools", entry.name, schemas.len());
                        }
                        Err(e) => {
                            warn!("server MCP '{}': list_tools failed: {}", entry.name, e);
                        }
                    }
                    self.sessions.insert(entry.name.clone(), session);
                }
                Err(e) => {
                    warn!("server MCP '{}': failed to connect: {}", entry.name, e);
                }
            }
        }
    }

    /// Get all tool schemas from server MCP sessions.
    pub fn all_tool_schemas(&self) -> Vec<Value> {
        self.sessions.values()
            .flat_map(|s| s.cached_schemas.iter().cloned())
            .collect()
    }

    /// Get session that owns a tool by wrapped name.
    pub fn find_tool_session(&self, wrapped_name: &str) -> Option<&ServerMcpSession> {
        self.sessions.values().find(|s| s.tool_name_map.contains_key(wrapped_name))
    }

    /// Call a tool by wrapped name.
    pub async fn call_tool(&self, wrapped_name: &str, arguments: Value) -> Result<String, NexusError> {
        let session = self.find_tool_session(wrapped_name)
            .ok_or_else(|| NexusError::new(ErrorCode::ToolNotFound, format!("no server MCP session has tool: {}", wrapped_name)))?;
        session.call_tool(wrapped_name, arguments).await
    }
}

