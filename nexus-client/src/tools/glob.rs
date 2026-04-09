use crate::config::ClientConfig;
use crate::tools::helpers::{IGNORED_DIRS, sanitize_path, tool_error};
use crate::tools::{Tool, ToolResult};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

pub struct GlobTool;

impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }
    fn description(&self) -> &str {
        "Find files matching a glob pattern, sorted by mtime (newest first)."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{
            "pattern":{"type":"string"},
            "path":{"type":"string","description":"Base dir (default: workspace)"}
        },"required":["pattern"]})
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
    let pat = match args.get("pattern").and_then(Value::as_str) {
        Some(p) => p,
        None => return ToolResult::error(tool_error("missing: pattern")),
    };
    let base = if let Some(p) = args.get("path").and_then(Value::as_str) {
        match sanitize_path(p, config, false) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(e),
        }
    } else {
        config.workspace.clone()
    };

    let full = base.join(pat).to_string_lossy().to_string();
    let entries = match tokio::task::spawn_blocking(move || {
        let paths = glob::glob(&full).map_err(|e| format!("bad pattern: {e}"))?;
        let mut entries: Vec<(std::time::SystemTime, String)> = Vec::new();
        for entry in paths.flatten() {
            if entry
                .components()
                .any(|c| IGNORED_DIRS.contains(&c.as_os_str().to_string_lossy().as_ref()))
            {
                continue;
            }
            let mt = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let rel = entry
                .strip_prefix(&base)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| entry.to_string_lossy().to_string());
            entries.push((mt, rel));
        }
        entries.sort_by(|a, b| b.0.cmp(&a.0));
        Ok::<_, String>(entries)
    })
    .await
    {
        Ok(Ok(e)) => e,
        Ok(Err(e)) => return ToolResult::error(tool_error(&e)),
        Err(e) => return ToolResult::error(tool_error(&format!("glob task failed: {e}"))),
    };

    if entries.is_empty() {
        return ToolResult::success("No files matched.");
    }
    ToolResult::success(
        entries
            .iter()
            .map(|(_, p)| p.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    )
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
    async fn test_glob_rs() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.rs"), "").unwrap();
        std::fs::write(d.path().join("b.txt"), "").unwrap();
        let r = exec(serde_json::json!({"pattern": "*.rs"}), &cfg(d.path())).await;
        assert!(r.output.contains("a.rs") && !r.output.contains("b.txt"));
    }

    #[tokio::test]
    async fn test_no_matches() {
        let d = tempfile::tempdir().unwrap();
        let r = exec(serde_json::json!({"pattern": "*.xyz"}), &cfg(d.path())).await;
        assert!(r.output.contains("No files"));
    }
}
