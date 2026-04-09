//! Server-side MCP client manager. Admin-configured MCP servers that run on the server.
//! Tools appear with device_name="server" in the tool schema.

use nexus_common::consts::DEFAULT_MCP_TOOL_TIMEOUT_SEC;
use nexus_common::mcp_utils::normalize_schema_for_openai;
use nexus_common::protocol::McpServerEntry;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, RawContent, Tool as McpTool};
use rmcp::service::RunningService;
use rmcp::transport::child_process::TokioChildProcess;
use serde_json::Value;
use std::collections::HashMap;
use tracing::{error, info, warn};

pub struct McpSession {
    pub server_name: String,
    pub tool_timeout: u64,
    service: RunningService<rmcp::RoleClient, ()>,
    tools: Vec<McpTool>,
}

pub struct ServerMcpManager {
    sessions: HashMap<String, McpSession>,
}

impl ServerMcpManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Initialize MCP servers from config.
    pub async fn initialize(&mut self, servers: &[McpServerEntry]) {
        for entry in servers.iter().filter(|e| e.enabled) {
            match start_session(entry).await {
                Ok(session) => {
                    info!(
                        "Server MCP started: {} ({} tools)",
                        entry.name,
                        session.tools.len()
                    );
                    self.sessions.insert(entry.name.clone(), session);
                }
                Err(e) => error!("Server MCP '{}' failed: {e}", entry.name),
            }
        }
    }

    /// Reinitialize: stop all, start new config.
    pub async fn reinitialize(&mut self, servers: &[McpServerEntry]) {
        self.sessions.clear();
        self.initialize(servers).await;
    }

    /// Get all tool schemas (prefixed with mcp_{server_name}_{tool_name}).
    pub fn tool_schemas(&self) -> Vec<Value> {
        self.sessions
            .values()
            .flat_map(|s| {
                s.tools.iter().map(move |t| {
                    let name = format!("mcp_{}_{}", s.server_name, t.name);
                    let desc = t.description.as_deref().unwrap_or("MCP tool");
                    let params = {
                        let v = serde_json::to_value(&*t.input_schema).unwrap_or_default();
                        normalize_schema_for_openai(&v)
                    };
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": name,
                            "description": desc,
                            "parameters": params,
                        }
                    })
                })
            })
            .collect()
    }

    /// Call an MCP tool by prefixed name. Extracts server name from prefix.
    pub async fn call_tool(&self, prefixed_name: &str, args: Value) -> Result<String, String> {
        let rest = prefixed_name
            .strip_prefix("mcp_")
            .ok_or_else(|| format!("Not an MCP tool: {prefixed_name}"))?;

        for (name, session) in &self.sessions {
            if let Some(tool_name) = rest.strip_prefix(&format!("{name}_")) {
                return call_mcp_tool(session, tool_name, args).await;
            }
        }
        Err(format!("Server MCP not found for: {prefixed_name}"))
    }

    /// Check if a prefixed tool name belongs to server MCP.
    pub fn has_tool(&self, prefixed_name: &str) -> bool {
        if let Some(rest) = prefixed_name.strip_prefix("mcp_") {
            self.sessions
                .keys()
                .any(|name| rest.starts_with(&format!("{name}_")))
        } else {
            false
        }
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }
}

async fn start_session(entry: &McpServerEntry) -> Result<McpSession, String> {
    let mut cmd = tokio::process::Command::new(&entry.command);
    cmd.args(&entry.args);
    if let Some(env) = &entry.env {
        for (k, v) in env {
            cmd.env(k, v);
        }
    }

    let child = TokioChildProcess::new(cmd).map_err(|e| format!("spawn '{}': {e}", entry.name))?;

    let service = ().serve(child).await.map_err(|e| format!("init '{}': {e}", entry.name))?;

    let tools = service
        .list_all_tools()
        .await
        .map_err(|e| format!("tools/list '{}': {e}", entry.name))?;

    Ok(McpSession {
        server_name: entry.name.clone(),
        tool_timeout: entry.tool_timeout.unwrap_or(DEFAULT_MCP_TOOL_TIMEOUT_SEC),
        service,
        tools,
    })
}

async fn call_mcp_tool(
    session: &McpSession,
    tool_name: &str,
    args: Value,
) -> Result<String, String> {
    let timeout = std::time::Duration::from_secs(session.tool_timeout);
    let args_map: serde_json::Map<String, Value> = match args {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };

    let params = CallToolRequestParams::new(tool_name.to_string()).with_arguments(args_map);

    let result = tokio::time::timeout(timeout, async {
        session
            .service
            .call_tool(params)
            .await
            .map_err(|e| format!("MCP call failed: {e}"))
    })
    .await
    .map_err(|_| {
        format!(
            "MCP tool '{tool_name}' timed out after {}s",
            session.tool_timeout
        )
    })?;

    let result = result?;

    let output: String = result
        .content
        .iter()
        .filter_map(|item| match &item.raw {
            RawContent::Text(t) => Some(t.text.clone()),
            _ => {
                warn!("Skipping non-text MCP content");
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    Ok(output)
}
