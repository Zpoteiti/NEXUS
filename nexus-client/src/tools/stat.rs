use async_trait::async_trait;
use nexus_common::protocol::FsPolicy;
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;
use tokio::time::{timeout, Duration};

use super::fs_helpers::{FS_TOOL_TIMEOUT_SEC, resolve_path_for_read};
use super::{LocalTool, ToolError};

pub struct StatTool;

impl StatTool {
    pub fn new() -> Self {
        StatTool
    }
}

#[async_trait]
impl LocalTool for StatTool {
    fn name(&self) -> &'static str {
        "stat"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "stat",
                "description": "Get metadata information about a file or directory (size, modified time, etc.)",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The file or directory path to stat"
                        }
                    },
                    "required": ["path"]
                }
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String, ToolError> {
        self.execute_with_policy(args, &FsPolicy::Sandbox).await
    }
}

impl StatTool {
    pub async fn execute_with_policy(&self, args: Value, policy: &FsPolicy) -> Result<String, ToolError> {
        timeout(Duration::from_secs(FS_TOOL_TIMEOUT_SEC), async {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidParams("missing required field: path".to_string()))?;
            let fp = resolve_path_for_read(path, policy).await?;
            Self::stat_core(fp).await
        })
        .await
        .unwrap_or_else(|_| Err(ToolError::Timeout(FS_TOOL_TIMEOUT_SEC)))
    }

    async fn stat_core(fp: PathBuf) -> Result<String, ToolError> {
        if !fp.exists() {
            return Err(ToolError::NotFound(format!("path not found: {}", fp.display())));
        }

        let metadata = fs::metadata(&fp)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to stat: {}", e)))?;

        let file_type = if metadata.is_dir() {
            "directory"
        } else if metadata.is_file() {
            "file"
        } else if metadata.is_symlink() {
            "symlink"
        } else {
            "unknown"
        };

        let size = metadata.len();
        let modified = metadata
            .modified()
            .map(|t| {
                let datetime: chrono::DateTime<chrono::Local> = t.into();
                datetime.format("%Y-%m-%d %H:%M:%S").to_string()
            })
            .unwrap_or_else(|_| "unknown".to_string());

        Ok(format!(
            "Path: {}\nType: {}\nSize: {} bytes\nModified: {}",
            fp.display(),
            file_type,
            size,
            modified
        ))
    }
}
