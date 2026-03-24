/// 职责边界：
/// 1. 实现四个文件系统工具：read_file、write_file、edit_file、list_dir。
/// 2. 对应 nanobot/agent/tools/filesystem.py 的核心逻辑，适配 Rust 的同步文件 API。
/// 3. 不做 Guardrails 检查（由 executor.rs 负责）。

use async_trait::async_trait;
use nexus_common::consts::{EXIT_CODE_ERROR, EXIT_CODE_SUCCESS, MAX_TOOL_OUTPUT_CHARS};
use serde_json::{Value, json};
use std::path::Path;
use tokio::sync::mpsc;

use super::{LocalTool, ToolResult, truncate_output};

// ─────────────────────────────────────────────────────────────────────────────
// read_file
// ─────────────────────────────────────────────────────────────────────────────

const READ_DEFAULT_LIMIT: usize = 2000;

pub struct ReadFileTool;

#[async_trait]
impl LocalTool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read the contents of a file. Returns numbered lines. Use offset and limit to paginate through large files.",
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

    async fn execute(&self, args: Value, _tx: mpsc::Sender<String>) -> ToolResult {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.trim().is_empty() => p.to_string(),
            _ => {
                return ToolResult {
                    exit_code: EXIT_CODE_ERROR,
                    output: "missing required parameter: path".to_string(),
                }
            }
        };
        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(1)
            .max(1);
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(READ_DEFAULT_LIMIT)
            .max(1);

        match read_file_impl(&path, offset, limit) {
            Ok(output) => ToolResult { exit_code: EXIT_CODE_SUCCESS, output },
            Err(e) => ToolResult { exit_code: EXIT_CODE_ERROR, output: e },
        }
    }
}

fn read_file_impl(path: &str, offset: usize, limit: usize) -> Result<String, String> {
    let fp = Path::new(path);
    if !fp.exists() {
        return Err(format!("File not found: {path}"));
    }
    if !fp.is_file() {
        return Err(format!("Not a file: {path}"));
    }

    let raw = std::fs::read(fp).map_err(|e| format!("Error reading file: {e}"))?;
    if raw.is_empty() {
        return Ok(format!("(Empty file: {path})"));
    }

    let text = String::from_utf8_lossy(&raw).into_owned();
    let all_lines: Vec<&str> = text.lines().collect();
    let total = all_lines.len();

    if offset > total {
        return Err(format!(
            "offset {offset} is beyond end of file ({total} lines)"
        ));
    }

    let start = offset - 1;
    let end = (start + limit).min(total);
    let numbered: Vec<String> = all_lines[start..end]
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{}| {}", start + i + 1, line))
        .collect();

    let mut result = numbered.join("\n");

    // 字符数截断
    if result.chars().count() > MAX_TOOL_OUTPUT_CHARS {
        result = truncate_output(&result);
    }

    if end < total {
        result.push_str(&format!(
            "\n\n(Showing lines {offset}-{end} of {total}. Use offset={} to continue.)",
            end + 1
        ));
    } else {
        result.push_str(&format!("\n\n(End of file — {total} lines total)"));
    }

    Ok(result)
}

// ─────────────────────────────────────────────────────────────────────────────
// write_file
// ─────────────────────────────────────────────────────────────────────────────

pub struct WriteFileTool;

#[async_trait]
impl LocalTool for WriteFileTool {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn schema(&self) -> Value {
        json!({
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

    async fn execute(&self, args: Value, _tx: mpsc::Sender<String>) -> ToolResult {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.trim().is_empty() => p.to_string(),
            _ => {
                return ToolResult {
                    exit_code: EXIT_CODE_ERROR,
                    output: "missing required parameter: path".to_string(),
                }
            }
        };
        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => {
                return ToolResult {
                    exit_code: EXIT_CODE_ERROR,
                    output: "missing required parameter: content".to_string(),
                }
            }
        };

        let fp = Path::new(&path);
        if let Some(parent) = fp.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return ToolResult {
                    exit_code: EXIT_CODE_ERROR,
                    output: format!("Failed to create parent directories: {e}"),
                };
            }
        }

