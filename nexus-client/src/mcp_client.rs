/// Responsibility boundary:
/// 1. Launches and connects to third-party MCP servers locally (via rmcp SDK).
/// 2. On init, sends `tools/list` to discover external tools and prefixes names with `mcp_` (namespace isolation).
/// 3. On execution requests, forwards them to the corresponding MCP server and returns results.

use nexus_common::mcp_utils::normalize_schema_for_openai;
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::time::{timeout, Duration};

use crate::config::{McpServerConfig, TransportType};
use crate::tools::ToolError;

/// MCP Server session (based on rmcp SDK).
pub struct McpSession {
    server_name: String,
    client: rmcp::service::RunningService<rmcp::RoleClient, ()>,
    /// Wrapped-name → original-name mapping. Interior-mutable so `list_tools` can take `&self`.
    pub(crate) tool_name_map: tokio::sync::RwLock<HashMap<String, String>>,
    tool_timeout: u64,
}

impl McpSession {
    /// Connect to an MCP server and complete the initialization handshake.
    pub async fn connect(config: &McpServerConfig) -> Result<Self, ToolError> {
        let tool_timeout = config.tool_timeout.unwrap_or(nexus_common::consts::DEFAULT_MCP_TOOL_TIMEOUT_SEC);

        let mut cmd = tokio::process::Command::new(&config.command);
        cmd.args(&config.args);
        if let Some(env) = &config.env {
            cmd.envs(env);
        }

        let transport = rmcp::transport::TokioChildProcess::new(cmd)
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to spawn MCP server '{}': {}", config.name, e)))?;

        let client = timeout(
            Duration::from_secs(30),
            rmcp::serve_client((), transport),
        )
        .await
        .map_err(|_| ToolError::ExecutionFailed(format!("MCP server '{}': initialization timeout", config.name)))?
        .map_err(|e| ToolError::ExecutionFailed(format!("MCP server '{}': initialization failed: {}", config.name, e)))?;

        Ok(Self {
            server_name: config.name.clone(),
            client,
            tool_name_map: tokio::sync::RwLock::new(HashMap::new()),
            tool_timeout,
        })
    }

    /// List all available tools.
    ///
    /// Takes `&self` (not `&mut self`) so callers only need a read lock on the manager.
    /// The internal `tool_name_map` is updated via interior mutability.
    pub async fn list_tools(&self) -> Result<Vec<Value>, ToolError> {
        let tools = self.client.list_all_tools()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("MCP server '{}': list_tools failed: {}", self.server_name, e)))?;

        let mut new_map = HashMap::new();
        let mut schemas = Vec::new();
        for tool in tools {
            let original_name = tool.name.to_string();
            let wrapped_name = format!("mcp_{}_{}", self.server_name, original_name);
            new_map.insert(wrapped_name.clone(), original_name);

            let description = tool.description
                .map(|d| d.to_string())
                .unwrap_or_default();

            let input_schema: Value = serde_json::to_value(&*tool.input_schema)
                .unwrap_or_else(|_| json!({"type": "object", "properties": {}}));

            // Normalize schema for OpenAI function calling format
            let normalized_schema = normalize_schema_for_openai(&input_schema);

            let schema = json!({
                "type": "function",
                "function": {
                    "name": wrapped_name,
                    "description": description,
                    "parameters": normalized_schema
                }
            });
            schemas.push(schema);
        }

        // Atomically replace the name map
        *self.tool_name_map.write().await = new_map;

        Ok(schemas)
    }

    /// Call an MCP tool.
    pub async fn call_tool(
        &self,
        wrapped_name: &str,
        arguments: Value,
    ) -> Result<String, ToolError> {
        let original_name = self
            .tool_name_map
            .read()
            .await
            .get(wrapped_name)
            .ok_or_else(|| ToolError::NotFound(format!("MCP tool not found: {}", wrapped_name)))?
            .clone();

        let args_map = arguments.as_object()
            .cloned()
            .unwrap_or_default();

        let mut params = rmcp::model::CallToolRequestParams::default();
        params.name = original_name.into();
        params.arguments = Some(args_map);

        let result = timeout(
            Duration::from_secs(self.tool_timeout),
            self.client.call_tool(params),
        )
        .await
        .map_err(|_| ToolError::Timeout(self.tool_timeout))?
        .map_err(|e| ToolError::ExecutionFailed(format!("MCP tool call failed: {}", e)))?;

        if result.is_error == Some(true) {
            let error_text = extract_text_from_content(&result.content);
            return Err(ToolError::ExecutionFailed(
                if error_text.is_empty() { "MCP tool returned error".to_string() } else { error_text }
            ));
        }

        Ok(extract_text_from_content(&result.content))
    }
}

