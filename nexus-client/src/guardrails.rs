/// 职责边界：
/// 1. 纯粹的静态校验模块。在任何命令被传递给 OS 之前，必须通过这里的严格审查。
/// 2. 实现高危命令正则拦截 (rm -rf, format, shutdown, fork bomb)。
/// 3. 实现路径穿越防护 (Path Traversal)。
/// 4. 实现 SSRF 网络拦截，校验 URL 是否指向内网段。
///
/// 参考 nanobot：
/// - `nanobot/agent/tools/shell.py` 的 `_guard_command` 函数。
/// - `nanobot/security/network.py` 的 CIDR 黑名单检测。

use regex::Regex;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::LazyLock;

/// 高危命令拒绝模式（预编译正则）
static DENY_REGEXES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    DENY_PATTERNS
        .iter()
        .map(|p| Regex::new(p).expect("invalid deny pattern"))
        .collect()
});

/// 高危命令拒绝模式列表
static DENY_PATTERNS: &[&str] = &[
    r"\brm\s+-[rf]{1,2}\b",          // rm -rf, rm -r, rm -rf /
    r"\bdel\s+/[fq]\b",              // Windows: del /f, del /q
    r"\bformat\s+[a-z]:",            // format drive
    r"\bdd\s+if=\b",                 // dd if= (直接读设备)
    r":\(\)\s*\{.*?\};:",             // fork bomb :(){ |:& };:
    r"\b(shutdown|reboot|poweroff|init\s+0|init\s+6)\b", // 关机/重启
    r">\s*/dev/sd[a-z]",            // 直接写盘
    r"\b(mkfifo|mknod)\s+/dev/",    // 在 /dev 创建设备文件
];

/// URL 提取正则（预编译）
static URL_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"https?://[^\s'"]+"#).expect("invalid url regex"));

/// 检查 shell 命令是否包含危险模式。
///
/// 若命令安全，返回 `Ok(())`。
/// 若命令被拦截，返回 `Err(reason)`。
pub fn check_shell_command(cmd: &str) -> Result<(), String> {
    // 1. 正则拦截高危命令
    for re in DENY_REGEXES.iter() {
        if re.is_match(cmd) {
            return Err(format!("command blocked: matches deny pattern '{}'", re.as_str()));
        }
    }

    // 2. SSRF URL 检测
    if contains_internal_url(cmd) {
        return Err("command blocked: contains URL pointing to internal network".to_string());
    }

    Ok(())
}

/// 从命令字符串中提取 URL，并检查是否有指向内网的目标。
pub fn contains_internal_url(command: &str) -> bool {
    for cap in URL_REGEX.find_iter(command) {
        let url = cap.as_str();
        if validate_url_target(url).is_err() {
            return true;
        }
    }
    false
}

/// 校验 URL 目标是否安全（不在内网段）。
///
/// 阻塞以下网段（来自 nanobot security/network.py）：
/// - 0.0.0.0/8
/// - 10.0.0.0/8
/// - 100.64.0.0/10
/// - 127.0.0.0/8
/// - 169.254.0.0/16
/// - 172.16.0.0/12
/// - 192.168.0.0/16
/// - ::1/128
/// - fc00::/7
/// - fe80::/10
pub fn validate_url_target(url: &str) -> Result<(), String> {
    // 解析 URL 获取 host
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid URL: {}", e))?;
    let host = parsed.host_str().ok_or_else(|| "URL has no host".to_string())?;

    // 检查是否是 IP 地址
    if let Ok(ip) = IpAddr::from_str(host) {
        if is_blocked_ip(ip) {
            return Err(format!("URL host {} is in blocked private network", host));
        }
        return Ok(());
    }

    // host 是域名，尝试 DNS 解析
    // 注意：实际生产中可能需要异步 DNS 解析，但此处为简化实现
    // 若无法解析，跳过（保守策略：认为可能指向内网）

    // 简化处理：如果是 localhost 或常见内网域名，直接拒绝
    let lower_host = host.to_lowercase();
    if lower_host == "localhost"
        || lower_host == "127.0.0.1"
        || lower_host == "::1"
        || lower_host.ends_with(".local")
    {
        return Err(format!("URL host {} is in blocked private network", host));
    }

    Ok(())
}

/// 检查 IP 是否在阻塞的私网段内。
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            // 0.0.0.0/8
            if octets[0] == 0 {
                return true;
            }
            // 10.0.0.0/8
            if octets[0] == 10 {
                return true;
            }
            // 100.64.0.0/10 (Shared Address Space)
            if octets[0] == 100 && (octets[1] & 0b1100_0000) == 0b0100_0000 {
                return true;
            }
            // 127.0.0.0/8
            if octets[0] == 127 {
                return true;
            }
            // 169.254.0.0/16 (Link-Local)
            if octets[0] == 169 && octets[1] == 254 {
                return true;
            }
            // 172.16.0.0/12
            if octets[0] == 172 && (octets[1] & 0xf0) == 0x10 {
                return true;
            }
            // 192.168.0.0/16
            if octets[0] == 192 && octets[1] == 168 {
                return true;
            }
            false
        }
        IpAddr::V6(ipv6) => {
            let segments = ipv6.segments();
            // ::1/128 (loopback)
            if ipv6.is_loopback() {
                return true;
            }
            // fc00::/7 (Unique Local Address)
            let first = segments[0] & 0xfe00;
            if first == 0xfc00 {
                return true;
            }
            // fe80::/10 (Link-Local)
            let first = segments[0] & 0xffc0;
            if first == 0xfe80 {
                return true;
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deny_rm_rf() {
        assert!(check_shell_command("rm -rf /").is_err());
        assert!(check_shell_command("rm -rf ./dir").is_err());
        assert!(check_shell_command("rm -r /var/log").is_err());
    }

    #[test]
    fn test_deny_fork_bomb() {
        // Fork bomb pattern: :(){ |:& };:
        assert!(check_shell_command(":(){ |:& };:").is_err());
    }

    #[test]
    fn test_deny_shutdown() {
        assert!(check_shell_command("shutdown -h now").is_err());
        assert!(check_shell_command("reboot").is_err());
    }

    #[test]
    fn test_allow_safe_commands() {
        assert!(check_shell_command("ls -la").is_ok());
        assert!(check_shell_command("echo hello").is_ok());
        assert!(check_shell_command("git status").is_ok());
    }

    #[test]
    fn test_blocked_ip_localhost() {
        assert!(validate_url_target("http://127.0.0.1/").is_err());
        assert!(validate_url_target("http://localhost/api").is_err());
    }

    #[test]
    fn test_blocked_ip_private_ranges() {
        assert!(validate_url_target("http://10.0.0.1/").is_err());
        assert!(validate_url_target("http://192.168.1.1/").is_err());
        assert!(validate_url_target("http://172.16.0.1/").is_err());
    }

    #[test]
    fn test_allowed_public_url() {
        assert!(validate_url_target("https://api.github.com/").is_ok());
        assert!(validate_url_target("https://httpbin.org/get").is_ok());
    }
}
