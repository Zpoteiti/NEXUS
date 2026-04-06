/// Shared path resolution helpers for filesystem tools.

use nexus_common::protocol::FsPolicy;
use std::path::PathBuf;

use super::ToolError;
use crate::env;
use crate::env::FsOp;

/// Per-tool timeout in seconds for filesystem operations.
pub const FS_TOOL_TIMEOUT_SEC: u64 = 30;

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
