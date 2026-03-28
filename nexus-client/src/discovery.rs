/// 职责边界：
/// 1. 负责在 Client 启动时，收集本地的"物理环境"信息 (OS 类型、架构、当前 workspace 等)。
/// 2. 负责扫描并聚合所有可用的工具：
///    - 收集内置的原生工具 (如 shell, fs)。
///    - 调用 `mcp_client.rs` 收集外部挂载的工具。
///    - 调用 `skills.rs` 扫描并聚合自定义 Skill 工具。
/// 3. 将聚合后的 Schema 列表组装成 `RegisterTools` 消息发给 Server。

use serde_json::{json, Value};
use std::path::PathBuf;

/// 收集本地系统环境信息，供 Server 构建上下文使用。
pub async fn gather_system_context() -> Value {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    json!({
        "os": os,
        "arch": arch,
        "home": home,
        "cwd": cwd,
    })
}

/// 发现并聚合所有可用工具的 Schema。
///
/// 工具来源优先级：
/// 1. 内置工具（shell 工具，discover_builtin_tools）
/// 2. MCP 工具（mcp_client.rs，待实现时接入）
/// 3. Skill 工具（skills.rs，待实现时接入）
///
/// 所有 Schema 均以 OpenAI function calling 格式返回：
/// { "type": "function", "function": { "name": ..., "description": ..., "parameters": {...} } }
pub async fn discover_all_tools(
    _mcp_servers: Vec<crate::config::McpServerConfig>,
    _skills_dir: PathBuf,
) -> Vec<Value> {
    let mut all_schemas = Vec::new();

    // 1. 内置工具
    all_schemas.extend(discover_builtin_tools());

    // 2. MCP 工具（mcp_client.rs 实现后接入）
    // let mcp_tools = mcp_client::discover_mcp_tools(mcp_servers).await;
    // all_schemas.extend(mcp_tools);

    // 3. Skill 工具（skills.rs 实现后接入）
    // let skill_tools = skills::discover_skill_tools(skills_dir).await;
    // all_schemas.extend(skill_tools);

    all_schemas
}

/// 内置工具 Schema 发现。
///
/// 当前只有一个 `run_shell_command` 工具。
/// 其余内置工具（fs、env 等）在实现时加入。
fn discover_builtin_tools() -> Vec<Value> {
    vec![json!({
        "type": "function",
        "function": {
            "name": "run_shell_command",
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
                        "description": "Optional execution timeout in seconds. Defaults to 60.",
                        "default": 60
                    }
                },
                "required": ["command"]
            }
        }
    })]
}
