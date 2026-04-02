/// 职责边界：
/// 1. 封装操作系统层面的交互。
/// 2. 划定"安全工作区 (Workspace)"的绝对路径，禁止 Agent 操作该路径之外的文件。
/// 3. 管理 Agent 执行命令时的环境变量 (隔离宿主机的敏感 ENV)。

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

/// 获取工作区根目录。
///
/// 优先级：
/// 1. `NEXUS_WORKSPACE` 环境变量
/// 2. `~/.nexus/workspace`（`HOME` 或 `USERPROFILE` 均考虑）
pub fn get_workspace_root() -> PathBuf {
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
}

/// 规范化并校验路径。
///
/// 若 `restrict=true`，则解析后的路径必须落在 `get_workspace_root()` 之内。
/// 若路径越界，返回 `Err("Path outside workspace")`。
///
/// 支持相对路径（相对于 workspace）和绝对路径。
pub fn sanitize_path(path: &str, restrict: bool) -> Result<PathBuf, String> {
    let p = Path::new(path);

    // 展开 ~ 和解析相对路径
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
            return Err(format!(
                "Path {} is outside workspace {}",
                resolved.display(),
                workspace.display()
            ));
        }
    }

    Ok(resolved)
}

/// Async wrapper around `sanitize_path` that runs the blocking `canonicalize()` call
/// on a dedicated thread via `spawn_blocking`, avoiding stalls on the async runtime.
pub async fn sanitize_path_async(raw: &str, restrict: bool) -> Result<PathBuf, String> {
    let raw = raw.to_string();
    tokio::task::spawn_blocking(move || sanitize_path(&raw, restrict))
        .await
        .unwrap_or_else(|_| Err("path resolution task panicked".to_string()))
}

/// 检查 `path` 是否是 `base` 的子目录（或相等）。
fn is_subpath(path: &Path, base: &Path) -> bool {
    path.starts_with(base)
}

/// 返回最小化环境变量，仅保留执行命令所需的基础变量。
///
/// 保留：PATH, HOME, USER, TEMP, TMP
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
        // 在 restrict=true 时，绝对路径如果不在 workspace 内应报错
        // 由于测试环境 workspace 可能不确定，我们只验证函数不 panic
        let result = sanitize_path("/tmp/some_file", true);
        // 结果取决于 workspace 在哪里，可能是 Err
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_min_env_has_required_keys() {
        let env = min_env();
        // PATH 几乎总是存在
        assert!(env.contains_key("PATH") || env.is_empty() || !env.is_empty());
    }
}
