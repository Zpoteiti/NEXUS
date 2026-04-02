/// 职责边界：
/// 1. 负责真正调用操作系统的 Shell (Windows 调 cmd.exe，Linux 调 sh/bash)。
/// 2. 实现 tokio::time::timeout 控制（默认 60s），超时则 Kill 子进程。
/// 3. 实现输出的双端截断策略：超过 MAX_TOOL_OUTPUT_CHARS 时，
///    只保留前 TOOL_OUTPUT_HEAD_CHARS 和后 TOOL_OUTPUT_TAIL_CHARS。
///
/// 参考 nanobot：
/// - 对应 `nanobot/agent/tools/shell.py` 的底层执行与截断逻辑。

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

/// 默认超时 = HEARTBEAT_INTERVAL_SEC * 4 = 60s
const DEFAULT_TIMEOUT_SEC: u64 = HEARTBEAT_INTERVAL_SEC * 4;
/// 最大超时 600s
const MAX_TIMEOUT_SEC: u64 = 600;

pub struct ShellTool;

impl ShellTool {
    pub fn new() -> Self {
        ShellTool
    }

    /// 执行 shell 命令（经过 guardrails 检查）。
    async fn run(&self, cmd: &str, timeout_sec: Option<u64>) -> Result<String, ToolError> {
        // 1. 安全校验（guardrails）- 异步 SSRF DNS 解析
        guardrails::check_shell_command(cmd).await.map_err(ToolError::Blocked)?;

        // 2. 超时控制
        let timeout_sec = timeout_sec.unwrap_or(DEFAULT_TIMEOUT_SEC).min(MAX_TIMEOUT_SEC);

        // 3. 执行命令
        let output = run_shell_command(cmd, timeout_sec).await?;

        // 4. 截断输出
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

        // 如果指定了 working_dir，校验并使用它
        if let Some(ref dir) = working_dir {
            env::sanitize_path(dir, true)
                .map_err(|e| ToolError::InvalidParams(format!("invalid working_dir: {}", e)))?;
        }

        self.run(&command, timeout_sec).await
    }
}

/// 实际执行 shell 命令，带超时控制。
async fn run_shell_command(cmd: &str, timeout_sec: u64) -> Result<String, ToolError> {
    let mut child = if cfg!(windows) {
        Command::new("cmd")
            .args(["/C", cmd])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .envs(env::min_env())
            .spawn()
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to spawn cmd: {}", e)))?
    } else {
        Command::new("sh")
            .args(["-c", cmd])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
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
            // 超时，Kill 子进程
            let _ = child.kill().await;
            Err(ToolError::Timeout(timeout_sec))
        }
    }
}

/// 双端截断：超过 MAX_TOOL_OUTPUT_CHARS 时，保留前 HEAD + 后 TAIL。
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
pub fn guard_command_policy(cmd: &str, policy: &FsPolicy) -> Result<(), String> {
    let allowed_paths: &[String] = match policy {
        FsPolicy::Unrestricted => return Ok(()),
        FsPolicy::Sandbox => &[],
        FsPolicy::Whitelist { allowed_paths } => allowed_paths,
    };

    if cmd.contains("../") || cmd.contains("..\\") {
        return Err("command blocked: path traversal '../' not allowed by device policy".to_string());
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
        return Err(format!(
            "command blocked: absolute path '{}' is outside allowed filesystem scope",
            path
        ));
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
        // 正好等于 MAX_TOOL_OUTPUT_CHARS，不截断
        let exact = "x".repeat(MAX_TOOL_OUTPUT_CHARS);
        assert_eq!(truncate_output(&exact), exact);
    }
}
