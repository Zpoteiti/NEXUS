/// 职责边界：
/// 1. 负责在 Client 启动时，收集本地的"物理环境"信息 (OS 类型、架构、当前 workspace 等)。
/// 2. 负责扫描并聚合所有可用的工具：
///    - 收集内置的原生工具 (如 shell, fs)。
///    - 调用 `mcp_client.rs` 收集外部挂载的工具。
///    - (未来) 扫描特定目录下的自定义 .sh / .py 脚本并自动封装成工具。
/// 3. 将聚合后的 Schema 列表组装成 `RegisterTools` 消息发给 Server。
///
/// 参考 nanobot：
/// - 替代 `nanobot` 中启动时的工具注册表加载阶段。

use serde_json::Value;

use crate::tools;

/// M2 阶段：仅返回内置工具的 Schema 列表。
/// MCP 工具和 Skill 工具留空，后续里程碑填入。
pub fn discover_all_tools() -> Vec<Value> {
    let schemas = tools::get_all_schemas();

    // TODO M3: extend with MCP tool schemas from mcp_client
    // TODO M4: extend with Skill tool schemas from skills scanner

    schemas
}
