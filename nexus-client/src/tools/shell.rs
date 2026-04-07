/// Responsibility boundary:
/// 1. Performs the actual OS shell invocation (cmd.exe on Windows, sh/bash on Linux).
/// 2. Implements tokio::time::timeout control (default 60s), killing the child process on timeout.
/// 3. Implements dual-end output truncation: when exceeding MAX_TOOL_OUTPUT_CHARS,
///    keeps only the first TOOL_OUTPUT_HEAD_CHARS and last TOOL_OUTPUT_TAIL_CHARS.

use async_trait::async_trait;
use nexus_common::consts::{
    HEARTBEAT_INTERVAL_SEC, MAX_TOOL_OUTPUT_CHARS, TOOL_OUTPUT_HEAD_CHARS,
    TOOL_OUTPUT_TAIL_CHARS,
};
use nexus_common::protocol::FsPolicy;
use serde_json::Value;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::{LocalTool, ToolError};
use crate::guardrails;
use crate::env;

/// Default timeout = HEARTBEAT_INTERVAL_SEC * 4 = 60s
const DEFAULT_TIMEOUT_SEC: u64 = HEARTBEAT_INTERVAL_SEC * 4;
/// Maximum timeout 600s
const MAX_TIMEOUT_SEC: u64 = 600;

pub struct ShellTool;

impl ShellTool {
    pub fn new() -> Self {
        ShellTool
    }

    /// Execute a shell command (after guardrails checks).
    async fn run(&self, cmd: &str, timeout_sec: Option<u64>, working_dir: Option<&std::path::Path>) -> Result<String, ToolError> {
        // 1. Security validation (guardrails) - async SSRF DNS resolution
        guardrails::check_shell_command(cmd).await?;

        // 2. Timeout control
        let timeout_sec = timeout_sec.unwrap_or(DEFAULT_TIMEOUT_SEC).min(MAX_TIMEOUT_SEC);

        // 3. Execute command
        let output = run_shell_command(cmd, timeout_sec, working_dir).await?;

        // 4. Truncate output
        let truncated = truncate_output(&output);
        Ok(truncated)
    }
}

impl Default for ShellTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LocalTool for ShellTool {
    fn name(&self) -> &'static str {
        "shell"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "shell",
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
                            "description": "Optional execution timeout in seconds. Defaults to 60, max 600.",
                            "minimum": 1,
                            "maximum": 600
                        },
                        "working_dir": {
                            "type": "string",
                            "description": "Optional working directory for the command. Must be within workspace."
                        }
                    },
                    "required": ["command"]
                }
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String, ToolError> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("missing required field: command".to_string()))?
            .to_string();

        let timeout_sec = args
            .get("timeout_sec")
            .and_then(|v| v.as_u64());

        let working_dir = args
            .get("working_dir")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // If working_dir is specified, validate and use it
        let resolved_dir = if let Some(ref dir) = working_dir {
            Some(env::sanitize_path(dir, true)?)
        } else {
            None
        };

        self.run(&command, timeout_sec, resolved_dir.as_deref()).await
    }
}

/// Actually execute a shell command, with timeout control.
async fn run_shell_command(cmd: &str, timeout_sec: u64, working_dir: Option<&std::path::Path>) -> Result<String, ToolError> {
    let workspace = working_dir.map(|p| p.to_path_buf()).unwrap_or_else(env::get_workspace_root);
    let mut child = if cfg!(windows) {
        Command::new("cmd")
            .args(["/C", cmd])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(&workspace)
            .envs(env::min_env())
            .spawn()
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to spawn cmd: {}", e)))?
    } else {
        Command::new("sh")
            .args(["-c", cmd])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(&workspace)
            .envs(env::min_env())
            .spawn()
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to spawn sh: {}", e)))?
    };

    let result = timeout(Duration::from_secs(timeout_sec), async {
        let (mut stdout, mut stderr) = (
            String::new(),
            String::new(),
        );

        if let Some(mut out) = child.stdout.take() {
            out.read_to_string(&mut stdout)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("failed to read stdout: {}", e)))?;
        }

        if let Some(mut err) = child.stderr.take() {
            err.read_to_string(&mut stderr)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("failed to read stderr: {}", e)))?;
        }

        let status = child
            .wait()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to wait child: {}", e)))?;

        let output = if !stderr.is_empty() {
            format!("{}\nSTDERR:\n{}", stdout, stderr)
        } else {
            stdout
        };

        if !status.success() {
            return Err(ToolError::ExecutionFailed(format!(
                "command exited with {}: {}",
                status,
                output
            )));
        }

        Ok(output)
    })
    .await;

    match result {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) => Err(e),
        Err(_) => {
            // Timeout -- kill the child process
            let _ = child.kill().await;
            Err(ToolError::Timeout(timeout_sec))
        }
    }
}

