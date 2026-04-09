//! Shared tool helpers: path sanitization, output truncation, ignored dirs.

use crate::config::ClientConfig;
use nexus_common::consts::{MAX_TOOL_OUTPUT_CHARS, TOOL_OUTPUT_HEAD_CHARS, TOOL_OUTPUT_TAIL_CHARS};
use nexus_common::protocol::FsPolicy;
use std::path::{Path, PathBuf};

/// Directories auto-ignored by list_dir, glob, grep.
pub const IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "__pycache__",
    ".venv",
    "venv",
    "dist",
    "build",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".coverage",
    "htmlcov",
];

/// Format a tool error with hint suffix.
pub fn tool_error(msg: &str) -> String {
    format!("Error: {msg}\n\n[Analyze the error and try a different approach.]")
}

/// Truncate output to MAX_TOOL_OUTPUT_CHARS using head/tail strategy.
pub fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_TOOL_OUTPUT_CHARS {
        return output.to_string();
    }
    let head = &output[..TOOL_OUTPUT_HEAD_CHARS];
    let tail = &output[output.len() - TOOL_OUTPUT_TAIL_CHARS..];
    format!("{head}\n... ({} chars truncated) ...\n{tail}", output.len())
}

/// Validate and resolve a path against the client's FsPolicy.
///
/// 1. Expand `~` to home dir
/// 2. Relative paths joined to workspace
/// 3. Canonicalize (resolve symlinks) — for new files, canonicalize parent
/// 4. Sandbox mode: path must start with workspace. Exceptions: /dev/null, /tmp/nexus*
/// 5. Unrestricted: all paths allowed
pub fn sanitize_path(path: &str, config: &ClientConfig, write: bool) -> Result<PathBuf, String> {
    let expanded = if path.starts_with("~/") || path == "~" {
        let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
        path.replacen('~', &home, 1)
    } else {
        path.to_string()
    };

    let abs = if Path::new(&expanded).is_absolute() {
        PathBuf::from(&expanded)
    } else {
        config.workspace.join(&expanded)
    };

    let canonical = if abs.exists() {
        abs.canonicalize()
            .map_err(|e| format!("canonicalize: {e}"))?
    } else {
        let parent = abs.parent().ok_or("no parent")?;
        let name = abs.file_name().ok_or("no filename")?;
        if parent.exists() {
            parent
                .canonicalize()
                .map_err(|e| format!("canonicalize parent: {e}"))?
                .join(name)
        } else if write {
            abs
        } else {
            return Err(tool_error(&format!("path not found: {path}")));
        }
    };

    match config.fs_policy {
        FsPolicy::Sandbox => {
            let s = canonical.to_string_lossy();
            if s == "/dev/null" || s.starts_with("/tmp/nexus") {
                return Ok(canonical);
            }
            let ws = config
                .workspace
                .canonicalize()
                .unwrap_or_else(|_| config.workspace.clone());
            if !canonical.starts_with(&ws) {
                return Err(tool_error(&format!(
                    "path outside workspace in Sandbox mode: {path}. Your workspace is: {}",
                    config.workspace.display()
                )));
            }
        }
        FsPolicy::Unrestricted => {}
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(ws: &str) -> ClientConfig {
        ClientConfig {
            workspace: PathBuf::from(ws),
            fs_policy: FsPolicy::Sandbox,
            shell_timeout: 60,
            ssrf_whitelist: vec![],
            mcp_servers: vec![],
        }
    }

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate_output("hi"), "hi");
    }

    #[test]
    fn test_truncate_long() {
        let long = "x".repeat(20_000);
        let r = truncate_output(&long);
        assert!(r.len() < long.len());
        assert!(r.contains("truncated"));
    }

    #[test]
    fn test_tool_error_format() {
        assert!(tool_error("oops").starts_with("Error: "));
    }

    #[test]
    fn test_sanitize_relative() {
        let c = sandbox("/tmp");
        assert!(
            sanitize_path("test.txt", &c, true)
                .unwrap()
                .starts_with("/tmp")
        );
    }

    #[test]
    fn test_sandbox_blocks_outside() {
        let c = sandbox("/tmp/workspace");
        assert!(sanitize_path("/etc/passwd", &c, false).is_err());
    }

    #[test]
    fn test_sandbox_allows_dev_null() {
        let c = sandbox("/tmp/workspace");
        assert!(sanitize_path("/dev/null", &c, false).is_ok());
    }

    #[test]
    fn test_sandbox_allows_tmp_nexus() {
        let c = sandbox("/tmp/workspace");
        assert!(sanitize_path("/tmp/nexus_cache", &c, true).is_ok());
    }

    #[test]
    fn test_unrestricted_allows_all() {
        let c = ClientConfig {
            workspace: PathBuf::from("/tmp"),
            fs_policy: FsPolicy::Unrestricted,
            shell_timeout: 60,
            ssrf_whitelist: vec![],
            mcp_servers: vec![],
        };
        assert!(sanitize_path("/etc/passwd", &c, false).is_ok());
    }
}
