use crate::config::ClientConfig;
use crate::tools::helpers::{sanitize_path, tool_error};
use crate::tools::{Tool, ToolResult};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

pub struct EditFileTool;

impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }
    fn description(&self) -> &str {
        "Replace exact text in a file. On 0 matches, tries fuzzy whitespace-tolerant match."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{
            "file_path":{"type":"string"}, "old_string":{"type":"string"}, "new_string":{"type":"string"}
        },"required":["file_path","old_string","new_string"]})
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

/// Fuzzy matching: normalize both sides to LF, strip whitespace per line,
/// find sliding window of same line count with matching stripped content.
/// Returns (matched_fragment_with_original_whitespace, match_count).
fn find_match(content: &str, old_text: &str) -> (Option<String>, usize) {
    let content = content.replace("\r\n", "\n");
    let old_text = old_text.replace("\r\n", "\n");

    // Exact match first
    if content.contains(&old_text) {
        return (Some(old_text.clone()), content.matches(&old_text).count());
    }

    // Fuzzy: line-stripped sliding window
    let old_lines: Vec<&str> = old_text.lines().collect();
    if old_lines.is_empty() {
        return (None, 0);
    }
    let stripped_old: Vec<String> = old_lines.iter().map(|l| l.trim().to_string()).collect();
    let content_lines: Vec<&str> = content.lines().collect();

    let mut candidates = Vec::new();
    for i in 0..=content_lines.len().saturating_sub(stripped_old.len()) {
        let window = &content_lines[i..i + stripped_old.len()];
        let stripped_win: Vec<String> = window.iter().map(|l| l.trim().to_string()).collect();
        if stripped_win == stripped_old {
            candidates.push(window.join("\n"));
        }
    }

    if candidates.is_empty() {
        (None, 0)
    } else {
        let c = candidates.len();
        (Some(candidates.into_iter().next().unwrap()), c)
    }
}

/// Find the closest matching chunk in content to show as a diff hint.
/// Uses a simple line-by-line similarity score (matching lines / total lines).
fn find_closest_match(content: &str, old_text: &str) -> String {
    let old_lines: Vec<&str> = old_text.lines().collect();
    if old_lines.is_empty() {
        return String::new();
    }
    let content_lines: Vec<&str> = content.lines().collect();
    if content_lines.is_empty() {
        return String::new();
    }

    let window = old_lines.len();
    let mut best_score = 0usize;
    let mut best_window: Option<Vec<&str>> = None;

    for i in 0..=content_lines.len().saturating_sub(window) {
        let candidate = &content_lines[i..i + window];
        let score: usize = candidate
            .iter()
            .zip(old_lines.iter())
            .filter(|(a, b)| a.trim() == b.trim())
            .count();
        if score > best_score {
            best_score = score;
            best_window = Some(candidate.to_vec());
        }
    }

    // Only show if at least 30% of lines match
    if best_score * 10 < window * 3 {
        return String::new();
    }

    if let Some(win) = best_window {
        let mut diff = String::from("\n\nClosest match found:\n");
        for (actual, expected) in win.iter().zip(old_lines.iter()) {
            if actual.trim() == expected.trim() {
                diff.push_str(&format!("  {actual}\n"));
            } else {
                diff.push_str(&format!("- {expected}\n"));
                diff.push_str(&format!("+ {actual}\n"));
            }
        }
        diff
    } else {
        String::new()
    }
}

async fn exec(args: Value, config: &ClientConfig) -> ToolResult {
    let p = match args.get("file_path").and_then(Value::as_str) {
        Some(p) => p,
        None => return ToolResult::error(tool_error("missing: file_path")),
    };
    let old = match args.get("old_string").and_then(Value::as_str) {
        Some(s) if !s.is_empty() => s,
        _ => return ToolResult::error(tool_error("old_string must be non-empty")),
    };
    let new = match args.get("new_string").and_then(Value::as_str) {
        Some(s) => s,
        None => return ToolResult::error(tool_error("missing: new_string")),
    };

    let path = match sanitize_path(p, config, true) {
        Ok(p) => p,
        Err(e) => return ToolResult::error(e),
    };
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(e) => return ToolResult::error(tool_error(&format!("read: {e}"))),
    };

    let (matched, count) = find_match(&content, old);

    match count {
        0 => {
            let hint = find_closest_match(&content, old);
            ToolResult::error(tool_error(&format!("old_string not found in {p}{hint}")))
        }
        1 => {
            let new_content = content.replacen(&matched.unwrap(), new, 1);
            match tokio::fs::write(&path, &new_content).await {
                Ok(()) => ToolResult::success(format!("Edited {p}")),
                Err(e) => ToolResult::error(tool_error(&format!("write: {e}"))),
            }
        }
        n => ToolResult::error(tool_error(&format!(
            "{n} matches in {p} — must be exactly 1"
        ))),
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

    #[test]
    fn test_find_exact() {
        let (m, c) = find_match("hello world", "world");
        assert_eq!(c, 1);
        assert_eq!(m.unwrap(), "world");
    }

    #[test]
    fn test_find_multiple() {
        assert_eq!(find_match("aa bb aa", "aa").1, 2);
    }

    #[test]
    fn test_find_fuzzy() {
        let (m, c) = find_match("    fn f() {\n        x();\n    }", "fn f() {\n    x();\n}");
        assert_eq!(c, 1);
        assert!(m.unwrap().contains("        x();"));
    }

    #[test]
    fn test_find_none() {
        assert_eq!(find_match("hello", "bye").1, 0);
    }

    #[tokio::test]
    async fn test_edit_exact() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("t.rs");
        std::fs::write(&f, "fn main() { old() }").unwrap();
        let r = exec(
            serde_json::json!({"file_path": f.to_str().unwrap(), "old_string": "old()", "new_string": "new()"}),
            &cfg(d.path()),
        )
        .await;
        assert_eq!(r.exit_code, 0);
        assert!(std::fs::read_to_string(&f).unwrap().contains("new()"));
    }

    #[tokio::test]
    async fn test_edit_no_match() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("t.txt");
        std::fs::write(&f, "hello").unwrap();
        let r = exec(
            serde_json::json!({"file_path": f.to_str().unwrap(), "old_string": "bye", "new_string": "x"}),
            &cfg(d.path()),
        )
        .await;
        assert_eq!(r.exit_code, 1);
    }

    #[tokio::test]
    async fn test_edit_multi_match() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("t.txt");
        std::fs::write(&f, "aa bb aa").unwrap();
        let r = exec(
            serde_json::json!({"file_path": f.to_str().unwrap(), "old_string": "aa", "new_string": "cc"}),
            &cfg(d.path()),
        )
        .await;
        assert_eq!(r.exit_code, 1);
    }
}