        match std::fs::write(fp, content.as_bytes()) {
            Ok(()) => ToolResult {
                exit_code: EXIT_CODE_SUCCESS,
                output: format!("Successfully wrote to {path} ({} chars)", content.chars().count()),
            },
            Err(e) => ToolResult {
                exit_code: EXIT_CODE_ERROR,
                output: format!("Error writing file: {e}"),
            },
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// edit_file
// ─────────────────────────────────────────────────────────────────────────────

pub struct EditFileTool;

#[async_trait]
impl LocalTool for EditFileTool {
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "edit_file",
                "description": "Edit a file by replacing old_text with new_text. Supports minor whitespace/line-ending differences. Set replace_all=true to replace every occurrence.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The file path to edit"
                        },
                        "old_text": {
                            "type": "string",
                            "description": "The text to find and replace"
                        },
                        "new_text": {
                            "type": "string",
                            "description": "The text to replace with"
                        },
                        "replace_all": {
                            "type": "boolean",
                            "description": "Replace all occurrences (default false)"
                        }
                    },
                    "required": ["path", "old_text", "new_text"]
                }
            }
        })
    }

    async fn execute(&self, args: Value, _tx: mpsc::Sender<String>) -> ToolResult {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.trim().is_empty() => p.to_string(),
            _ => {
                return ToolResult {
                    exit_code: EXIT_CODE_ERROR,
                    output: "missing required parameter: path".to_string(),
                }
            }
        };
        let old_text = match args.get("old_text").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => {
                return ToolResult {
                    exit_code: EXIT_CODE_ERROR,
                    output: "missing required parameter: old_text".to_string(),
                }
            }
        };
        let new_text = match args.get("new_text").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => {
                return ToolResult {
                    exit_code: EXIT_CODE_ERROR,
                    output: "missing required parameter: new_text".to_string(),
                }
            }
        };
        let replace_all = args
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        match edit_file_impl(&path, &old_text, &new_text, replace_all) {
            Ok(output) => ToolResult { exit_code: EXIT_CODE_SUCCESS, output },
            Err(e) => ToolResult { exit_code: EXIT_CODE_ERROR, output: e },
        }
    }
}

fn edit_file_impl(
    path: &str,
    old_text: &str,
    new_text: &str,
    replace_all: bool,
) -> Result<String, String> {
    let fp = Path::new(path);
    if !fp.exists() {
        return Err(format!("File not found: {path}"));
    }

    let raw = std::fs::read(fp).map_err(|e| format!("Error reading file: {e}"))?;
    let uses_crlf = raw.windows(2).any(|w| w == b"\r\n");

    // 统一转换为 LF 工作
    let content = String::from_utf8_lossy(&raw)
        .into_owned()
        .replace("\r\n", "\n");
    let normalized_old = old_text.replace("\r\n", "\n");
    let normalized_new = new_text.replace("\r\n", "\n");

    let matched = find_match(&content, &normalized_old);
    let Some(fragment) = matched else {
        return Err(format!(
            "old_text not found in {path}. Verify the file content and try again."
        ));
    };

    // 计算匹配次数（用原始 fragment 查找）
    let count = content.matches(fragment.as_str()).count();
    if count > 1 && !replace_all {
        return Err(format!(
            "old_text appears {count} times in {path}. \
             Provide more context to make it unique, or set replace_all=true."
        ));
    }

    let new_content = if replace_all {
        content.replace(fragment.as_str(), &normalized_new)
    } else {
        content.replacen(fragment.as_str(), &normalized_new, 1)
    };

    // 若原文件使用 CRLF，写回时恢复
    let final_content = if uses_crlf {
        new_content.replace('\n', "\r\n")
    } else {
        new_content
    };

    std::fs::write(fp, final_content.as_bytes())
        .map_err(|e| format!("Error writing file: {e}"))?;

    Ok(format!("Successfully edited {path}"))
}

