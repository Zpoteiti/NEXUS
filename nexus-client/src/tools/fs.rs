/// 职责边界：
/// 1. 提供文件系统操作工具：read_file、write_file、list_dir、stat。
/// 2. 所有路径操作必须限制在 workspace 内（通过 `env::sanitize_path`）。
/// 3. read_file 支持文本分页和图片元信息返回。
///
/// 参考 nanobot：
/// - `nanobot/agent/tools/filesystem.py` 的 ReadFileTool、WriteFileTool、ListDirTool。

use async_trait::async_trait;
use nexus_common::protocol::FsPolicy;
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::time::{timeout, Duration};

use super::{LocalTool, ToolError};
use crate::env;
use crate::env::FsOp;

/// Per-tool timeout in seconds for filesystem operations.
const FS_TOOL_TIMEOUT_SEC: u64 = 30;

/// 最大读取字符数（参考 nanobot _MAX_CHARS = 128_000）
const MAX_CHARS: usize = 128_000;
/// 默认每页行数
const DEFAULT_LIMIT: usize = 2000;
/// list_dir 默认最大条目数
const LIST_DIR_DEFAULT_MAX: usize = 200;

/// 检测图片 MIME 类型（magic bytes）。
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

/// 将路径解析为 workspace 内的绝对路径。
#[allow(dead_code)]
fn resolve_path(path: &str) -> Result<PathBuf, ToolError> {
    env::sanitize_path(path, true)
        .map_err(|e| ToolError::InvalidParams(format!("path outside workspace: {}", e)))
}

/// 解析并校验路径，返回 PathBuf。
#[allow(dead_code)]
fn resolve_required_path(path: &str) -> Result<PathBuf, ToolError> {
    if path.is_empty() {
        return Err(ToolError::InvalidParams("path is required".to_string()));
    }
    resolve_path(path)
}

/// Async version of `resolve_path` — runs canonicalize off the async runtime.
async fn resolve_path_async(path: &str) -> Result<PathBuf, ToolError> {
    env::sanitize_path_async(path, true)
        .await
        .map_err(|e| ToolError::InvalidParams(format!("path outside workspace: {}", e)))
}

/// Async version of `resolve_required_path`.
async fn resolve_required_path_async(path: &str) -> Result<PathBuf, ToolError> {
    if path.is_empty() {
        return Err(ToolError::InvalidParams("path is required".to_string()));
    }
    resolve_path_async(path).await
}

/// Policy-aware path resolution for read operations.
async fn resolve_path_for_read(path: &str, policy: &FsPolicy) -> Result<PathBuf, ToolError> {
    if path.is_empty() {
        return Err(ToolError::InvalidParams("path is required".to_string()));
    }
    env::sanitize_path_with_policy_async(path, FsOp::Read, policy)
        .await
        .map_err(|e| ToolError::InvalidParams(format!("path access denied: {}", e)))
}

/// Policy-aware path resolution for write operations.
async fn resolve_path_for_write(path: &str, policy: &FsPolicy) -> Result<PathBuf, ToolError> {
    if path.is_empty() {
        return Err(ToolError::InvalidParams("path is required".to_string()));
    }
    env::sanitize_path_with_policy_async(path, FsOp::Write, policy)
        .await
        .map_err(|e| ToolError::InvalidParams(format!("path access denied: {}", e)))
}

// ---------------------------------------------------------------------------
// read_file
// ---------------------------------------------------------------------------

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
        timeout(Duration::from_secs(FS_TOOL_TIMEOUT_SEC), async {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidParams("missing required field: path".to_string()))?;
            let fp = resolve_required_path_async(path).await?;
            self.read_file_core(&args, fp).await
        })
        .await
        .unwrap_or_else(|_| Err(ToolError::Timeout(FS_TOOL_TIMEOUT_SEC)))
    }
}

impl ReadFileTool {
    /// Execute with policy-aware path resolution.
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

        // 文件不存在
        if !fp.exists() {
            return Err(ToolError::NotFound(format!("file not found: {}", path_display)));
        }
        // 不是文件
        if !fp.is_file() {
            return Err(ToolError::InvalidParams(format!("not a file: {}", path_display)));
        }

        // 读取原始字节
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

        // 图片检测
        if let Some(_mime) = detect_image_mime(&raw) {
            let size_kb = raw.len() / 1024;
            return Ok(format!("[Image: {}, {}KB]", fp.display(), size_kb));
        }

        // 尝试 UTF-8 解码
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

        // 字符数截断
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

        // 追加分页提示
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

// ---------------------------------------------------------------------------
// write_file
// ---------------------------------------------------------------------------

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
        timeout(Duration::from_secs(FS_TOOL_TIMEOUT_SEC), async {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidParams("missing required field: path".to_string()))?;
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidParams("missing required field: content".to_string()))?
                .to_string();
            let fp = resolve_required_path_async(path).await?;
            Self::write_file_core(fp, content).await
        })
        .await
        .unwrap_or_else(|_| Err(ToolError::Timeout(FS_TOOL_TIMEOUT_SEC)))
    }
}

impl WriteFileTool {
    /// Execute with policy-aware path resolution.
    pub async fn execute_with_policy(&self, args: Value, policy: &FsPolicy) -> Result<String, ToolError> {
        timeout(Duration::from_secs(FS_TOOL_TIMEOUT_SEC), async {
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
        })
        .await
        .unwrap_or_else(|_| Err(ToolError::Timeout(FS_TOOL_TIMEOUT_SEC)))
    }

    async fn write_file_core(fp: PathBuf, content: String) -> Result<String, ToolError> {
        // 创建父目录
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

// ---------------------------------------------------------------------------
// list_dir
// ---------------------------------------------------------------------------

/// list_dir 忽略的目录名
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
        timeout(Duration::from_secs(FS_TOOL_TIMEOUT_SEC), async {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidParams("missing required field: path".to_string()))?;
            let dp = resolve_required_path_async(path).await?;
            Self::list_dir_core(&args, dp).await
        })
        .await
        .unwrap_or_else(|_| Err(ToolError::Timeout(FS_TOOL_TIMEOUT_SEC)))
    }
}

impl ListDirTool {
    /// Execute with policy-aware path resolution.
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
            // rglob
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
                // 添加子目录待遍历
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

// ---------------------------------------------------------------------------
// stat
// ---------------------------------------------------------------------------

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
        timeout(Duration::from_secs(FS_TOOL_TIMEOUT_SEC), async {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidParams("missing required field: path".to_string()))?;
            let fp = resolve_required_path_async(path).await?;
            Self::stat_core(fp).await
        })
        .await
        .unwrap_or_else(|_| Err(ToolError::Timeout(FS_TOOL_TIMEOUT_SEC)))
    }
}

impl StatTool {
    /// Execute with policy-aware path resolution.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_path_with_restrict_false_always_succeeds() {
        // restrict=false 时，任何路径都应该成功解析（即使是明显无效的路径）
        let result = env::sanitize_path("/tmp/test_file_xyz123", false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_resolve_path_empty_string_accepted() {
        // 空字符串路径在 sanitize_path 中会被处理
        let result = env::sanitize_path("", false);
        assert!(result.is_ok());
    }

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
