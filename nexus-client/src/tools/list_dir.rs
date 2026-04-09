use crate::config::ClientConfig;
use crate::tools::helpers::{IGNORED_DIRS, sanitize_path, tool_error};
use crate::tools::{Tool, ToolResult};
use nexus_common::consts::DEFAULT_LIST_DIR_MAX;
use serde_json::Value;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

pub struct ListDirTool;

impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }
    fn description(&self) -> &str {
        "List directory contents. Use '.' for workspace root. Relative paths resolve from workspace."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{
            "path":{"type":"string"}, "recursive":{"type":"boolean","default":false},
            "max_entries":{"type":"integer","default":200}
        },"required":["path"]})
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
    let recursive = args
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let max = args
        .get("max_entries")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_LIST_DIR_MAX as u64) as usize;

    let path = match sanitize_path(p, config, false) {
        Ok(p) => p,
        Err(e) => return ToolResult::error(e),
    };
    if !path.is_dir() {
        return ToolResult::error(tool_error(&format!(
            "not a directory: {p}. Your workspace is: {}",
            config.workspace.display()
        )));
    }

    let entries = match tokio::task::spawn_blocking(move || {
        let mut entries = Vec::new();
        if recursive {
            collect_rec(&path, &path, &mut entries, max * 2);
        } else {
            collect_flat(&path, &mut entries, max * 2);
        }
        entries.sort();
        entries
    })
    .await
    {
        Ok(e) => e,
        Err(e) => return ToolResult::error(tool_error(&format!("list_dir task failed: {e}"))),
    };

    let mut out: String = entries
        .iter()
        .take(max)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    if entries.len() > max {
        out.push_str(&format!("\n... ({} total, showing {max})", entries.len()));
    }
    ToolResult::success(out)
}

fn collect_flat(dir: &Path, entries: &mut Vec<String>, max: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        if entries.len() >= max {
            break;
        }
        let n = e.file_name().to_string_lossy().to_string();
        if IGNORED_DIRS.contains(&n.as_str()) {
            continue;
        }
        entries.push(if e.file_type().map(|f| f.is_dir()).unwrap_or(false) {
            format!("[DIR]  {n}")
        } else {
            format!("[FILE] {n}")
        });
    }
}

fn collect_rec(base: &Path, dir: &Path, entries: &mut Vec<String>, max: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        if entries.len() >= max {
            break;
        }
        let n = e.file_name().to_string_lossy().to_string();
        if IGNORED_DIRS.contains(&n.as_str()) {
            continue;
        }
        let rel = e
            .path()
            .strip_prefix(base)
            .unwrap_or(&e.path())
            .to_string_lossy()
            .to_string();
        if e.file_type().map(|f| f.is_dir()).unwrap_or(false) {
            entries.push(format!("{rel}/"));
            collect_rec(base, &e.path(), entries, max);
        } else {
            entries.push(rel);
        }
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
    async fn test_flat() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.txt"), "").unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        let r = exec(
            serde_json::json!({"path": d.path().to_str().unwrap()}),
            &cfg(d.path()),
        )
        .await;
        assert!(r.output.contains("[FILE] a.txt") && r.output.contains("[DIR]  sub"));
    }

    #[tokio::test]
    async fn test_recursive() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("s")).unwrap();
        std::fs::write(d.path().join("s/f.txt"), "").unwrap();
        let r = exec(
            serde_json::json!({"path": d.path().to_str().unwrap(), "recursive": true}),
            &cfg(d.path()),
        )
        .await;
        assert!(r.output.contains("s/") && r.output.contains("s/f.txt"));
    }

    #[tokio::test]
    async fn test_ignores_git() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join(".git")).unwrap();
        std::fs::create_dir(d.path().join("src")).unwrap();
        let r = exec(
            serde_json::json!({"path": d.path().to_str().unwrap()}),
            &cfg(d.path()),
        )
        .await;
        assert!(!r.output.contains(".git") && r.output.contains("src"));
    }
}