/// Extract text from rmcp Content list.
fn extract_text_from_content(content: &[rmcp::model::Content]) -> String {
    use std::fmt::Write;
    let mut output = String::new();
    for item in content {
        match &item.raw {
            rmcp::model::RawContent::Text(text) => {
                writeln!(output, "{}", text.text).ok();
            }
            rmcp::model::RawContent::Image(img) => {
                writeln!(output, "[Image: {}, size={}]", img.mime_type, img.data.len()).ok();
            }
            rmcp::model::RawContent::Resource(res) => {
                writeln!(output, "[Resource: {:?}]", res.resource).ok();
            }
            _ => {
                writeln!(output, "[Content block]").ok();
            }
        }
    }
    output.trim().to_string()
}

/// MCP client global manager.
pub struct McpClientManager {
    sessions: HashMap<String, McpSession>,
}

impl McpClientManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Initialize all MCP servers.
    pub async fn initialize(&mut self, servers: &[McpServerConfig]) -> Result<(), ToolError> {
        for server in servers.iter().filter(|s| s.enabled) {
            if server.transport_type != TransportType::Stdio {
                tracing::warn!(
                    "MCP server '{}': transport '{:?}' not implemented yet, skipping",
                    server.name,
                    server.transport_type
                );
                continue;
            }

            match McpSession::connect(server).await {
                Ok(session) => {
                    match session.list_tools().await {
                        Ok(schemas) => {
                            tracing::info!(
                                "MCP server '{}': discovered {} tools",
                                server.name,
                                schemas.len()
                            );
                        }
                        Err(e) => {
                            tracing::warn!("MCP server '{}': failed to list tools: {}", server.name, e);
                        }
                    }
                    self.sessions.insert(server.name.clone(), session);
                }
                Err(e) => {
                    tracing::warn!("MCP server '{}': failed to connect: {}", server.name, e);
                }
            }
        }
        Ok(())
    }

    /// Call an MCP tool. Uses the reverse index for O(1) server lookup.
    pub async fn call_tool(
        &self,
        wrapped_name: &str,
        arguments: Value,
    ) -> Result<String, ToolError> {
        // O(1) lookup via reverse index
        let index = crate::discovery::get_mcp_tool_index().await;
        let server_name = index.get(wrapped_name)
            .ok_or_else(|| ToolError::NotFound(format!("no MCP server has tool: {}", wrapped_name)))?
            .clone();
        drop(index);

        let session = self.sessions.get(server_name.as_str())
            .ok_or_else(|| ToolError::NotFound(format!("MCP server '{}' not found", server_name)))?;
        tracing::debug!("routing MCP tool '{}' to server '{}'", wrapped_name, server_name);
        session.call_tool(wrapped_name, arguments).await
    }

    /// Get names of all connected servers.
    pub fn server_names(&self) -> Vec<&str> {
        self.sessions.keys().map(|s| s.as_str()).collect()
    }

    /// Get a shared reference to the session for a specific server.
    pub fn get_session(&self, server_name: &str) -> Option<&McpSession> {
        self.sessions.get(server_name)
    }
}

impl Default for McpClientManager {
    fn default() -> Self {
        Self::new()
    }
}

