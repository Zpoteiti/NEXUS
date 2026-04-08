/// Responsibility boundary:
/// 1. Receives `protocol::ExecuteToolRequest`.
/// 2. Acts as a router:
///    - Looks up built-in tools via `LOCAL_TOOL_REGISTRY` and executes them.
///    - If tool_name starts with "mcp_", extracts server_name and tool_name and forwards to the MCP session.
/// 3. Wraps any Ok(String) or Err(String) from modules into `protocol::ToolExecutionResult` for the upper layer.

use nexus_common::consts::{EXIT_CODE_SUCCESS, EXIT_CODE_TIMEOUT};
use nexus_common::protocol::{ExecuteToolRequest, FsPolicy, ToolExecutionResult};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use tokio::sync::RwLock;
use tokio::time::{timeout, Duration};

use crate::discovery;
use crate::tools::edit::EditFileTool;
use crate::tools::fs::{ListDirTool, ReadFileTool, StatTool, WriteFileTool};
use crate::tools::shell::ShellTool;
use crate::tools::{LocalTool, ToolError};

/// Top-level execution timeout in seconds.
const EXECUTOR_TIMEOUT_SEC: u64 = 120;

/// Filesystem tool names that require policy-aware dispatch.
const FS_TOOLS: &[&str] = &["read_file", "write_file", "edit_file", "list_dir", "stat"];

/// Local tool registry -- single source of truth shared by executor.rs and discovery.rs.
pub static LOCAL_TOOL_REGISTRY: LazyLock<HashMap<&'static str, Box<dyn LocalTool>>> =
    LazyLock::new(|| {
        HashMap::from_iter([
            ("shell", Box::new(ShellTool::new()) as Box<dyn LocalTool>),
            ("read_file", Box::new(ReadFileTool::new())),
            ("write_file", Box::new(WriteFileTool::new())),
            ("list_dir", Box::new(ListDirTool::new())),
            ("stat", Box::new(StatTool::new())),
            ("edit_file", Box::new(EditFileTool::new())),
        ])
    });

/// Execute a tool call request with a 120s top-level timeout guard.
pub async fn execute_tool_request(
    req: ExecuteToolRequest,
    fs_policy: &Arc<RwLock<FsPolicy>>,
) -> ToolExecutionResult {
    let request_id = req.request_id.clone();
    match timeout(
        Duration::from_secs(EXECUTOR_TIMEOUT_SEC),
        execute_tool_inner(req, fs_policy),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => ToolExecutionResult {
            request_id,
            exit_code: EXIT_CODE_TIMEOUT,
            output: format!("execution timed out after {}s", EXECUTOR_TIMEOUT_SEC),
        },
    }
}

/// Inner implementation without top-level timeout.
async fn execute_tool_inner(
    req: ExecuteToolRequest,
    fs_policy: &Arc<RwLock<FsPolicy>>,
) -> ToolExecutionResult {
    let request_id = req.request_id.clone();
    let tool_name = req.tool_name.clone();
    let arguments = req.arguments;

    // Route to the corresponding tool
    let result = if FS_TOOLS.contains(&tool_name.as_str()) || tool_name == "shell" {
        let policy = fs_policy.read().await.clone();
        if tool_name == "shell" {
            // Shell policy guard: enforce FsPolicy on shell commands before execution
            let cmd = arguments
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Err(e) = crate::tools::shell::guard_command_policy(cmd, &policy) {
                return ToolExecutionResult {
                    request_id,
                    exit_code: e.exit_code(),
                    output: e.to_string(),
                };
            }
            ShellTool::new().execute_with_policy(arguments, &policy).await
        } else {
            execute_fs_tool(&tool_name, arguments, &policy).await
        }
    } else if let Some(tool) = LOCAL_TOOL_REGISTRY.get(tool_name.as_str()) {
        tool.execute(arguments).await
    } else if tool_name.starts_with("mcp_") {
        // MCP tool -- manager finds the owning session by wrapped tool name
        let manager = discovery::get_mcp_manager().await;
        manager
            .call_tool(&tool_name, arguments)
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

/// Dispatch filesystem tool calls with the active FsPolicy.
async fn execute_fs_tool(
    tool_name: &str,
    arguments: Value,
    policy: &FsPolicy,
) -> Result<String, ToolError> {
    match tool_name {
        "read_file" => ReadFileTool::new().execute_with_policy(arguments, policy).await,
        "write_file" => WriteFileTool::new().execute_with_policy(arguments, policy).await,
        "list_dir" => ListDirTool::new().execute_with_policy(arguments, policy).await,
        "stat" => StatTool::new().execute_with_policy(arguments, policy).await,
        "edit_file" => EditFileTool::new().execute_with_policy(arguments, policy).await,
        _ => Err(ToolError::NotFound(format!("unknown fs tool: {}", tool_name))),
    }
}

