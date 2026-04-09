pub mod client;

use client::McpSession;
use nexus_common::protocol::McpServerEntry;
use serde_json::Value;
use std::collections::HashMap;
use tracing::{error, info};

/// Manages all MCP server sessions. Handles lifecycle, config diff, reinit.
pub struct McpManager {
    sessions: HashMap<String, McpSession>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Initialize MCP servers from config. Start all enabled servers.
    pub async fn initialize(&mut self, servers: &[McpServerEntry]) {
        for entry in servers.iter().filter(|e| e.enabled) {
            match McpSession::start(entry).await {
                Ok(s) => {
                    info!("MCP session started: {}", entry.name);
                    self.sessions.insert(entry.name.clone(), s);
                }
                Err(e) => {
                    error!("MCP '{}': {e}", entry.name);
                }
            }
        }
    }

    /// Apply config update: diff against current, start/stop/restart as needed.
    pub async fn apply_config(&mut self, new: &[McpServerEntry]) {
        let current: Vec<String> = self.sessions.keys().cloned().collect();
        let new_names: Vec<&str> = new.iter().map(|s| s.name.as_str()).collect();

        // Remove servers no longer in config
        for name in &current {
            if !new_names.contains(&name.as_str()) {
                self.sessions.remove(name);
            }
        }

        // Start/restart servers
        for entry in new {
            if !entry.enabled {
                self.sessions.remove(&entry.name);
                continue;
            }
            self.sessions.remove(&entry.name);
            match McpSession::start(entry).await {
                Ok(s) => {
                    self.sessions.insert(entry.name.clone(), s);
                }
                Err(e) => {
                    error!("MCP restart '{}': {e}", entry.name);
                }
            }
        }
    }

    /// Collect all MCP tool schemas for registration.
    pub fn all_tool_schemas(&self) -> Vec<Value> {
        self.sessions
            .values()
            .flat_map(|s| s.tool_schemas())
            .collect()
    }

    /// Route a tool call to the correct MCP session.
    /// Tool name format: mcp_{server_name}_{tool_name}
    pub async fn call_tool(&self, prefixed: &str, args: Value) -> Result<String, String> {
        let rest = prefixed
            .strip_prefix("mcp_")
            .ok_or_else(|| format!("not MCP: {prefixed}"))?;
        for (name, session) in &self.sessions {
            if let Some(tool) = rest.strip_prefix(&format!("{name}_")) {
                return session.call_tool(tool, args).await;
            }
        }
        Err(format!("MCP server not found for: {prefixed}"))
    }

    pub fn is_mcp_tool(name: &str) -> bool {
        name.starts_with("mcp_")
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_mcp_tool() {
        assert!(McpManager::is_mcp_tool("mcp_gh_list"));
        assert!(!McpManager::is_mcp_tool("read_file"));
    }

    #[test]
    fn test_empty_manager() {
        let m = McpManager::new();
        assert_eq!(m.session_count(), 0);
    }
}
