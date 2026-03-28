/// 职责边界：
/// 1. 接收 `protocol::ExecuteToolRequest`。
/// 2. 扮演"路由器"角色：
///    - 通过 `LOCAL_TOOL_REGISTRY` 查找内置工具并执行。
///    - 如果 tool_name 以 "mcp_" 开头，解析出 server_name 和 tool_name，转发给 MCP 会话。
/// 3. 将任何模块返回的 Ok(String) 或 Err(String) 统一包装为 `protocol::ToolExecutionResult` 向上层返回。

use nexus_common::consts::EXIT_CODE_SUCCESS;
use nexus_common::protocol::{ExecuteToolRequest, ToolExecutionResult};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::discovery;
use crate::mcp_client::parse_mcp_tool_name;
use crate::tools::fs::{ListDirTool, ReadFileTool, StatTool, WriteFileTool};
use crate::tools::shell::ShellTool;
use crate::tools::{LocalTool, ToolError};

/// 本地工具注册表 — executor.rs 和 discovery.rs 共用的单一样本来源。
pub static LOCAL_TOOL_REGISTRY: LazyLock<HashMap<&'static str, Box<dyn LocalTool>>> =
    LazyLock::new(|| {
        HashMap::from_iter([
            ("shell", Box::new(ShellTool::new()) as Box<dyn LocalTool>),
            ("read_file", Box::new(ReadFileTool::new())),
            ("write_file", Box::new(WriteFileTool::new())),
            ("list_dir", Box::new(ListDirTool::new())),
            ("stat", Box::new(StatTool::new())),
        ])
    });

/// 执行工具调用请求。
pub async fn execute_tool_request(req: ExecuteToolRequest) -> ToolExecutionResult {
    let request_id = req.request_id.clone();
    let tool_name = req.tool_name.clone();
    let arguments = req.arguments;

    // 路由到对应工具
    let result = if let Some(tool) = LOCAL_TOOL_REGISTRY.get(tool_name.as_str()) {
        tool.execute(arguments).await
    } else if let Some((server_name, _tool_name)) = parse_mcp_tool_name(&tool_name) {
        // MCP 工具
        let mut manager = discovery::get_mcp_manager().await;
        manager
            .call_tool(server_name, &tool_name, arguments)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))
    } else {
        Err(ToolError::NotFound(format!("unknown tool: {}", tool_name)))
    };

    match result {
        Ok(output) => ToolExecutionResult {
            request_id,
            exit_code: EXIT_CODE_SUCCESS,
            output,
        },
        Err(e) => ToolExecutionResult {
            request_id,
            exit_code: e.exit_code(),
            output: e.to_string(),
        },
    }
}

/// 从 schema 校验必填参数。
#[allow(dead_code)]
pub fn validate_required_params(
    _tool_name: &str,
    arguments: &Value,
    schema: &Value,
) -> Option<String> {
    let params = schema.get("function")?.get("parameters")?;
    let required = params.get("required")?.as_array()?;

    for req in required {
        let field = req.as_str()?;
        if !arguments.get(field).is_some() {
            return Some(field.to_string());
        }
    }
    None
}
