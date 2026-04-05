/// 职责边界：
/// 1. 负责在本地启动并连接第三方 MCP Server（通过 rmcp SDK）。
/// 2. 初始化时发送 `tools/list` 获取外部工具，并在命名前加上 `mcp_` 前缀（命名隔离）。
/// 3. 收到执行请求时，将请求透传给对应的 MCP Server 并返回结果。
// TODO: migrate to NexusError when nexus-client uses nexus-common error types

use nexus_common::mcp_utils::normalize_schema_for_openai;
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::time::{timeout, Duration};

use crate::config::{McpServerConfig, TransportType};
use crate::tools::ToolError;

/// MCP Server 会话（基于 rmcp SDK）。
pub struct McpSession {
    server_name: String,
    client: rmcp::service::RunningService<rmcp::RoleClient, ()>,
    tool_name_map: HashMap<String, String>,
    tool_timeout: u64,
}

impl McpSession {
    /// 连接 MCP Server 并完成初始化握手。
    pub async fn connect(config: &McpServerConfig) -> Result<Self, String> {
        let tool_timeout = config.tool_timeout.unwrap_or(nexus_common::consts::DEFAULT_MCP_TOOL_TIMEOUT_SEC);

        let mut cmd = tokio::process::Command::new(&config.command);
        cmd.args(&config.args);
        if let Some(env) = &config.env {
            cmd.envs(env);
        }

        let transport = rmcp::transport::TokioChildProcess::new(cmd)
            .map_err(|e| format!("failed to spawn MCP server '{}': {}", config.name, e))?;

        let client = timeout(
            Duration::from_secs(30),
            rmcp::serve_client((), transport),
        )
        .await
        .map_err(|_| format!("MCP server '{}': initialization timeout", config.name))?
        .map_err(|e| format!("MCP server '{}': initialization failed: {}", config.name, e))?;

        Ok(Self {
            server_name: config.name.clone(),
            client,
            tool_name_map: HashMap::new(),
            tool_timeout,
        })
    }

    /// 列出所有可用工具。
    pub async fn list_tools(&mut self) -> Result<Vec<Value>, String> {
        let tools = self.client.list_all_tools()
            .await
            .map_err(|e| format!("MCP server '{}': list_tools failed: {}", self.server_name, e))?;

        let mut schemas = Vec::new();
        for tool in tools {
            let original_name = tool.name.to_string();
            let wrapped_name = format!("mcp_{}_{}", self.server_name, original_name);
            self.tool_name_map.insert(wrapped_name.clone(), original_name);

            let description = tool.description
                .map(|d| d.to_string())
                .unwrap_or_default();

            let input_schema: Value = serde_json::to_value(&*tool.input_schema)
                .unwrap_or_else(|_| json!({"type": "object", "properties": {}}));

            // 规范化 schema，参考 nanobot _normalize_schema_for_openai()
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

        Ok(schemas)
    }

    /// 调用 MCP 工具。
    pub async fn call_tool(
        &self,
        wrapped_name: &str,
        arguments: Value,
    ) -> Result<String, ToolError> {
        let original_name = self
            .tool_name_map
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

/// MCP 客户端全局管理器。
pub struct McpClientManager {
    sessions: HashMap<String, McpSession>,
}

impl McpClientManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// 初始化所有 MCP 服务器。
    pub async fn initialize(&mut self, servers: &[McpServerConfig]) -> Result<(), String> {
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
                Ok(mut session) => {
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

    /// 调用 MCP 工具。根据 wrapped_name 查找拥有该工具的 session。
    pub async fn call_tool(
        &self,
        wrapped_name: &str,
        arguments: Value,
    ) -> Result<String, ToolError> {
        let (server_name, session) = self.sessions.iter()
            .find(|(_, session)| session.tool_name_map.contains_key(wrapped_name))
            .ok_or_else(|| ToolError::NotFound(format!("no MCP server has tool: {}", wrapped_name)))?;
        tracing::debug!("routing MCP tool '{}' to server '{}'", wrapped_name, server_name);
        session.call_tool(wrapped_name, arguments).await
    }

    /// 获取所有已连接服务器的名称。
    pub fn server_names(&self) -> Vec<&str> {
        self.sessions.keys().map(|s| s.as_str()).collect()
    }

    /// 获取指定服务器的可变会话引用。
    pub fn get_session_mut(&mut self, server_name: &str) -> Option<&mut McpSession> {
        self.sessions.get_mut(server_name)
    }
}

impl Default for McpClientManager {
    fn default() -> Self {
        Self::new()
    }
}

