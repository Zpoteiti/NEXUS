use crate::config::ClientConfig;
use crate::tools::helpers::{sanitize_path, tool_error};
use crate::tools::{Tool, ToolResult};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

pub struct WriteFileTool;

impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        "Write content to a file. Creates parent directories."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{
            "path":{"type":"string"}, "content":{"type":"string"}
        },"required":["path","content"]})
    }
    fn execute(
        &self,
        args: Value,
        config: &ClientConfig,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let config = config.clone();
        Box::pin(async move { exec(args, &config).await })
    }
}

async fn exec(args: Value, config: &ClientConfig) -> ToolResult {
    let p = match args.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => return ToolResult::error(tool_error("missing: path")),
    };
    let content = match args.get("content").and_then(Value::as_str) {
        Some(c) => c,
        None => return ToolResult::error(tool_error("missing: content")),
    };

    let path = match sanitize_path(p, config, true) {
        Ok(p) => p,
        Err(e) => return ToolResult::error(e),
    };

    if let Some(parent) = path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            return ToolResult::error(tool_error(&format!("mkdir: {e}")));
        }
    }

    match tokio::fs::write(&path, content).await {
        Ok(()) => ToolResult::success(format!("Wrote {} chars to {p}", content.len())),
        Err(e) => ToolResult::error(tool_error(&format!("write: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_common::protocol::FsPolicy;

    fn cfg(d: &std::path::Path) -> ClientConfig {
        ClientConfig {
            workspace: d.to_path_buf(),
            fs_policy: FsPolicy::Unrestricted,
            shell_timeout: 60,
            ssrf_whitelist: vec![],
            mcp_servers: vec![],
        }
    }

    #[tokio::test]
    async fn test_write() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("o.txt");
        let r = exec(
            serde_json::json!({"path": f.to_str().unwrap(), "content": "hi"}),
            &cfg(d.path()),
        )
        .await;
        assert_eq!(r.exit_code, 0);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "hi");
    }

    #[tokio::test]
    async fn test_creates_dirs() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a/b/c.txt");
        exec(
            serde_json::json!({"path": f.to_str().unwrap(), "content": "deep"}),
            &cfg(d.path()),
        )
        .await;
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "deep");
    }
}
