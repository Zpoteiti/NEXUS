use async_trait::async_trait;
use nexus_common::protocol::FsPolicy;
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;
use super::fs_helpers::{execute_with_timeout, resolve_path_for_write};
use super::{LocalTool, ToolError};

pub struct WriteFileTool;

impl WriteFileTool {
    pub fn new() -> Self {
        WriteFileTool
    }
}

#[async_trait]
impl LocalTool for WriteFileTool {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "Write content to a file at the given path. Creates parent directories if needed.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The file path to write to"
                        },
                        "content": {
                            "type": "string",
                            "description": "The content to write"
                        }
                    },
                    "required": ["path", "content"]
                }
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String, ToolError> {
        self.execute_with_policy(args, &FsPolicy::Sandbox).await
    }
}

impl WriteFileTool {
    pub async fn execute_with_policy(&self, args: Value, policy: &FsPolicy) -> Result<String, ToolError> {
        execute_with_timeout(|| async {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidParams("missing required field: path".to_string()))?;
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidParams("missing required field: content".to_string()))?
                .to_string();
            let fp = resolve_path_for_write(path, policy).await?;
            Self::write_file_core(fp, content).await
        }).await
    }

    async fn write_file_core(fp: PathBuf, content: String) -> Result<String, ToolError> {
        if let Some(parent) = fp.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("failed to create directory: {}", e)))?;
        }

        fs::write(&fp, content.as_bytes())
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to write file: {}", e)))?;

        Ok(format!("Successfully wrote {} bytes to {}", content.len(), fp.display()))
    }
}
