//! Guardrails for Sandbox mode: dangerous command deny-list + SSRF protection.
//! Only active in Sandbox mode — Unrestricted mode skips all checks.

use regex::Regex;
use std::net::IpAddr;
use std::sync::LazyLock;

/// Compiled regex patterns for dangerous commands.
static DENY_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (
            Regex::new(r"\brm\s+-[rf]{1,2}\b").unwrap(),
            "rm -rf/rm -r/rm -f",
        ),
        (Regex::new(r"\bdel\s+/[fq]\b").unwrap(), "del /f or /q"),
        (Regex::new(r"\bformat\s+[a-z]:").unwrap(), "drive format"),
        (Regex::new(r"\bdd\s+if=\b").unwrap(), "dd"),
        (Regex::new(r":\(\)\s*\{.*?\}\s*;\s*:").unwrap(), "fork bomb"),
        (
            Regex::new(r"\b(shutdown|reboot|poweroff|init\s+0|init\s+6)\b").unwrap(),
            "shutdown/reboot",
        ),
        (Regex::new(r">\s*/dev/sd[a-z]").unwrap(), "disk write"),
        (
            Regex::new(r"\b(mkfifo|mknod)\s+/dev/").unwrap(),
            "device creation",
        ),
    ]
});

/// Blocked IP ranges for SSRF protection.
static BLOCKED_RANGES: LazyLock<Vec<ipnet::IpNet>> = LazyLock::new(|| {
    [
        "0.0.0.0/8",
        "10.0.0.0/8",
        "100.64.0.0/10",
        "127.0.0.0/8",
        "169.254.0.0/16",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "::1/128",
        "fc00::/7",
        "fe80::/10",
    ]
    .iter()
    .map(|s| s.parse().unwrap())
    .collect()
});

/// URL extraction regex.
static URL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"https?://[^\s'""]+"#).unwrap());

/// Check command against dangerous pattern deny-list.
pub fn check_deny_list(cmd: &str) -> Option<String> {
    DENY_PATTERNS
        .iter()
        .find(|(p, _)| p.is_match(cmd))
        .map(|(_, d)| format!("Blocked: {d}"))
}

/// Check command for path traversal patterns.
pub fn check_path_traversal(cmd: &str) -> Option<String> {
    if cmd.contains("../") || cmd.contains("..\\") {
        Some("Blocked: path traversal".into())
    } else {
        None
    }
}

fn is_blocked(ip: &IpAddr, whitelist: &[ipnet::IpNet]) -> bool {
    if whitelist.iter().any(|n| n.contains(ip)) {
        return false;
    }
    BLOCKED_RANGES.iter().any(|n| n.contains(ip))
}

/// Check command for URLs targeting internal/private addresses (SSRF).
/// Uses async DNS resolution to avoid blocking the tokio executor.
pub async fn check_ssrf(cmd: &str, whitelist: &[String]) -> Option<String> {
    let wl: Vec<ipnet::IpNet> = whitelist.iter().filter_map(|s| s.parse().ok()).collect();
    for m in URL_RE.find_iter(cmd) {
        let url = m.as_str();
        let host = url
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .split('/')
            .next()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("");
        if host.is_empty() {
            continue;
        }
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_blocked(&ip, &wl) {
                return Some(format!("Blocked: SSRF — {url}"));
            }
            continue;
        }
        match tokio::net::lookup_host(format!("{host}:80")).await {
            Ok(addrs) => {
                for a in addrs {
                    if is_blocked(&a.ip(), &wl) {
                        return Some(format!("Blocked: SSRF — {host} resolves to private IP"));
                    }
                }
            }
            Err(_) => {
                return Some(format!("Blocked: SSRF — DNS failed for {host}"));
            }
        }
    }
    None
}

/// Run all guardrail checks. Returns Some(reason) if any check fails.
pub async fn check_all(cmd: &str, ssrf_whitelist: &[String]) -> Option<String> {
    if let Some(reason) = check_deny_list(cmd) {
        return Some(reason);
    }
    if let Some(reason) = check_path_traversal(cmd) {
        return Some(reason);
    }
    check_ssrf(cmd, ssrf_whitelist).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_rm_rf() {
        assert!(check_deny_list("rm -rf /").is_some());
    }
    #[test]
    fn deny_safe_rm() {
        assert!(check_deny_list("rm file.txt").is_none());
    }
    #[test]
    fn deny_shutdown() {
        assert!(check_deny_list("shutdown -h now").is_some());
    }
    #[test]
    fn deny_fork_bomb() {
        assert!(check_deny_list(":() { :|:& }; :").is_some());
    }
    #[test]
    fn traversal_blocks() {
        assert!(check_path_traversal("cat ../../../etc/passwd").is_some());
    }
    #[test]
    fn traversal_safe() {
        assert!(check_path_traversal("cat file.txt").is_none());
    }
    #[tokio::test]
    async fn ssrf_blocks_localhost() {
        assert!(check_ssrf("curl http://127.0.0.1/", &[]).await.is_some());
    }
    #[tokio::test]
    async fn ssrf_blocks_private() {
        assert!(check_ssrf("curl http://10.0.0.1/", &[]).await.is_some());
    }
    #[tokio::test]
    async fn ssrf_allows_public() {
        assert!(
            check_ssrf("curl https://api.github.com/", &[])
                .await
                .is_none()
        );
    }
    #[tokio::test]
    async fn ssrf_whitelist_overrides() {
        assert!(
            check_ssrf("curl http://10.0.0.1/", &["10.0.0.0/8".into()])
                .await
                .is_none()
        );
    }
    #[tokio::test]
    async fn ssrf_blocks_metadata() {
        assert!(
            check_ssrf("curl http://169.254.169.254/", &[])
                .await
                .is_some()
        );
    }
    #[tokio::test]
    async fn check_all_safe() {
        assert!(check_all("ls -la", &[]).await.is_none());
    }
}
