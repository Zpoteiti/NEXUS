/// 职责边界：
/// 1. 负责真正调用操作系统的 Shell（Windows 调 cmd /C，Linux/macOS 调 sh -c）。
/// 2. 【核心】实现 tokio::time::timeout 控制（默认 60s，最大 600s），超时返回 EXIT_CODE_TIMEOUT。
/// 3. 合并 stdout + stderr，通过 tx 发送一次流式 chunk，再作为最终结果返回。
/// 4. 应用双端截断策略（由 super::truncate_output 实现）。
///
/// 注意：Guardrails（高危命令拦截、路径穿越、SSRF 检测）由 executor.rs 在调用本模块前完成，
/// 本模块只负责"干净地执行"已通过安全检查的命令。

use async_trait::async_trait;
use nexus_common::consts::{EXIT_CODE_ERROR, EXIT_CODE_SUCCESS, EXIT_CODE_TIMEOUT};
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

use super::{LocalTool, ToolResult, truncate_output};

const DEFAULT_TIMEOUT_SEC: u64 = 60;
const MAX_TIMEOUT_SEC: u64 = 600;

pub struct ShellTool;

#[async_trait]
impl LocalTool for ShellTool {
    fn name(&self) -> &'static str {
        "shell"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "shell",
                "description": "Execute a shell command and return its output. Use with caution.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "The shell command to execute"
                        },
                        "working_dir": {
                            "type": "string",
                            "description": "Optional working directory for the command"
                        },
                        "timeout": {
                            "type": "integer",
                            "description": "Timeout in seconds (default 60, max 600)",
                            "minimum": 1,
                            "maximum": 600
                        }
                    },
                    "required": ["command"]
                }
            }
        })
    }

    async fn execute(&self, args: Value, tx: mpsc::Sender<String>) -> ToolResult {
        let command = match args.get("command").and_then(|v| v.as_str()) {
            Some(cmd) if !cmd.trim().is_empty() => cmd.to_string(),
            _ => {
                return ToolResult {
                    exit_code: EXIT_CODE_ERROR,
                    output: "missing required parameter: command".to_string(),
                }
            }
        };

        let working_dir = args
            .get("working_dir")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let timeout_sec = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_SEC)
            .min(MAX_TIMEOUT_SEC);

        run_shell(&command, working_dir.as_deref(), timeout_sec, tx).await
    }
}

async fn run_shell(
    command: &str,
    working_dir: Option<&str>,
    timeout_sec: u64,
    tx: mpsc::Sender<String>,
) -> ToolResult {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("cmd");
        c.args(["/C", command]);
        c
    };

    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", command]);
        c
    };

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    match timeout(Duration::from_secs(timeout_sec), cmd.output()).await {
        Err(_) => ToolResult {
            exit_code: EXIT_CODE_TIMEOUT,
            output: format!("Command timed out after {timeout_sec} seconds"),
        },
        Ok(Err(e)) => ToolResult {
            exit_code: EXIT_CODE_ERROR,
            output: format!("Failed to spawn process: {e}"),
        },
        Ok(Ok(output)) => {
            let mut parts: Vec<String> = Vec::new();

            if !output.stdout.is_empty() {
                parts.push(String::from_utf8_lossy(&output.stdout).into_owned());
            }
            if !output.stderr.is_empty() {
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                if !stderr.trim().is_empty() {
                    parts.push(format!("STDERR:\n{stderr}"));
                }
            }

            let raw_exit = output.status.code().unwrap_or(1);
            parts.push(format!("\nExit code: {raw_exit}"));

            let combined = truncate_output(&parts.join("\n"));

            // 流式回传（fire-and-forget，tx 关闭时忽略错误）
            let _ = tx.send(combined.clone()).await;

            let exit_code = if raw_exit == 0 {
                EXIT_CODE_SUCCESS
            } else {
                EXIT_CODE_ERROR
            };
            ToolResult { exit_code, output: combined }
        }
    }
}
