use async_trait::async_trait;
use nexus_common::protocol::FsPolicy;
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::time::{timeout, Duration};

use super::fs_helpers::{FS_TOOL_TIMEOUT_SEC, resolve_path_for_read};
use super::{LocalTool, ToolError};

/// Maximum read characters (ref nanobot _MAX_CHARS = 128_000)
const MAX_CHARS: usize = 128_000;
/// Default lines per page
const DEFAULT_LIMIT: usize = 2000;

/// Detect image MIME type via magic bytes.
fn detect_image_mime(data: &[u8]) -> Option<&'static str> {
    if data.len() >= 8 && data[0..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        return Some("image/png");
    }
    if data.len() >= 3 && data[0..3] == [0xFF, 0xD8, 0xFF] {
        return Some("image/jpeg");
    }
    if data.len() >= 6 {
        match &data[0..6] {
            b"GIF87a" | b"GIF89a" => return Some("image/gif"),
            _ => {}
        }
    }
    if data.len() >= 12 && data[0..4] == *b"RIFF" && &data[8..12] == *b"WEBP" {
        return Some("image/webp");
    }
    None
}

pub struct ReadFileTool;

impl ReadFileTool {
    pub fn new() -> Self {
        ReadFileTool
    }
}

#[async_trait]
impl LocalTool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read the contents of a file. Returns numbered lines for text files, or image metadata for image files. Use offset and limit to paginate through large text files.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The file path to read"
                        },
                        "offset": {
                            "type": "integer",
                            "description": "Line number to start reading from (1-indexed, default 1)",
                            "minimum": 1
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of lines to read (default 2000)",
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

impl ReadFileTool {
    pub async fn execute_with_policy(&self, args: Value, policy: &FsPolicy) -> Result<String, ToolError> {
        timeout(Duration::from_secs(FS_TOOL_TIMEOUT_SEC), async {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidParams("missing required field: path".to_string()))?;
            let fp = resolve_path_for_read(path, policy).await?;
            self.read_file_core(&args, fp).await
        })
        .await
        .unwrap_or_else(|_| Err(ToolError::Timeout(FS_TOOL_TIMEOUT_SEC)))
    }

    async fn read_file_core(&self, args: &Value, fp: PathBuf) -> Result<String, ToolError> {
        let path_display = fp.display().to_string();

        if !fp.exists() {
            return Err(ToolError::NotFound(format!("file not found: {}", path_display)));
        }
        if !fp.is_file() {
            return Err(ToolError::InvalidParams(format!("not a file: {}", path_display)));
        }

        let mut file = fs::File::open(&fp)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to open file: {}", e)))?;
        let mut raw = Vec::new();
        file.read_to_end(&mut raw)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to read file: {}", e)))?;

        if raw.is_empty() {
            return Ok(format!("(Empty file: {})", path_display));
        }

        if let Some(_mime) = detect_image_mime(&raw) {
            let size_kb = raw.len() / 1024;
            return Ok(format!("[Image: {}, {}KB]", fp.display(), size_kb));
        }

        let text_content = match String::from_utf8(raw) {
            Ok(s) => s,
            Err(_) => {
                return Err(ToolError::ExecutionFailed(
                    "cannot read binary file (only UTF-8 text and images are supported)".to_string(),
                ));
            }
        };

        let all_lines: Vec<&str> = text_content.split('\n').collect();
        let total = all_lines.len();

        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as usize;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_LIMIT as u64) as usize;

        let offset = offset.max(1);
        if offset > total {
            return Err(ToolError::InvalidParams(format!(
                "offset {} is beyond end of file ({} lines)",
                offset, total
            )));
        }

        let start = offset - 1;
        let end = (start + limit).min(total);
        let mut numbered = Vec::with_capacity(end - start);
        for (i, line) in all_lines[start..end].iter().enumerate() {
            numbered.push(format!("{}| {}", start + i + 1, line));
        }

        let mut result = numbered.join("\n");

        if result.len() > MAX_CHARS {
            let mut chars = 0;
            let mut cut = 0;
            for (i, line) in numbered.iter().enumerate() {
                chars += line.len() + 1;
                if chars > MAX_CHARS {
                    cut = i;
                    break;
                }
            }
            result = numbered[..cut].join("\n");
        }

        if end < total {
            result.push_str(&format!(
                "\n\n(Showing lines {}-{} of {}. Use offset={} to continue.)",
                offset,
                end,
                total,
                end + 1
            ));
        } else {
            result.push_str(&format!("\n\n(End of file — {} lines total)", total));
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_detect_image_png() {
        let png_header = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00];
        assert_eq!(detect_image_mime(&png_header), Some("image/png"));
    }

    #[tokio::test]
    async fn test_detect_image_jpeg() {
        let jpeg_header = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        assert_eq!(detect_image_mime(&jpeg_header), Some("image/jpeg"));
    }

    #[tokio::test]
    async fn test_detect_image_gif() {
        let gif_header = b"GIF89a".to_vec();
        assert_eq!(detect_image_mime(&gif_header), Some("image/gif"));
    }

    #[tokio::test]
    async fn test_detect_image_webp() {
        let webp_header = b"RIFF\x00\x00\x00\x00WEBP".to_vec();
        assert_eq!(detect_image_mime(&webp_header), Some("image/webp"));
    }

    #[tokio::test]
    async fn test_detect_non_image() {
        let text = b"hello world".to_vec();
        assert_eq!(detect_image_mime(&text), None);
    }
}
