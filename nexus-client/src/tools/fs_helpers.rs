/// Shared path resolution helpers for filesystem tools.

use nexus_common::protocol::FsPolicy;
use std::future::Future;
use std::path::PathBuf;
use tokio::time::{timeout, Duration};

use super::ToolError;
use crate::env;
use crate::env::FsOp;

/// Per-tool timeout in seconds for filesystem operations.
pub const FS_TOOL_TIMEOUT_SEC: u64 = 30;

/// Map an IO error to a ToolError with a descriptive message.
pub fn map_io_error(e: std::io::Error, label: &str, path: &str) -> ToolError {
    match e.kind() {
        std::io::ErrorKind::NotFound => ToolError::NotFound(format!("{} not found: {}", label, path)),
        std::io::ErrorKind::PermissionDenied => ToolError::ExecutionFailed(format!("permission denied: {}", path)),
        _ => ToolError::ExecutionFailed(format!("failed to {}: {}", label, e)),
    }
}

/// Wraps a filesystem tool operation with the standard timeout.
pub async fn execute_with_timeout<F, Fut>(f: F) -> Result<String, ToolError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<String, ToolError>>,
{
    timeout(Duration::from_secs(FS_TOOL_TIMEOUT_SEC), f())
        .await
        .unwrap_or_else(|_| Err(ToolError::Timeout(FS_TOOL_TIMEOUT_SEC)))
}

/// Policy-aware path resolution for read operations.
pub async fn resolve_path_for_read(path: &str, policy: &FsPolicy) -> Result<PathBuf, ToolError> {
    if path.is_empty() {
        return Err(ToolError::InvalidParams("path is required".to_string()));
    }
    env::sanitize_path_with_policy_async(path, FsOp::Read, policy).await
}

/// Policy-aware path resolution for write operations.
pub async fn resolve_path_for_write(path: &str, policy: &FsPolicy) -> Result<PathBuf, ToolError> {
    if path.is_empty() {
        return Err(ToolError::InvalidParams("path is required".to_string()));
    }
    env::sanitize_path_with_policy_async(path, FsOp::Write, policy).await
}