/// 在 content 中查找 old_text：先精确匹配，再逐行 trim 后滑动窗口匹配。
/// 返回 content 中实际匹配到的原始片段（用于 replace）。
fn find_match(content: &str, old_text: &str) -> Option<String> {
    if content.contains(old_text) {
        return Some(old_text.to_string());
    }

    let old_lines: Vec<&str> = old_text.lines().collect();
    if old_lines.is_empty() {
        return None;
    }
    let stripped_old: Vec<&str> = old_lines.iter().map(|l| l.trim()).collect();
    let content_lines: Vec<&str> = content.lines().collect();

    if content_lines.len() < old_lines.len() {
        return None;
    }

    for i in 0..=(content_lines.len() - old_lines.len()) {
        let window = &content_lines[i..i + old_lines.len()];
        let stripped_window: Vec<&str> = window.iter().map(|l| l.trim()).collect();
        if stripped_window == stripped_old {
            return Some(window.join("\n"));
        }
    }

    None
}

// ─────────────────────────────────────────────────────────────────────────────
// list_dir
// ─────────────────────────────────────────────────────────────────────────────

const LIST_DEFAULT_MAX: usize = 200;

const IGNORE_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "venv",
    "dist",
    "build",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".vs",
    "obj",
    "bin",
];

pub struct ListDirTool;

#[async_trait]
impl LocalTool for ListDirTool {
    fn name(&self) -> &'static str {
        "list_dir"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "list_dir",
                "description": "List the contents of a directory. Set recursive=true to explore nested structure. Common noise directories (.git, node_modules, target, etc.) are auto-ignored.",
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

    async fn execute(&self, args: Value, _tx: mpsc::Sender<String>) -> ToolResult {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.trim().is_empty() => p.to_string(),
            _ => {
                return ToolResult {
                    exit_code: EXIT_CODE_ERROR,
                    output: "missing required parameter: path".to_string(),
                }
            }
        };
        let recursive = args
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let max_entries = args
            .get("max_entries")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(LIST_DEFAULT_MAX)
            .max(1);

        match list_dir_impl(&path, recursive, max_entries) {
            Ok(output) => ToolResult { exit_code: EXIT_CODE_SUCCESS, output },
            Err(e) => ToolResult { exit_code: EXIT_CODE_ERROR, output: e },
        }
    }
}

fn list_dir_impl(path: &str, recursive: bool, max_entries: usize) -> Result<String, String> {
    let dp = Path::new(path);
    if !dp.exists() {
        return Err(format!("Directory not found: {path}"));
    }
    if !dp.is_dir() {
        return Err(format!("Not a directory: {path}"));
    }

    let mut items: Vec<String> = Vec::new();
    let mut total: usize = 0;

    if recursive {
        collect_recursive(dp, dp, &mut items, &mut total, max_entries)
            .map_err(|e| format!("Error listing directory: {e}"))?;
    } else {
        let mut entries: Vec<_> = std::fs::read_dir(dp)
            .map_err(|e| format!("Error listing directory: {e}"))?
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if IGNORE_DIRS.contains(&name_str.as_ref()) {
                continue;
            }
            total += 1;
            if items.len() < max_entries {
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let prefix = if is_dir { "[dir]  " } else { "[file] " };
                items.push(format!("{prefix}{name_str}"));
            }
        }
    }

    if total == 0 {
        return Ok(format!("Directory {path} is empty"));
    }

    let mut result = items.join("\n");
    if total > max_entries {
        result.push_str(&format!(
            "\n\n(truncated, showing first {max_entries} of {total} entries)"
        ));
    }
    Ok(result)
}

fn collect_recursive(
    root: &Path,
    dir: &Path,
    items: &mut Vec<String>,
    total: &mut usize,
    max_entries: usize,
) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // 检查当前条目名是否在忽略列表
        if IGNORE_DIRS.contains(&name_str.as_ref()) {
            continue;
        }

        *total += 1;
        if items.len() < max_entries {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            items.push(if is_dir {
                format!("[dir]  {rel}/")
            } else {
                format!("[file] {rel}")
            });
        }

        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            collect_recursive(root, &path, items, total, max_entries)?;
        }
    }
    Ok(())
}
