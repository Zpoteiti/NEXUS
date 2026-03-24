/// 职责边界：
/// 1. 定义所有本地原生工具必须实现的 `LocalTool` Trait。
/// 2. 规范工具的名称、JSON Schema 描述，以及执行入口。
/// 3. 【核心】执行入口需要支持流式输出 (Streaming)，通过传入一个 Sender 管道来实时回传 stdout。

use async_trait::async_trait;
use nexus_common::consts::{EXIT_CODE_ERROR, MAX_TOOL_OUTPUT_CHARS, TOOL_OUTPUT_HEAD_CHARS, TOOL_OUTPUT_TAIL_CHARS};
use serde_json::Value;
use tokio::sync::mpsc;

pub mod fs;
pub mod shell;

/// 工具执行结果（不含 request_id，由 executor 层包装进 ToolExecutionResult）
pub struct ToolResult {
    pub exit_code: i32,
    pub output: String,
}

#[async_trait]
pub trait LocalTool: Send + Sync {
    fn name(&self) -> &'static str;
    /// 返回 OpenAI function calling 格式的 Schema
    fn schema(&self) -> Value;
    /// tx 用于流式回传执行日志（stdout/stderr chunks）
    async fn execute(&self, args: Value, tx: mpsc::Sender<String>) -> ToolResult;
}

pub fn all_tools() -> Vec<Box<dyn LocalTool>> {
    vec![
        Box::new(shell::ShellTool),
        Box::new(fs::ReadFileTool),
        Box::new(fs::WriteFileTool),
        Box::new(fs::EditFileTool),
        Box::new(fs::ListDirTool),
    ]
}

/// 返回所有内置工具的 Schema 列表，供 RegisterTools 上报给 Server
pub fn get_all_schemas() -> Vec<Value> {
    all_tools().iter().map(|t| t.schema()).collect()
}

/// 按名称分发工具调用
pub async fn execute_tool(name: &str, args: Value, tx: mpsc::Sender<String>) -> ToolResult {
    for tool in all_tools() {
        if tool.name() == name {
            return tool.execute(args, tx).await;
        }
    }
    ToolResult {
        exit_code: EXIT_CODE_ERROR,
        output: format!("tool '{}' not found", name),
    }
}

/// 双端截断：超过 MAX_TOOL_OUTPUT_CHARS 时保留头尾各一段，中间插入省略提示
pub(crate) fn truncate_output(s: &str) -> String {
    let char_count = s.chars().count();
    if char_count <= MAX_TOOL_OUTPUT_CHARS {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let truncated = char_count - MAX_TOOL_OUTPUT_CHARS;
    let head: String = chars[..TOOL_OUTPUT_HEAD_CHARS].iter().collect();
    let tail: String = chars[char_count - TOOL_OUTPUT_TAIL_CHARS..].iter().collect();
    format!("{head}\n\n... ({truncated} chars truncated) ...\n\n{tail}")
}
