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

use nexus_common::protocol::SkillSummary;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::LazyLock;
use tokio::sync::RwLock;

use crate::config::McpServerConfig;
use crate::mcp_client::McpClientManager;
use crate::skills;

/// 全局 MCP 客户端管理器（跨心跳复用）
static MCP_MANAGER: LazyLock<RwLock<McpClientManager>> =
    LazyLock::new(|| RwLock::new(McpClientManager::new()));

/// 发现并聚合所有可用工具的 Schema 和 Skill 摘要。
///
/// 返回: (tools schemas, skill summaries, tools_hash, skills_hash)
pub async fn discover_all(
    _mcp_servers: &[McpServerConfig],
    skills_dir: &PathBuf,
) -> (Vec<Value>, Vec<SkillSummary>, String, String) {
    let mut all_schemas = Vec::new();

    // 1. 内置工具
    all_schemas.extend(discover_builtin_tools());

    // 2. MCP 工具
    let mcp_tools = discover_mcp_tools_internal().await;
    all_schemas.extend(mcp_tools);

    // 3. Skills
    let skill_summaries = skills::scan_skills(skills_dir);

    let tools_hash = compute_hash(&all_schemas);
    let skills_hash = compute_hash(&skill_summaries);

    (all_schemas, skill_summaries, tools_hash, skills_hash)
}

/// 发现并聚合所有可用工具的 Schema（不含 Skills）。
#[allow(dead_code)]
pub async fn discover_tools(
    mcp_servers: &[McpServerConfig],
    skills_dir: &PathBuf,
) -> Vec<Value> {
    let (schemas, _, _, _) = discover_all(mcp_servers, skills_dir).await;
    schemas
}

/// 初始化 MCP 客户端管理器。
#[allow(dead_code)]
pub async fn init_mcp(mcp_servers: &[McpServerConfig]) {
    let mut manager = MCP_MANAGER.write().await;
    if let Err(e) = manager.initialize(mcp_servers).await {
        tracing::warn!("failed to initialize MCP servers: {}", e);
    }
}

/// 内部调用：发现 MCP 工具。
async fn discover_mcp_tools_internal() -> Vec<Value> {
    // 简化处理：返回空，实际的工具 schema 需要在 session 初始化时获取
    // 此处返回空是安全的，因为 MCP 工具发现会延迟到 RegisterTools 时
    Vec::new()
}

/// 发现 MCP 工具（供外部调用，初始化并返回 schema）。
#[allow(dead_code)]
pub async fn discover_mcp_tools(_mcp_servers: &[McpServerConfig]) -> Vec<Value> {
    // MCP 工具发现目前返回空，实际的 schema 在 session 初始化时通过 MCP manager 获取
    Vec::new()
}

/// 内置工具 Schema 发现。
///
/// 当前只有一个 `shell` 工具。
fn discover_builtin_tools() -> Vec<Value> {
    vec![json!({
        "type": "function",
        "function": {
            "name": "shell",
            "description": "Execute a shell command on this device and return its stdout/stderr output.",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute."
                    },
                    "timeout_sec": {
                        "type": "integer",
                        "description": "Optional execution timeout in seconds. Defaults to 60, max 600.",
                        "minimum": 1,
                        "maximum": 600
                    },
                    "working_dir": {
                        "type": "string",
                        "description": "Optional working directory for the command. Must be within workspace."
                    }
                },
                "required": ["command"]
            }
        }
    })]
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
