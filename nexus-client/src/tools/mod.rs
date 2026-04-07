/// Responsibility boundary:
/// 1. Defines the `LocalTool` trait that all local native tools must implement.
/// 2. Standardizes tool names, JSON Schema descriptions, and execution entry points.
/// 3. Unified error type ToolError, mapped to exit_code defined in nexus-common.

use async_trait::async_trait;
use nexus_common::consts::{EXIT_CODE_CANCELLED, EXIT_CODE_ERROR, EXIT_CODE_TIMEOUT};
use serde_json::Value;

/// Local tool trait. All built-in tools must implement this trait.
#[async_trait]
pub trait LocalTool: Send + Sync {
    /// Unique tool name.
    fn name(&self) -> &'static str;
    /// Tool JSON Schema (OpenAI function calling format).
    fn schema(&self) -> Value;
    /// Execute the tool.
    async fn execute(&self, args: Value) -> Result<String, ToolError>;
}

/// Tool execution error enum.
///
/// exit_code mapping (from nexus-common):
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
    /// Convert ToolError to exit_code (from nexus-common consts).
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

pub mod edit;
pub mod fs;
pub mod fs_helpers;
mod read_file;
mod write_file;
mod list_dir;
mod stat;
pub mod shell;
