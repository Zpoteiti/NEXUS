/// 职责边界：
/// 1. 负责在 Client 启动时，收集本地的"物理环境"信息 (OS 类型、架构、当前 workspace 等)。
/// 2. 负责扫描并聚合所有可用的工具：
///    - 收集内置的原生工具 (如 shell)。
///    - 调用 `mcp_client.rs` 收集外部挂载的工具。
/// 3. 将聚合后的 Schema 列表组装成 `RegisterTools` 消息发给 Server。

use serde_json::Value;
use std::sync::LazyLock;
use tokio::sync::RwLock;

use crate::config::McpServerConfig;
use crate::executor::LOCAL_TOOL_REGISTRY;
use crate::mcp_client::McpClientManager;

/// 全局 MCP 客户端管理器（跨心跳复用）
static MCP_MANAGER: LazyLock<RwLock<McpClientManager>> =
    LazyLock::new(|| RwLock::new(McpClientManager::new()));

/// 发现并聚合所有可用工具的 Schema。
///
/// 返回: (tools schemas, hash)
pub async fn discover_all(
    mcp_servers: &[McpServerConfig],
) -> (Vec<Value>, String) {
    let mut all_schemas = Vec::new();

    // 1. 内置工具
    all_schemas.extend(discover_builtin_tools().iter().cloned());

    // 2. MCP 工具 — initialize if not yet done, then collect schemas
    let mcp_tools = discover_mcp_tools_internal(mcp_servers).await;
    all_schemas.extend(mcp_tools);

    let hash = compute_hash(&all_schemas);

    (all_schemas, hash)
}

/// Hash of the last MCP config used for initialization.
static MCP_CONFIG_HASH: LazyLock<RwLock<Option<String>>> = LazyLock::new(|| RwLock::new(None));

/// 初始化（或重新初始化）MCP 客户端管理器。
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

/// 内部调用：确保 MCP 已初始化（如果配置变了则重新初始化），然后收集所有 MCP 工具 schemas。
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

    // Collect tool schemas from all connected MCP sessions
    let mut manager = MCP_MANAGER.write().await;
    let server_names: Vec<String> = manager.server_names().iter().map(|s| s.to_string()).collect();
    let mut all_schemas = Vec::new();
    for name in &server_names {
        if let Some(session) = manager.get_session_mut(name) {
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

/// 内置工具 Schema 发现（缓存结果）。
fn discover_builtin_tools() -> &'static [Value] {
    static CACHED: LazyLock<Vec<Value>> = LazyLock::new(|| {
        LOCAL_TOOL_REGISTRY
            .values()
            .map(|t| t.schema())
            .collect()
    });
    &*CACHED
}

/// 计算任意可序列化对象的哈希。
fn compute_hash<T: serde::Serialize>(value: &T) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let serialized = serde_json::to_string(value).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    serialized.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// 获取 MCP manager（供 executor 调用工具时使用）
pub async fn get_mcp_manager() -> tokio::sync::RwLockReadGuard<'static, McpClientManager> {
    MCP_MANAGER.read().await
}
