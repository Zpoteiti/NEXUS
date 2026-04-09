//! Runtime configuration received from server via LoginSuccess/ConfigUpdate.
//! Stored in Arc<RwLock<ClientConfig>> — single-user client, no DashMap needed.

use nexus_common::protocol::{FsPolicy, McpServerEntry};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub workspace: PathBuf,
    pub fs_policy: FsPolicy,
    pub shell_timeout: u64,
    pub ssrf_whitelist: Vec<String>,
    pub mcp_servers: Vec<McpServerEntry>,
}

/// Expand ~ to home directory and ensure the workspace directory exists.
fn resolve_workspace(path: &str) -> PathBuf {
    let expanded = if path.starts_with("~/") || path == "~" {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".into());
        path.replacen('~', &home, 1)
    } else if path.is_empty() {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".into());
        format!("{home}/.nexus/workspace")
    } else {
        path.to_string()
    };
    let pb = PathBuf::from(&expanded);
    // Best-effort create directory
    let _ = std::fs::create_dir_all(&pb);
    pb
}

impl ClientConfig {
    pub fn from_login(
        workspace_path: String,
        fs_policy: FsPolicy,
        shell_timeout: u64,
        ssrf_whitelist: Vec<String>,
        mcp_servers: Vec<McpServerEntry>,
    ) -> Self {
        Self {
            workspace: resolve_workspace(&workspace_path),
            fs_policy,
            shell_timeout,
            ssrf_whitelist,
            mcp_servers,
        }
    }

    /// Merge a ConfigUpdate. Returns true if mcp_servers changed (caller must reinit MCP).
    pub fn merge_update(
        &mut self,
        fs_policy: Option<FsPolicy>,
        mcp_servers: Option<Vec<McpServerEntry>>,
        workspace_path: Option<String>,
        shell_timeout: Option<u64>,
        ssrf_whitelist: Option<Vec<String>>,
    ) -> bool {
        if let Some(v) = fs_policy {
            self.fs_policy = v;
        }
        if let Some(v) = workspace_path {
            self.workspace = resolve_workspace(&v);
        }
        if let Some(v) = shell_timeout {
            self.shell_timeout = v;
        }
        if let Some(v) = ssrf_whitelist {
            self.ssrf_whitelist = v;
        }
        if let Some(v) = mcp_servers {
            self.mcp_servers = v;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ClientConfig {
        ClientConfig::from_login("/home/u/ws".into(), FsPolicy::Sandbox, 60, vec![], vec![])
    }

    #[test]
    fn test_from_login() {
        let c = cfg();
        assert_eq!(c.workspace, PathBuf::from("/home/u/ws"));
        assert_eq!(c.fs_policy, FsPolicy::Sandbox);
    }

    #[test]
    fn test_merge_partial() {
        let mut c = cfg();
        let mcp = c.merge_update(Some(FsPolicy::Unrestricted), None, None, Some(120), None);
        assert!(!mcp);
        assert_eq!(c.fs_policy, FsPolicy::Unrestricted);
        assert_eq!(c.shell_timeout, 120);
    }

    #[test]
    fn test_merge_mcp_returns_true() {
        let mut c = cfg();
        let mcp = c.merge_update(
            None,
            Some(vec![McpServerEntry {
                name: "t".into(),
                transport_type: None,
                command: "e".into(),
                args: vec![],
                env: None,
                url: None,
                headers: None,
                tool_timeout: None,
                enabled: true,
            }]),
            None,
            None,
            None,
        );
        assert!(mcp);
        assert_eq!(c.mcp_servers.len(), 1);
    }

    #[test]
    fn test_merge_none_preserves() {
        let mut c = cfg();
        c.merge_update(None, None, None, None, None);
        assert_eq!(c.fs_policy, FsPolicy::Sandbox);
    }
}
