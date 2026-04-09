//! Bubblewrap (bwrap) sandbox for Linux. Checked once at startup.
//! If bwrap unavailable, commands execute directly with guardrails + env isolation.

use std::path::Path;
use std::sync::LazyLock;

/// Whether bwrap is available on this system. Probed once at startup.
pub static BWRAP_AVAILABLE: LazyLock<bool> = LazyLock::new(|| {
    if !cfg!(target_os = "linux") {
        tracing::info!("bwrap: non-Linux");
        return false;
    }
    match std::process::Command::new("bwrap")
        .arg("--version")
        .output()
    {
        Ok(o) if o.status.success() => {
            tracing::info!("bwrap: {}", String::from_utf8_lossy(&o.stdout).trim());
            true
        }
        _ => {
            tracing::warn!("bwrap not found — no sandbox container");
            false
        }
    }
});

/// Build a bwrap-wrapped command argument list.
pub fn wrap_command(command: &str, workspace: &Path) -> Vec<String> {
    let ws = workspace.to_string_lossy().to_string();
    let parent = workspace
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or("/".into());
    vec![
        "bwrap".into(),
        "--ro-bind".into(),
        "/usr".into(),
        "/usr".into(),
        "--ro-bind-try".into(),
        "/bin".into(),
        "/bin".into(),
        "--ro-bind-try".into(),
        "/lib".into(),
        "/lib".into(),
        "--ro-bind-try".into(),
        "/lib64".into(),
        "/lib64".into(),
        "--ro-bind-try".into(),
        "/etc/alternatives".into(),
        "/etc/alternatives".into(),
        "--ro-bind-try".into(),
        "/etc/ssl/certs".into(),
        "/etc/ssl/certs".into(),
        "--ro-bind-try".into(),
        "/etc/resolv.conf".into(),
        "/etc/resolv.conf".into(),
        "--ro-bind-try".into(),
        "/etc/ld.so.cache".into(),
        "/etc/ld.so.cache".into(),
        "--proc".into(),
        "/proc".into(),
        "--dev".into(),
        "/dev".into(),
        "--tmpfs".into(),
        "/tmp".into(),
        "--tmpfs".into(),
        parent,
        "--dir".into(),
        ws.clone(),
        "--bind".into(),
        ws.clone(),
        ws,
        "--new-session".into(),
        "--die-with-parent".into(),
        "--".into(),
        "bash".into(),
        "-l".into(),
        "-c".into(),
        format!("'{}'", command.replace('\'', "'\\''")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_structure() {
        let c = wrap_command("echo hi", &PathBuf::from("/home/u/ws"));
        assert_eq!(c[0], "bwrap");
        assert!(c.contains(&"--new-session".into()) && c.contains(&"--die-with-parent".into()));
    }

    #[test]
    fn test_workspace_bind() {
        let c = wrap_command("ls", &PathBuf::from("/home/u/ws"));
        let i = c.iter().position(|a| a == "--bind").unwrap();
        assert_eq!(c[i + 1], "/home/u/ws");
    }
}
