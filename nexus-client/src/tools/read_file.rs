use crate::config::ClientConfig;
use crate::tools::helpers::{sanitize_path, tool_error};
use crate::tools::{Tool, ToolResult};
use nexus_common::consts::{DEFAULT_READ_FILE_LIMIT, MAX_READ_FILE_CHARS};
use nexus_common::mime::detect_mime_from_bytes;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

pub struct ReadFileTool;

impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Read file contents with line numbers. Images return metadata."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{
            "path":{"type":"string","description":"File path"},
            "offset":{"type":"integer","description":"Start line (1-indexed)","default":1},
            "limit":{"type":"integer","description":"Max lines","default":2000}
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
    let path_str = match args.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => return ToolResult::error(tool_error("missing: path")),
    };
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_READ_FILE_LIMIT as u64) as usize;

    let path = match sanitize_path(path_str, config, false) {
        Ok(p) => p,
        Err(e) => return ToolResult::error(e),
    };

    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(e) => {
            return ToolResult::error(tool_error(&format!(
                "file not found: {path_str}\n{e}"
            )))
        }
    };

    if let Some(mime) = detect_mime_from_bytes(&bytes)
        && mime.starts_with("image/")
    {
        return ToolResult::success(format!(
            "[Image: {path_str}, {}KB]",
            bytes.len() / 1024
        ));
    }

    let content = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return ToolResult::error(tool_error(&format!("binary file: {path_str}"))),
    };

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let start = (offset - 1).min(total);
    let end = (start + limit).min(total);

    let mut output = String::new();
    for (i, line) in lines[start..end].iter().enumerate() {
        output.push_str(&format!("{}| {line}\n", start + i + 1));
    }

    if output.len() > MAX_READ_FILE_CHARS {
        output.truncate(MAX_READ_FILE_CHARS);
        output.push_str("\n... (truncated)");
    }
    if end < total {
        output.push_str(&format!(
            "\nShowing lines {}-{} of {total}. Use offset to read more.",
            start + 1,
            end
        ));
    }

    ToolResult::success(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_common::protocol::FsPolicy;

    fn cfg(dir: &std::path::Path) -> ClientConfig {
        ClientConfig {
            workspace: dir.to_path_buf(),
            fs_policy: FsPolicy::Unrestricted,
            shell_timeout: 60,
            ssrf_whitelist: vec![],
            mcp_servers: vec![],
        }
    }

    #[tokio::test]
    async fn test_basic() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("t.txt"), "a\nb\nc\n").unwrap();
        let r = exec(
            serde_json::json!({"path": d.path().join("t.txt").to_str().unwrap()}),
            &cfg(d.path()),
        )
        .await;
        assert_eq!(r.exit_code, 0);
        assert!(r.output.contains("1| a") && r.output.contains("3| c"));
    }

    #[tokio::test]
    async fn test_offset_limit() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("b.txt"),
            (1..=100)
                .map(|i| format!("line{i}\n"))
                .collect::<String>(),
        )
        .unwrap();
        let r = exec(
            serde_json::json!({"path": d.path().join("b.txt").to_str().unwrap(), "offset": 50, "limit": 5}),
            &cfg(d.path()),
        )
        .await;
        assert!(r.output.contains("50| line50") && r.output.contains("Showing lines 50-54"));
    }

    #[tokio::test]
    async fn test_image() {
        let d = tempfile::tempdir().unwrap();
        let mut data = vec![0x89, b'P', b'N', b'G'];
        data.extend_from_slice(&[0u8; 1000]);
        std::fs::write(d.path().join("i.png"), &data).unwrap();
        let r = exec(
            serde_json::json!({"path": d.path().join("i.png").to_str().unwrap()}),
            &cfg(d.path()),
        )
        .await;
        assert!(r.output.contains("[Image:"));
    }

    #[tokio::test]
    async fn test_not_found() {
        let d = tempfile::tempdir().unwrap();
        let r = exec(
            serde_json::json!({"path": "/no/such/file"}),
            &cfg(d.path()),
        )
        .await;
        assert_eq!(r.exit_code, 1);
    }
}
