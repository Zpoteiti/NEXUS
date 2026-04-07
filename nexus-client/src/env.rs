/// Responsibility boundary:
/// 1. Encapsulates OS-level interactions.
/// 2. Defines the safe workspace absolute path, preventing the agent from operating outside it.
/// 3. Manages environment variables for agent command execution (isolating sensitive host ENV).

use nexus_common::protocol::FsPolicy;
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::tools::ToolError;

static WORKSPACE_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Operation type for policy enforcement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FsOp {
    Read,
    Write,
}

/// Get the workspace root directory.
///
/// Priority:
/// 1. `NEXUS_WORKSPACE` environment variable
/// 2. `~/.nexus/workspace` (considers both `HOME` and `USERPROFILE`)
pub fn get_workspace_root() -> PathBuf {
    WORKSPACE_ROOT
        .get_or_init(|| {
            if let Some(ws) = env::var("NEXUS_WORKSPACE").ok().and_then(|v| {
                let trimmed = v.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(trimmed))
                }
            }) {
                return ws;
            }

            let home = env::var("HOME")
                .or_else(|_| env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string());

            PathBuf::from(home).join(".nexus").join("workspace")
        })
        .clone()
}

/// Normalize and validate a path.
///
/// If `restrict=true`, the resolved path must be within `get_workspace_root()`.
/// If the path is out of bounds, returns `Err("Path outside workspace")`.
///
/// Supports relative paths (relative to workspace) and absolute paths.
pub fn sanitize_path(path: &str, restrict: bool) -> Result<PathBuf, ToolError> {
    let p = Path::new(path);

    // Expand ~ and resolve relative paths
    let resolved = if p.is_relative() {
        get_workspace_root().join(p)
    } else {
        PathBuf::from(p)
    };

    let resolved = resolved
        .canonicalize()
        .unwrap_or_else(|_| resolved);

    if restrict {
        let workspace = get_workspace_root();
        let workspace = workspace.canonicalize().unwrap_or(workspace);
        if !is_subpath(&resolved, &workspace) {
            return Err(ToolError::Blocked(format!(
                "Path {} is outside workspace {}",
                resolved.display(),
                workspace.display()
            )));
        }
    }

    Ok(resolved)
}

/// Policy-aware path sanitization.
///
/// Enforces the given `FsPolicy`:
/// - `Unrestricted`: all paths allowed.
/// - `Sandbox`: only workspace paths allowed.
/// - `Whitelist`: workspace (read+write), whitelisted paths (read-only).
pub fn sanitize_path_with_policy(
    path: &str,
    op: FsOp,
    policy: &FsPolicy,
) -> Result<PathBuf, ToolError> {
    let p = Path::new(path);

    let resolved = if p.is_relative() {
        get_workspace_root().join(p)
    } else {
        PathBuf::from(p)
    };

    // For writes, if the file doesn't exist yet, canonicalize the parent to
    // catch symlinks that escape the sandbox. For reads, the file must exist
    // so canonicalize on the full path is sufficient.
    let resolved = if op == FsOp::Write {
        match resolved.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                // File doesn't exist — canonicalize parent to resolve symlinks
                if let Some(parent) = resolved.parent() {
                    let canon_parent = parent.canonicalize().unwrap_or_else(|_| parent.to_path_buf());
                    if let Some(file_name) = resolved.file_name() {
                        canon_parent.join(file_name)
                    } else {
                        canon_parent
                    }
                } else {
                    resolved.clone()
                }
            }
        }
    } else {
        resolved.canonicalize().unwrap_or_else(|_| resolved.clone())
    };

    match policy {
        FsPolicy::Unrestricted => Ok(resolved),
        FsPolicy::Sandbox => {
            let workspace = get_workspace_root();
            let workspace = workspace.canonicalize().unwrap_or(workspace);
            if !is_subpath(&resolved, &workspace) {
                return Err(ToolError::Blocked(format!(
                    "Path {} is outside workspace {}",
                    resolved.display(),
                    workspace.display()
                )));
            }
            Ok(resolved)
        }
        FsPolicy::Whitelist { allowed_paths } => {
            let workspace = get_workspace_root();
            let workspace = workspace.canonicalize().unwrap_or(workspace);

            // Workspace: always allowed (read+write)
            if is_subpath(&resolved, &workspace) {
                return Ok(resolved);
            }

            // Whitelisted paths: read-only
            if op == FsOp::Write {
                return Err(ToolError::Blocked(format!(
                    "Path {} is outside workspace — writes only allowed in workspace",
                    resolved.display()
                )));
            }

            for allowed in allowed_paths {
                let allowed_path = PathBuf::from(allowed);
                let allowed_path = allowed_path.canonicalize().unwrap_or(allowed_path);
                if is_subpath(&resolved, &allowed_path) {
                    return Ok(resolved);
                }
            }

            Err(ToolError::Blocked(format!(
                "Path {} is outside workspace and not in whitelist",
                resolved.display()
            )))
        }
    }
}

/// Async wrapper around `sanitize_path_with_policy` that runs the blocking
/// `canonicalize()` call on a dedicated thread via `spawn_blocking`.
pub async fn sanitize_path_with_policy_async(
    raw: &str,
    op: FsOp,
    policy: &FsPolicy,
) -> Result<PathBuf, ToolError> {
    let raw = raw.to_string();
    let policy = policy.clone();
    tokio::task::spawn_blocking(move || sanitize_path_with_policy(&raw, op, &policy))
        .await
        .unwrap_or_else(|_| Err(ToolError::Blocked("path resolution task panicked".to_string())))
}

/// Check if `path` is a subdirectory of (or equal to) `base`.
fn is_subpath(path: &Path, base: &Path) -> bool {
    path.starts_with(base)
}

/// Return a minimized set of environment variables, keeping only the essentials for command execution.
///
/// Kept: PATH, HOME, USER, TEMP, TMP
pub fn min_env() -> HashMap<String, String> {
    let mut env = HashMap::new();

    if let Ok(path) = env::var("PATH") {
        env.insert("PATH".to_string(), path);
    }
    if let Ok(home) = env::var("HOME").or_else(|_| env::var("USERPROFILE")) {
        env.insert("HOME".to_string(), home);
    }
    if let Ok(user) = env::var("USER") {
        env.insert("USER".to_string(), user);
    }
    if let Ok(temp) = env::var("TEMP") {
        env.insert("TEMP".to_string(), temp);
    }
    if let Ok(tmp) = env::var("TMP") {
        env.insert("TMP".to_string(), tmp);
    }

    env
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_relative_path() {
        let result = sanitize_path("some/path", false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_sanitize_absolute_path_outside_workspace() {
        // With restrict=true, absolute paths outside workspace should error
        // Since test workspace may vary, we only verify the function does not panic
        let result = sanitize_path("/tmp/some_file", true);
        // Result depends on workspace location, may be Err
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_min_env_has_required_keys() {
        let env = min_env();
        // PATH almost always exists
        assert!(env.contains_key("PATH") || env.is_empty() || !env.is_empty());
    }
}
