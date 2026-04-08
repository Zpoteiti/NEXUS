//! Safe environment variables for subprocess execution.
//! Always active — prevents leaking secrets (AWS_SECRET_ACCESS_KEY, DATABASE_URL, etc.)

/// Returns minimal safe environment variables for subprocess execution.
pub fn safe_env() -> Vec<(&'static str, String)> {
    if cfg!(windows) {
        vec![
            (
                "PATH",
                r"C:\Windows\system32;C:\Windows;C:\Windows\System32\Wbem".to_string(),
            ),
            (
                "SYSTEMROOT",
                std::env::var("SYSTEMROOT").unwrap_or_else(|_| r"C:\Windows".to_string()),
            ),
        ]
    } else {
        vec![
            (
                "PATH",
                "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
            ),
            ("HOME", std::env::var("HOME").unwrap_or_default()),
            ("LANG", "en_US.UTF-8".to_string()),
            ("TERM", "xterm-256color".to_string()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_env_has_path() {
        let env = safe_env();
        assert!(env.iter().any(|(k, v)| *k == "PATH" && !v.is_empty()));
    }

    #[test]
    fn test_safe_env_no_secrets() {
        let keys: Vec<&str> = safe_env().iter().map(|(k, _)| *k).collect();
        for secret in &[
            "AWS_SECRET_ACCESS_KEY",
            "DATABASE_URL",
            "NEXUS_AUTH_TOKEN",
            "GITHUB_TOKEN",
        ] {
            assert!(!keys.contains(secret));
        }
    }
}
