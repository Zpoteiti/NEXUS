//! Single MCP server session. Manages one child process + MCP protocol handshake.

use nexus_common::consts::DEFAULT_MCP_TOOL_TIMEOUT_SEC;
use nexus_common::mcp_utils::normalize_schema_for_openai;
use nexus_common::protocol::McpServerEntry;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, RawContent, Tool as McpTool};
use rmcp::service::RunningService;
use rmcp::transport::child_process::TokioChildProcess;
use serde_json::Value;
use tracing::{info, warn};

pub struct McpSession {
    pub server_name: String,
    pub tool_timeout: u64,
    service: RunningService<rmcp::RoleClient, ()>,
    tools: Vec<McpTool>,
}

impl McpSession {
    /// Start an MCP server session: spawn process, initialize, discover tools.
    pub async fn start(entry: &McpServerEntry) -> Result<Self, String> {
        info!("Starting MCP server: {}", entry.name);

        let mut cmd = tokio::process::Command::new(&entry.command);
        cmd.args(&entry.args);
        if let Some(env) = &entry.env {
            for (k, v) in env {
                cmd.env(k, v);
            }
        }

        let child =
            TokioChildProcess::new(cmd).map_err(|e| format!("spawn '{}': {e}", entry.name))?;

        let service = ().serve(child).await.map_err(|e| format!("init '{}': {e}", entry.name))?;

        let tools = service
            .list_all_tools()
            .await
            .map_err(|e| format!("tools/list '{}': {e}", entry.name))?;

        info!("MCP '{}': {} tools", entry.name, tools.len());

        Ok(Self {
            server_name: entry.name.clone(),
            tool_timeout: entry.tool_timeout.unwrap_or(DEFAULT_MCP_TOOL_TIMEOUT_SEC),
            service,
            tools,
        })
    }

    /// Get tool schemas for registration (prefixed, normalized).
    pub fn tool_schemas(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|t| {
                let name = format!("mcp_{}_{}", self.server_name, t.name);
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
            .collect()
    }

    /// Call an MCP tool by its original (unprefixed) name.
    pub async fn call_tool(&self, tool_name: &str, args: Value) -> Result<String, String> {
        let timeout = std::time::Duration::from_secs(self.tool_timeout);
        let args_map: serde_json::Map<String, Value> = match args {
            Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };

        let params = CallToolRequestParams::new(tool_name.to_string()).with_arguments(args_map);

        let result = tokio::time::timeout(timeout, async {
            self.service
                .call_tool(params)
                .await
                .map_err(|e| format!("MCP call failed: {e}"))
        })
        .await
        .map_err(|_| {
            format!(
                "MCP tool '{tool_name}' timed out after {}s",
                self.tool_timeout
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
}
