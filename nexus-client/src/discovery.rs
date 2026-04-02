/// 职责边界：
/// 1. 负责在 Client 启动时，收集本地的"物理环境"信息 (OS 类型、架构、当前 workspace 等)。
/// 2. 负责扫描并聚合所有可用的工具：
///    - 收集内置的原生工具 (如 shell)。
///    - 调用 `mcp_client.rs` 收集外部挂载的工具。
///    - 调用 `skills.rs` 扫描并聚合自定义 Skill 工具。
/// 3. 将聚合后的 Schema 列表组装成 `RegisterTools` 消息发给 Server。
///
/// 统一发现（方案 B）：
/// - discovery.rs 统一调用链：discover_all() → 内置工具 + MCP 工具 + Skills
/// - MCP 和 Skills 的热加载检测在同一处管理

use nexus_common::protocol::SkillFull;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::LazyLock;
use tokio::sync::RwLock;

use crate::config::McpServerConfig;
use crate::executor::LOCAL_TOOL_REGISTRY;
use crate::mcp_client::McpClientManager;
use crate::skills;

/// 全局 MCP 客户端管理器（跨心跳复用）
static MCP_MANAGER: LazyLock<RwLock<McpClientManager>> =
    LazyLock::new(|| RwLock::new(McpClientManager::new()));

/// 发现并聚合所有可用工具的 Schema 和 Skill 全量信息。
///
/// 返回: (tools schemas, skills 全量列表, unified_hash)
/// unified_hash = hash(工具 schemas + 所有 Skill 的 name/description/content/always)
pub async fn discover_all(
    mcp_servers: &[McpServerConfig],
    skills_dir: &PathBuf,
) -> (Vec<Value>, Vec<SkillFull>, String) {
    let mut all_schemas = Vec::new();

    // 1. 内置工具
    all_schemas.extend(discover_builtin_tools().iter().cloned());

    // 2. MCP 工具 — initialize if not yet done, then collect schemas
    let mcp_tools = discover_mcp_tools_internal(mcp_servers).await;
    all_schemas.extend(mcp_tools);

    // 3. Skills（全量：always=true 带正文，always=false 不带正文）
    let skills_full = skills::scan_skills(skills_dir);

    // 单一 unified hash：工具 schemas + 所有 skill 的 name/description/content/always
    let hash = compute_hash(&(&all_schemas, &skills_full));

    (all_schemas, skills_full, hash)
}

/// 发现并聚合所有可用工具的 Schema（不含 Skills）。
#[allow(dead_code)]
pub async fn discover_tools(
    mcp_servers: &[McpServerConfig],
    skills_dir: &PathBuf,
) -> Vec<Value> {
    let (schemas, _, _) = discover_all(mcp_servers, skills_dir).await;
    schemas
}

/// Whether MCP has been initialized at least once.
static MCP_INITIALIZED: LazyLock<RwLock<bool>> = LazyLock::new(|| RwLock::new(false));

/// 初始化 MCP 客户端管理器。
pub async fn init_mcp(mcp_servers: &[McpServerConfig]) {
    let mut manager = MCP_MANAGER.write().await;
    if let Err(e) = manager.initialize(mcp_servers).await {
        tracing::warn!("failed to initialize MCP servers: {}", e);
    }
    let mut initialized = MCP_INITIALIZED.write().await;
    *initialized = true;
}

/// 内部调用：确保 MCP 已初始化，然后收集所有 MCP 工具 schemas。
async fn discover_mcp_tools_internal(mcp_servers: &[McpServerConfig]) -> Vec<Value> {
    // Initialize MCP on first call if servers are configured
    if !mcp_servers.is_empty() {
        let initialized = *MCP_INITIALIZED.read().await;
        if !initialized {
            init_mcp(mcp_servers).await;
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

/// 发现 MCP 工具（供外部调用，初始化并返回 schema）。
#[allow(dead_code)]
pub async fn discover_mcp_tools(mcp_servers: &[McpServerConfig]) -> Vec<Value> {
    discover_mcp_tools_internal(mcp_servers).await
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
pub async fn get_mcp_manager() -> tokio::sync::RwLockWriteGuard<'static, McpClientManager> {
    MCP_MANAGER.write().await
}
