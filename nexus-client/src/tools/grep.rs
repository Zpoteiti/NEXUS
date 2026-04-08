use crate::config::ClientConfig;
use crate::tools::helpers::{sanitize_path, tool_error, IGNORED_DIRS};
use crate::tools::{Tool, ToolResult};
use regex::Regex;
use serde_json::Value;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

pub struct GrepTool;

impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Search file contents with regex."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{
            "pattern":{"type":"string"}, "path":{"type":"string"},
            "include":{"type":"string","description":"Glob filter"},
            "context":{"type":"integer","default":0}
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
    let ctx = args
        .get("context")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let include = args.get("include").and_then(Value::as_str);
    let re = match Regex::new(pat) {
        Ok(r) => r,
        Err(e) => return ToolResult::error(tool_error(&format!("bad regex: {e}"))),
    };
    let base = if let Some(p) = args.get("path").and_then(Value::as_str) {
        match sanitize_path(p, config, false) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(e),
        }
    } else {
        config.workspace.clone()
    };
    let incl = include.and_then(|p| glob::Pattern::new(p).ok());
    let mut results = Vec::new();
    search_dir(&base, &base, &re, &incl, ctx, &mut results);
    if results.is_empty() {
        ToolResult::success("No matches found.")
    } else {
        ToolResult::success(results.join("\n"))
    }
}

fn search_dir(
    base: &Path,
    dir: &Path,
    re: &Regex,
    incl: &Option<glob::Pattern>,
    ctx: usize,
    res: &mut Vec<String>,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if IGNORED_DIRS.contains(&n.as_str()) {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            search_dir(base, &p, re, incl, ctx, res);
        } else if p.is_file() {
            if let Some(g) = incl
                && !g.matches(&n)
            {
                continue;
            }
            search_file(base, &p, re, ctx, res);
        }
    }
}

fn search_file(
    base: &Path,
    path: &Path,
    re: &Regex,
    ctx: usize,
    res: &mut Vec<String>,
) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let lines: Vec<&str> = content.lines().collect();
    let rel = path
        .strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy();
    for (i, line) in lines.iter().enumerate() {
        if re.is_match(line) {
            let s = i.saturating_sub(ctx);
            let e = (i + ctx + 1).min(lines.len());
            for (j, line) in lines[s..e].iter().enumerate() {
                let line_num = s + j;
                let pfx = if line_num == i { ">" } else { " " };
                res.push(format!("{rel}:{}{pfx} {line}", line_num + 1));
            }
            if ctx > 0 {
                res.push("--".into());
            }
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
    async fn test_basic() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("h.txt"),
            "hello\nbye\nhello again\n",
        )
        .unwrap();
        let r = exec(serde_json::json!({"pattern": "hello"}), &cfg(d.path())).await;
        assert!(r.output.contains("h.txt:1") && r.output.contains("h.txt:3"));
    }

    #[tokio::test]
    async fn test_include_filter() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.rs"), "fn main").unwrap();
        std::fs::write(d.path().join("b.txt"), "fn main").unwrap();
        let r = exec(
            serde_json::json!({"pattern": "fn", "include": "*.rs"}),
            &cfg(d.path()),
        )
        .await;
        assert!(r.output.contains("a.rs") && !r.output.contains("b.txt"));
    }

    #[tokio::test]
    async fn test_bad_regex() {
        let d = tempfile::tempdir().unwrap();
        let r = exec(
            serde_json::json!({"pattern": "[invalid"}),
            &cfg(d.path()),
        )
        .await;
        assert_eq!(r.exit_code, 1);
    }
}
