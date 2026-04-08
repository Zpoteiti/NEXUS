/// Responsibility boundary:
/// 1. At client startup, collects local environment info (OS type, architecture, workspace, etc.).
/// 2. Scans and aggregates all available tools:
///    - Collects built-in native tools (e.g. shell).
///    - Calls `mcp_client.rs` to collect externally mounted tools.
/// 3. Assembles aggregated schemas into a `RegisterTools` message for the server.

use serde_json::Value;
use std::sync::LazyLock;
use tokio::sync::RwLock;

use crate::config::McpServerConfig;
use crate::executor::LOCAL_TOOL_REGISTRY;
use crate::mcp_client::McpClientManager;

/// Global MCP client manager (reused across heartbeats).
static MCP_MANAGER: LazyLock<RwLock<McpClientManager>> =
    LazyLock::new(|| RwLock::new(McpClientManager::new()));

/// Discover and aggregate all available tool schemas.
///
/// Returns: (tool schemas, hash)
pub async fn discover_all(
    mcp_servers: &[McpServerConfig],
) -> (Vec<Value>, String) {
    let mut all_schemas = Vec::new();

    // 1. Built-in tools
    all_schemas.extend(discover_builtin_tools().iter().cloned());

    // 2. MCP tools -- initialize if not yet done, then collect schemas
    let mcp_tools = discover_mcp_tools_internal(mcp_servers).await;
    all_schemas.extend(mcp_tools);

    let hash = compute_hash(&all_schemas);

    (all_schemas, hash)
}

/// Hash of the last MCP config used for initialization.
static MCP_CONFIG_HASH: LazyLock<RwLock<Option<String>>> = LazyLock::new(|| RwLock::new(None));

/// Initialize (or reinitialize) the MCP client manager.
pub async fn init_mcp(mcp_servers: &[McpServerConfig]) {
    let mut manager = MCP_MANAGER.write().await;
    // Replace with a fresh manager to cleanly drop old sessions
    *manager = McpClientManager::new();
    if let Err(e) = manager.initialize(mcp_servers).await {
        tracing::warn!("failed to initialize MCP servers: {}", e);
    }
    // Store config hash to detect changes on next heartbeat
    let hash = compute_hash(&mcp_servers.iter().map(|s| (&s.name, &s.command, &s.args, &s.env)).collect::<Vec<_>>());
    *MCP_CONFIG_HASH.write().await = Some(hash);
}

/// Internal: ensure MCP is initialized (reinitialize if config changed), then collect all MCP tool schemas.
async fn discover_mcp_tools_internal(mcp_servers: &[McpServerConfig]) -> Vec<Value> {
    if !mcp_servers.is_empty() {
        let current_hash = compute_hash(&mcp_servers.iter().map(|s| (&s.name, &s.command, &s.args, &s.env)).collect::<Vec<_>>());
        let stored_hash = MCP_CONFIG_HASH.read().await.clone();
        if stored_hash.as_deref() != Some(&current_hash) {
            tracing::info!("MCP config changed, reinitializing MCP servers");
            init_mcp(mcp_servers).await;
        }
    } else {
        // No MCP servers configured — clear manager if it had sessions before
        let stored = MCP_CONFIG_HASH.read().await.clone();
        if stored.is_some() {
            let mut manager = MCP_MANAGER.write().await;
            *manager = McpClientManager::new();
            *MCP_CONFIG_HASH.write().await = None;
        }
    }

    // Collect tool schemas from all connected MCP sessions.
    // Only a read lock is needed: `list_tools` takes `&self` via interior mutability.
    let manager = MCP_MANAGER.read().await;
    let server_names: Vec<String> = manager.server_names().iter().map(|s| s.to_string()).collect();
    let mut all_schemas = Vec::new();
    for name in &server_names {
        if let Some(session) = manager.get_session(name) {
            match session.list_tools().await {
                Ok(schemas) => {
                    tracing::debug!("MCP server '{}': collected {} tool schemas", name, schemas.len());
                    all_schemas.extend(schemas);
                }
                Err(e) => {
                    tracing::warn!("MCP server '{}': failed to list tools during discovery: {}", name, e);
                }
            }
        }
    }
    all_schemas
}

/// Built-in tool schema discovery (cached result).
fn discover_builtin_tools() -> &'static [Value] {
    static CACHED: LazyLock<Vec<Value>> = LazyLock::new(|| {
        LOCAL_TOOL_REGISTRY
            .values()
            .map(|t| t.schema())
            .collect()
    });
    &*CACHED
}

/// Compute a hash of any serializable object.
pub fn compute_hash<T: serde::Serialize>(value: &T) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let serialized = serde_json::to_string(value).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    serialized.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Get the MCP manager (for use when executor calls tools).
pub async fn get_mcp_manager() -> tokio::sync::RwLockReadGuard<'static, McpClientManager> {
    MCP_MANAGER.read().await
}
