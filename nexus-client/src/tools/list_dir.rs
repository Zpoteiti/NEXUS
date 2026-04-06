use async_trait::async_trait;
use nexus_common::protocol::FsPolicy;
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;
use tokio::time::{timeout, Duration};

use super::fs_helpers::{FS_TOOL_TIMEOUT_SEC, resolve_path_for_read};
use super::{LocalTool, ToolError};

/// list_dir default max entries
const LIST_DIR_DEFAULT_MAX: usize = 200;

/// Directories to ignore in list_dir
const IGNORE_DIRS: &[&str] = &[
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

pub struct ListDirTool;

impl ListDirTool {
    pub fn new() -> Self {
        ListDirTool
    }
}

#[async_trait]
impl LocalTool for ListDirTool {
    fn name(&self) -> &'static str {
        "list_dir"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_dir",
                "description": "List the contents of a directory. Set recursive=true to explore nested structure. Common noise directories (.git, node_modules, __pycache__, etc.) are auto-ignored.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The directory path to list"
                        },
                        "recursive": {
                            "type": "boolean",
                            "description": "Recursively list all files (default false)"
                        },
                        "max_entries": {
                            "type": "integer",
                            "description": "Maximum entries to return (default 200)",
                            "minimum": 1
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

impl ListDirTool {
    pub async fn execute_with_policy(&self, args: Value, policy: &FsPolicy) -> Result<String, ToolError> {
        timeout(Duration::from_secs(FS_TOOL_TIMEOUT_SEC), async {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidParams("missing required field: path".to_string()))?;
            let dp = resolve_path_for_read(path, policy).await?;
            Self::list_dir_core(&args, dp).await
        })
        .await
        .unwrap_or_else(|_| Err(ToolError::Timeout(FS_TOOL_TIMEOUT_SEC)))
    }

    async fn list_dir_core(args: &Value, dp: PathBuf) -> Result<String, ToolError> {
        let path_display = dp.display().to_string();

        let recursive = args
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let max_entries = args
            .get("max_entries")
            .and_then(|v| v.as_u64())
            .unwrap_or(LIST_DIR_DEFAULT_MAX as u64) as usize;

        if !dp.exists() {
            return Err(ToolError::NotFound(format!("directory not found: {}", path_display)));
        }
        if !dp.is_dir() {
            return Err(ToolError::InvalidParams(format!("not a directory: {}", path_display)));
        }

        let cap = max_entries.max(1);
        let mut items: Vec<String> = Vec::new();
        let mut total: usize = 0;

        if recursive {
            let mut entries = Vec::new();
            let mut dir_queue: Vec<PathBuf> = vec![dp.clone()];

            while let Some(current) = dir_queue.pop() {
                let read_dir = match fs::read_dir(&current).await {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                let mut stream = tokio_stream::wrappers::ReadDirStream::new(read_dir);
                use tokio_stream::StreamExt;
                while let Some(item) = stream.next().await {
                    if let Ok(entry) = item {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if IGNORE_DIRS.contains(&name.as_str()) {
                            continue;
                        }
                        total += 1;
                        let entry_path = entry.path();
                        let rel = entry_path.strip_prefix(&dp).unwrap_or(&entry_path);
                        let rel_display = rel.display().to_string();
                        if entry_path.is_dir() {
                            entries.push(entry_path);
                            if items.len() < cap {
                                items.push(format!("{}/", rel_display));
                            }
                        } else if items.len() < cap {
                            items.push(rel_display);
                        }
                    }
                }
                for e in entries.drain(..) {
                    dir_queue.push(e);
                }
            }
        } else {
            let read_dir = fs::read_dir(&dp)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("failed to read directory: {}", e)))?;
            let mut stream = tokio_stream::wrappers::ReadDirStream::new(read_dir);
            use tokio_stream::StreamExt;
            while let Some(item) = stream.next().await {
                if let Ok(entry) = item {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if IGNORE_DIRS.contains(&name.as_str()) {
                        continue;
                    }
                    total += 1;
                    if items.len() < cap {
                        let pfx = if entry.path().is_dir() { "[DIR] " } else { "[FILE] " };
                        items.push(format!("{}{}", pfx, name));
                    }
                }
            }
        }

        if items.is_empty() && total == 0 {
            return Ok(format!("Directory {} is empty", path_display));
        }

        items.sort();
        let mut result = items.join("\n");
        if total > cap {
            result.push_str(&format!("\n\n(truncated, showing first {} of {} entries)", cap, total));
        }

        Ok(result)
    }
}
