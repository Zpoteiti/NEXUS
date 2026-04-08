pub mod edit_file;
pub mod glob;
pub mod grep;
pub mod helpers;
pub mod list_dir;
pub mod read_file;
pub mod shell;
pub mod write_file;

use crate::config::ClientConfig;
use nexus_common::consts::{EXIT_CODE_ERROR, EXIT_CODE_SUCCESS};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

/// Result returned by every tool execution.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub exit_code: i32,
    pub output: String,
}

impl ToolResult {
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            exit_code: EXIT_CODE_SUCCESS,
            output: output.into(),
        }
    }

    pub fn error(output: impl Into<String>) -> Self {
        Self {
            exit_code: EXIT_CODE_ERROR,
            output: output.into(),
        }
    }

    pub fn blocked(output: impl Into<String>) -> Self {
        Self {
            exit_code: nexus_common::consts::EXIT_CODE_CANCELLED,
            output: output.into(),
        }
    }

    pub fn timeout(output: impl Into<String>) -> Self {
        Self {
            exit_code: nexus_common::consts::EXIT_CODE_TIMEOUT,
            output: output.into(),
        }
    }
}

/// Trait for all tools (built-in and MCP wrappers).
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;
    fn execute(
        &self,
        args: Value,
        config: &ClientConfig,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>>;
}

/// Tool registry: maps tool names to implementations.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Dispatch a tool call by name.
    pub async fn dispatch(
        &self,
        name: &str,
        args: Value,
        config: &ClientConfig,
    ) -> ToolResult {
        match self.tools.get(name) {
            Some(tool) => tool.execute(args, config).await,
            None => ToolResult::error(helpers::tool_error(&format!("tool not found: {name}"))),
        }
    }

    /// Build OpenAI function-calling schemas for all registered tools.
    pub fn schemas(&self) -> Vec<Value> {
        self.tools
            .values()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name(),
                        "description": t.description(),
                        "parameters": t.parameters(),
                    }
                })
            })
            .collect()
    }

    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }
}

/// Register all 7 built-in tools.
pub fn register_builtin_tools(_registry: &mut ToolRegistry) {
    // TODO (Section 4-5): register each tool after creation
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct DummyTool;
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            "dummy"
        }
        fn description(&self) -> &str {
            "test"
        }
        fn parameters(&self) -> Value {
            serde_json::json!({"type":"object","properties":{},"required":[]})
        }
        fn execute(
            &self,
            _: Value,
            _: &ClientConfig,
        ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
            Box::pin(async { ToolResult::success("ok") })
        }
    }

    fn cfg() -> ClientConfig {
        ClientConfig {
            workspace: PathBuf::from("/tmp"),
            fs_policy: nexus_common::protocol::FsPolicy::Sandbox,
            shell_timeout: 60,
            ssrf_whitelist: vec![],
            mcp_servers: vec![],
        }
    }

    #[test]
    fn test_register_count() {
        let mut r = ToolRegistry::new();
        r.register(Box::new(DummyTool));
        assert_eq!(r.tool_count(), 1);
    }

    #[tokio::test]
    async fn test_dispatch_found() {
        let mut r = ToolRegistry::new();
        r.register(Box::new(DummyTool));
        let res = r.dispatch("dummy", Value::Null, &cfg()).await;
        assert_eq!(res.exit_code, 0);
    }

    #[tokio::test]
    async fn test_dispatch_not_found() {
        let r = ToolRegistry::new();
        let res = r.dispatch("missing", Value::Null, &cfg()).await;
        assert_eq!(res.exit_code, 1);
    }

    #[test]
    fn test_schemas_format() {
        let mut r = ToolRegistry::new();
        r.register(Box::new(DummyTool));
        let s = r.schemas();
        assert_eq!(s[0]["function"]["name"], "dummy");
    }
}
