/// 职责边界：
/// 1. 定义所有本地原生工具必须实现的 `LocalTool` Trait。
/// 2. 规范工具的名称、JSON Schema 描述，以及执行入口。
/// 3. 【核心】执行入口需要支持流式输出 (Streaming)，通过传入一个 Sender 管道来实时回传 stdout。

use serde_json::Value;
use async_trait::async_trait;

// TODO: 定义 LocalTool trait
/*
#[async_trait]
pub trait LocalTool {
    fn name(&self) -> &'static str;
    fn schema(&self) -> Value;
    // tx 用于流式回传执行日志 (stdout/stderr)
    async fn execute(&self, args: Value, tx: tokio::sync::mpsc::Sender<String>) -> Result<String, String>;
}
*/

pub mod shell;