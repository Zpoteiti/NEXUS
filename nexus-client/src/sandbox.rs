//! Bubblewrap (bwrap) sandbox for Linux shell command isolation.
//!
//! When active, shell commands run inside a bwrap namespace with:
//! - Workspace directory: read-write
//! - System directories: read-only
//! - Config/secrets: hidden behind tmpfs

use std::path::Path;
use std::sync::LazyLock;

use crate::env::get_workspace_root;

/// Whether bwrap is available on this system (checked once at startup).
static BWRAP_AVAILABLE: LazyLock<bool> = LazyLock::new(|| {
    if !cfg!(target_os = "linux") {
        return false;
    }
    std::process::Command::new("bwrap")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
});

/// Check if bwrap sandbox is available.
pub fn is_available() -> bool {
    *BWRAP_AVAILABLE
}

/// Wrap a shell command in a bwrap sandbox.
///
/// Returns the full bwrap command string. The caller should execute this
/// instead of the original command.
pub fn wrap_command(command: &str, working_dir: &Path) -> String {
    let workspace = get_workspace_root();
    let workspace_str = workspace.display();

    // Resolve cwd relative to workspace
    let cwd = if working_dir.starts_with(&workspace) {
        working_dir.display().to_string()
    } else {
        workspace_str.to_string()
    };

    let mut args: Vec<String> = vec![
        "bwrap".to_string(),
        "--new-session".to_string(),
        "--die-with-parent".to_string(),
    ];

    // Required system directories (read-only)
    args.extend_from_slice(&["--ro-bind".to_string(), "/usr".to_string(), "/usr".to_string()]);

    // Optional system directories (read-only, skip if missing)
    for path in &[
        "/bin",
        "/lib",
        "/lib64",
        "/etc/alternatives",
        "/etc/ssl/certs",
        "/etc/resolv.conf",
        "/etc/ld.so.cache",
    ] {
        args.push("--ro-bind-try".to_string());
        args.push(path.to_string());
        args.push(path.to_string());
    }

    // Proc, dev, tmp
    args.extend_from_slice(&[
        "--proc".to_string(),
        "/proc".to_string(),
        "--dev".to_string(),
        "/dev".to_string(),
        "--tmpfs".to_string(),
        "/tmp".to_string(),
    ]);

    // Mask workspace parent (hides config files like ~/.nexus/)
    if let Some(parent) = workspace.parent() {
        args.push("--tmpfs".to_string());
        args.push(parent.display().to_string());
    }

    // Recreate and bind workspace directory (read-write)
    args.push("--dir".to_string());
    args.push(workspace_str.to_string());
    args.push("--bind".to_string());
    args.push(workspace_str.to_string());
    args.push(workspace_str.to_string());

    // Set working directory and execute
    args.push("--chdir".to_string());
    args.push(cwd);
    args.push("--".to_string());
    args.push("sh".to_string());
    args.push("-c".to_string());
    args.push(command.to_string());

    // Shell-escape each argument
    args.iter()
        .map(|a| shell_escape(a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Simple shell escaping for bwrap arguments.
fn shell_escape(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_alphanumeric() || "-_/.:=".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrap_command_contains_bwrap() {
        let workspace = get_workspace_root();
        let cmd = wrap_command("echo hello", &workspace);
        assert!(cmd.starts_with("bwrap"));
        assert!(cmd.contains("--new-session"));
        assert!(cmd.contains("--die-with-parent"));
        assert!(cmd.contains("echo hello"));
    }

    #[test]
    fn test_wrap_command_contains_workspace() {
        let workspace = get_workspace_root();
        let cmd = wrap_command("ls", &workspace);
        assert!(cmd.contains(&workspace.display().to_string()));
    }

    #[test]
    fn test_shell_escape_simple() {
        assert_eq!(shell_escape("hello"), "hello");
        assert_eq!(shell_escape("/usr/bin"), "/usr/bin");
    }

    #[test]
    fn test_shell_escape_special() {
        assert_eq!(shell_escape("hello world"), "'hello world'");
    }
}
