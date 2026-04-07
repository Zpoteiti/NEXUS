/// Edit file tool — targeted string replacement.
///
/// Finds `old_string` in the file and replaces it with `new_string`.
/// Validates that `old_string` appears exactly once (errors on 0 or >1 matches).

use async_trait::async_trait;
use nexus_common::protocol::FsPolicy;
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;
use tokio::time::{timeout, Duration};

use super::fs_helpers::{FS_TOOL_TIMEOUT_SEC, resolve_path_for_write};
use super::{LocalTool, ToolError};

pub struct EditFileTool;

impl EditFileTool {
    pub fn new() -> Self {
        EditFileTool
    }
}

#[async_trait]
impl LocalTool for EditFileTool {
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "edit_file",
                "description": "Apply a targeted string replacement edit to a file. The old_string must appear exactly once in the file. Use this for surgical edits instead of rewriting entire files.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "The path to the file to edit"
                        },
                        "old_string": {
                            "type": "string",
                            "description": "The exact string to find and replace (must appear exactly once)"
                        },
                        "new_string": {
                            "type": "string",
                            "description": "The replacement string"
                        }
                    },
                    "required": ["file_path", "old_string", "new_string"]
                }
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String, ToolError> {
        self.execute_with_policy(args, &FsPolicy::Sandbox).await
    }
}

impl EditFileTool {
    pub async fn execute_with_policy(
        &self,
        args: Value,
        policy: &FsPolicy,
    ) -> Result<String, ToolError> {
        timeout(Duration::from_secs(FS_TOOL_TIMEOUT_SEC), async {
            let file_path = args
                .get("file_path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    ToolError::InvalidParams("missing required field: file_path".to_string())
                })?;
            let old_string = args
                .get("old_string")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    ToolError::InvalidParams("missing required field: old_string".to_string())
                })?;
            let new_string = args
                .get("new_string")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    ToolError::InvalidParams("missing required field: new_string".to_string())
                })?;

            let fp = resolve_path_for_write(file_path, policy).await?;
            Self::edit_file_core(fp, old_string, new_string).await
        })
        .await
        .unwrap_or_else(|_| Err(ToolError::Timeout(FS_TOOL_TIMEOUT_SEC)))
    }

    async fn edit_file_core(
        fp: PathBuf,
        old_string: &str,
        new_string: &str,
    ) -> Result<String, ToolError> {
        let path_display = fp.display().to_string();

        // File must exist
        if !fp.exists() {
            return Err(ToolError::NotFound(format!(
                "file not found: {}",
                path_display
            )));
        }
        if !fp.is_file() {
            return Err(ToolError::InvalidParams(format!(
                "not a file: {}",
                path_display
            )));
        }

        // Read current content
        let content = fs::read_to_string(&fp)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to read file: {}", e)))?;

        // Validate old_string is non-empty
        if old_string.is_empty() {
            return Err(ToolError::InvalidParams(
                "old_string must not be empty".to_string(),
            ));
        }

        // Count occurrences
        let match_count = content.matches(old_string).count();

        if match_count == 0 {
            return Err(ToolError::InvalidParams(format!(
                "old_string not found in {}",
                path_display
            )));
        }
        if match_count > 1 {
            return Err(ToolError::InvalidParams(format!(
                "old_string found {} times in {} (must appear exactly once)",
                match_count, path_display
            )));
        }

        // Perform replacement
        let new_content = content.replacen(old_string, new_string, 1);

        fs::write(&fp, new_content.as_bytes())
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to write file: {}", e)))?;

        Ok(format!(
            "Successfully edited {} (replaced {} bytes with {} bytes)",
            path_display,
            old_string.len(),
            new_string.len()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_edit_file_success() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "hello world").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let result = EditFileTool::edit_file_core(
            PathBuf::from(&path),
            "hello",
            "goodbye",
        )
        .await;

        assert!(result.is_ok());
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "goodbye world");
    }

    #[tokio::test]
    async fn test_edit_file_not_found() {
        let result = EditFileTool::edit_file_core(
            PathBuf::from("/tmp/nonexistent_edit_test_file_xyz"),
            "hello",
            "goodbye",
        )
        .await;

        assert!(matches!(result, Err(ToolError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_edit_file_no_match() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "hello world").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let result = EditFileTool::edit_file_core(
            PathBuf::from(&path),
            "foobar",
            "baz",
        )
        .await;

        assert!(matches!(result, Err(ToolError::InvalidParams(_))));
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("not found"));
    }

    #[tokio::test]
    async fn test_edit_file_multiple_matches() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "aaa bbb aaa").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let result = EditFileTool::edit_file_core(
            PathBuf::from(&path),
            "aaa",
            "ccc",
        )
        .await;

        assert!(matches!(result, Err(ToolError::InvalidParams(_))));
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("2 times"));
    }

    #[tokio::test]
    async fn test_edit_file_empty_old_string() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "hello world").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let result = EditFileTool::edit_file_core(
            PathBuf::from(&path),
            "",
            "something",
        )
        .await;

        assert!(matches!(result, Err(ToolError::InvalidParams(_))));
    }
}
