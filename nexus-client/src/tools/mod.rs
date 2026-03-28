/// 职责边界：
/// 1. 定义所有本地原生工具必须实现的 `LocalTool` Trait。
/// 2. 规范工具的名称、JSON Schema 描述，以及执行入口。
/// 3. 统一错误类型 ToolError，映射到 nexus-common 中定义的 exit_code。

use async_trait::async_trait;
use nexus_common::consts::{EXIT_CODE_CANCELLED, EXIT_CODE_ERROR, EXIT_CODE_TIMEOUT};
use serde_json::Value;

/// 本地工具 Trait，所有内置工具必须实现此 Trait。
#[async_trait]
pub trait LocalTool: Send + Sync {
    /// 工具唯一名称
    fn name(&self) -> &'static str;
    /// 工具的 JSON Schema（OpenAI function calling 格式）
    fn schema(&self) -> Value;
    /// 执行工具
    async fn execute(&self, args: Value) -> Result<String, ToolError>;
}

/// 工具执行错误枚举。
///
/// exit_code 映射关系（来自 nexus-common）：
///   ToolError::Timeout          → EXIT_CODE_TIMEOUT (-1)
///   ToolError::Blocked         → EXIT_CODE_CANCELLED (-2)
///   ToolError::NotFound         → EXIT_CODE_ERROR (1)
///   ToolError::InvalidParams    → EXIT_CODE_ERROR (1)
///   ToolError::ExecutionFailed  → EXIT_CODE_ERROR (1)
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("guardrail blocked: {0}")]
    Blocked(String),

    #[error("timeout after {0}s")]
    Timeout(u64),

    #[error("tool not found: {0}")]
    NotFound(String),

    #[error("invalid params: {0}")]
    InvalidParams(String),

    #[error("execution failed: {0}")]
    ExecutionFailed(String),
}

impl ToolError {
    /// 将 ToolError 转换为 exit_code（来自 nexus-common consts）
    pub fn exit_code(&self) -> i32 {
        match self {
            ToolError::Timeout(_) => EXIT_CODE_TIMEOUT,
            ToolError::Blocked(_) => EXIT_CODE_CANCELLED,
            ToolError::NotFound(_)
            | ToolError::InvalidParams(_)
            | ToolError::ExecutionFailed(_) => EXIT_CODE_ERROR,
        }
    }
}

pub mod fs;
pub mod shell;
