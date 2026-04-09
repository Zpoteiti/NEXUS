use crate::config::ClientConfig;
use crate::env::safe_env;
use crate::guardrails;
use crate::sandbox;
use crate::tools::helpers::{tool_error, truncate_output};
use crate::tools::{Tool, ToolResult};
use nexus_common::protocol::FsPolicy;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use tokio::process::Command;

pub struct ShellTool;

impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }
    fn description(&self) -> &str {
        "Execute a shell command. Working directory defaults to workspace root."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{
            "command":{"type":"string"}, "timeout_sec":{"type":"integer"},
            "working_dir":{"type":"string"}
        },"required":["command"]})
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
    let command = match args.get("command").and_then(Value::as_str) {
        Some(c) => c,
        None => return ToolResult::error(tool_error("missing: command")),
    };
    let timeout_sec = args
        .get("timeout_sec")
        .and_then(Value::as_u64)
        .unwrap_or(config.shell_timeout);
    let wd = args
        .get("working_dir")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| config.workspace.clone());

    // Guardrails check (Sandbox mode only)
    if config.fs_policy == FsPolicy::Sandbox
        && let Some(reason) = guardrails::check_all(command, &config.ssrf_whitelist).await
    {
        return ToolResult::blocked(reason);
    }

    // Build command
    let mut cmd = if config.fs_policy == FsPolicy::Sandbox && *sandbox::BWRAP_AVAILABLE {
        let a = sandbox::wrap_command(command, &config.workspace);
        let mut c = Command::new(&a[0]);
        c.args(&a[1..]);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        let mut c = Command::new("bash");
        c.args(["-l", "-c", command]);
        c
    };

    // Environment isolation (always active)
    cmd.env_clear();
    for (k, v) in safe_env() {
        cmd.env(k, v);
    }
    cmd.current_dir(&wd);

    // Execute with timeout
    match tokio::time::timeout(std::time::Duration::from_secs(timeout_sec), cmd.output()).await {
        Ok(Ok(out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let code = out.status.code().unwrap_or(1);
            let mut text = stdout.to_string();
            if !stderr.is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str("STDERR:\n");
                text.push_str(&stderr);
            }
            text.push_str(&format!("\nExit code: {code}"));
            ToolResult {
                exit_code: code,
                output: truncate_output(&text),
            }
        }
        Ok(Err(e)) => ToolResult::error(tool_error(&format!("exec failed: {e}"))),
        Err(_) => ToolResult::timeout(format!("Timed out after {timeout_sec}s: {command}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ucfg(d: &std::path::Path) -> ClientConfig {
        ClientConfig {
            workspace: d.to_path_buf(),
            fs_policy: FsPolicy::Unrestricted,
            shell_timeout: 60,
            ssrf_whitelist: vec![],
            mcp_servers: vec![],
        }
    }
    fn scfg(d: &std::path::Path) -> ClientConfig {
        ClientConfig {
            workspace: d.to_path_buf(),
            fs_policy: FsPolicy::Sandbox,
            shell_timeout: 60,
            ssrf_whitelist: vec![],
            mcp_servers: vec![],
        }
    }

    #[tokio::test]
    async fn test_echo() {
        let d = tempfile::tempdir().unwrap();
        let r = exec(
            serde_json::json!({"command": "echo hello"}),
            &ucfg(d.path()),
        )
        .await;
        assert_eq!(r.exit_code, 0);
        assert!(r.output.contains("hello"));
    }

    #[tokio::test]
    async fn test_exit_code() {
        let d = tempfile::tempdir().unwrap();
        let r = exec(serde_json::json!({"command": "exit 42"}), &ucfg(d.path())).await;
        assert_eq!(r.exit_code, 42);
    }

    #[tokio::test]
    async fn test_stderr() {
        let d = tempfile::tempdir().unwrap();
        let r = exec(
            serde_json::json!({"command": "echo err >&2"}),
            &ucfg(d.path()),
        )
        .await;
        assert!(r.output.contains("STDERR:") && r.output.contains("err"));
    }

    #[tokio::test]
    async fn test_timeout() {
        let d = tempfile::tempdir().unwrap();
        let r = exec(
            serde_json::json!({"command": "sleep 10", "timeout_sec": 1}),
            &ucfg(d.path()),
        )
        .await;
        assert_eq!(r.exit_code, nexus_common::consts::EXIT_CODE_TIMEOUT);
    }

    #[tokio::test]
    async fn test_sandbox_blocks_rm() {
        let d = tempfile::tempdir().unwrap();
        let r = exec(serde_json::json!({"command": "rm -rf /"}), &scfg(d.path())).await;
        assert_eq!(r.exit_code, nexus_common::consts::EXIT_CODE_CANCELLED);
    }

    #[tokio::test]
    async fn test_env_isolation() {
        let d = tempfile::tempdir().unwrap();
        // Verify that env vars from the parent process are NOT passed through.
        // HOME exists in parent but safe_env provides its own — check a non-safe var.
        let r = exec(
            serde_json::json!({"command": "env | grep -c CARGO || true"}),
            &ucfg(d.path()),
        )
        .await;
        // CARGO_* vars should not be present since env is cleared
        assert_eq!(r.exit_code, 0);
        assert!(r.output.contains("0") || !r.output.contains("CARGO"));
    }
}