/// Dual-end truncation: when exceeding MAX_TOOL_OUTPUT_CHARS, keep HEAD + TAIL.
fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_TOOL_OUTPUT_CHARS {
        return output.to_string();
    }

    let head: String = output.chars().take(TOOL_OUTPUT_HEAD_CHARS).collect();
    let tail: String = output
        .chars()
        .rev()
        .take(TOOL_OUTPUT_TAIL_CHARS)
        .collect::<String>()
        .chars()
        .rev()
        .collect();

    let middle_len = output.len() - TOOL_OUTPUT_HEAD_CHARS - TOOL_OUTPUT_TAIL_CHARS;
    format!(
        "{}\n... ({} chars truncated) ...\n{}",
        head, middle_len, tail
    )
}

/// Validate a shell command against the filesystem policy.
/// Returns Err with a reason if the command violates the policy.
pub fn guard_command_policy(cmd: &str, policy: &FsPolicy) -> Result<(), ToolError> {
    let allowed_paths: &[String] = match policy {
        FsPolicy::Unrestricted => return Ok(()),
        FsPolicy::Sandbox => &[],
        FsPolicy::Whitelist { allowed_paths } => allowed_paths,
    };

    if cmd.contains("../") || cmd.contains("..\\") {
        return Err(ToolError::Blocked(
            "command blocked: path traversal '../' not allowed by device policy".to_string(),
        ));
    }

    let workspace = env::get_workspace_root();
    let workspace_str = workspace.to_string_lossy();

    for path in extract_absolute_paths(cmd) {
        if path.starts_with("/dev/null") || path.starts_with("/tmp/nexus") {
            continue;
        }
        if path.starts_with(workspace_str.as_ref()) {
            continue;
        }
        if allowed_paths.iter().any(|ap| path.starts_with(ap.as_str())) {
            continue;
        }
        return Err(ToolError::Blocked(format!(
            "command blocked: absolute path '{}' is outside allowed filesystem scope",
            path
        )));
    }
    Ok(())
}

/// Extract absolute paths from a shell command string (single pass).
fn extract_absolute_paths(cmd: &str) -> Vec<&str> {
    let mut paths = Vec::new();
    for token in cmd.split_whitespace() {
        let clean = token
            .trim_start_matches('>')
            .trim_start_matches('<')
            .trim_start_matches('|');
        if clean.starts_with('/') && clean.len() > 1 {
            paths.push(clean);
        }
        if let Some(pos) = token.find('=') {
            let after = &token[pos + 1..];
            if after.starts_with('/') && after.len() > 1 {
                paths.push(after);
            }
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_short_output() {
        let short = "hello world";
        assert_eq!(truncate_output(short), short);
    }

    #[test]
    fn test_truncate_long_output() {
        let long = "a".repeat(15000);
        let result = truncate_output(&long);
        assert!(result.contains("... ("));
        assert!(result.contains(" chars truncated) ..."));
    }

    #[test]
    fn test_truncate_exact_boundary() {
        // Exactly MAX_TOOL_OUTPUT_CHARS, no truncation
        let exact = "x".repeat(MAX_TOOL_OUTPUT_CHARS);
        assert_eq!(truncate_output(&exact), exact);
    }
}
