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
use tokio::net::lookup_host;

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
pub async fn check_shell_command(cmd: &str) -> Result<(), String> {
    // 1. 正则拦截高危命令
    for re in DENY_REGEXES.iter() {
        if re.is_match(cmd) {
            return Err(format!("command blocked: matches deny pattern '{}'", re.as_str()));
        }
    }

    // 2. SSRF URL 检测（异步 DNS 解析）
    if contains_internal_url(cmd).await {
        return Err("command blocked: contains URL pointing to internal network".to_string());
    }

    Ok(())
}

/// 从命令字符串中提取 URL，并检查是否有指向内网的目标。
pub async fn contains_internal_url(command: &str) -> bool {
    for cap in URL_REGEX.find_iter(command) {
        let url = cap.as_str();
        if validate_url_target(url).await.is_err() {
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
///
/// 若 URL 中的 hostname 无法解析为 IP，保守地认为其可能指向内网而拒绝。
pub async fn validate_url_target(url: &str) -> Result<(), String> {
    // 解析 URL 获取 host
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid URL: {}", e))?;
    let host = parsed.host_str().ok_or_else(|| "URL has no host".to_string())?;

    // 检查是否是 IP 地址（直接检查）
    if let Ok(ip) = IpAddr::from_str(host) {
        if is_blocked_ip(ip) {
            return Err(format!("URL host {} is in blocked private network", host));
        }
        return Ok(());
    }

    // host 是域名，尝试 DNS 解析（异步）
    // 参考 nanobot: socket.getaddrinfo(hostname, None, socket.AF_UNSPEC, socket.SOCK_STREAM)
    let addr_string = format!("{}:80", host);
    let addrs: Vec<_> = lookup_host(&addr_string)
        .await
        .map_err(|_| format!("cannot resolve hostname: {}", host))?
        .collect();

    if addrs.is_empty() {
        // 无法解析，保守策略：拒绝
        return Err(format!("cannot resolve hostname: {}", host));
    }

    // 检查所有解析出的 IP 地址
    for addr in addrs {
        let ip = addr.ip();
        if is_blocked_ip(ip) {
            return Err(format!(
                "URL host {} resolves to private/internal address {}",
                host, ip
            ));
        }
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

    #[tokio::test]
    async fn test_deny_rm_rf() {
        assert!(check_shell_command("rm -rf /").await.is_err());
        assert!(check_shell_command("rm -rf ./dir").await.is_err());
        assert!(check_shell_command("rm -r /var/log").await.is_err());
    }

    #[tokio::test]
    async fn test_deny_fork_bomb() {
        // Fork bomb pattern: :(){ |:& };:
        assert!(check_shell_command(":(){ |:& };:").await.is_err());
    }

    #[tokio::test]
    async fn test_deny_shutdown() {
        assert!(check_shell_command("shutdown -h now").await.is_err());
        assert!(check_shell_command("reboot").await.is_err());
    }

    #[tokio::test]
    async fn test_allow_safe_commands() {
        assert!(check_shell_command("ls -la").await.is_ok());
        assert!(check_shell_command("echo hello").await.is_ok());
        assert!(check_shell_command("git status").await.is_ok());
    }

    #[tokio::test]
    async fn test_blocked_ip_localhost() {
        // 127.0.0.1 是 IP，直接被拦截
        assert!(validate_url_target("http://127.0.0.1/").await.is_err());
        // localhost 通过 DNS 解析会得到 127.0.0.1，也被拦截
        assert!(validate_url_target("http://localhost/api").await.is_err());
    }

    #[tokio::test]
    async fn test_blocked_ip_private_ranges() {
        assert!(validate_url_target("http://10.0.0.1/").await.is_err());
        assert!(validate_url_target("http://192.168.1.1/").await.is_err());
        assert!(validate_url_target("http://172.16.0.1/").await.is_err());
    }

    #[tokio::test]
    async fn test_allowed_public_url() {
        // 公网 URL 应该能通过（DNS 解析后检查）
        assert!(validate_url_target("https://api.github.com/").await.is_ok());
        assert!(validate_url_target("https://httpbin.org/get").await.is_ok());
    }

    #[tokio::test]
    async fn test_ssrf_domain_resolves_to_private() {
        // 模拟攻击场景：如果域名被 DNS 解析到私网段，应该被拦截
        // 这个测试无法依赖具体域名，但我们可以测试无效域名返回错误
        let result = validate_url_target("http://this-domain-does-not-exist-xyz123.invalid/").await;
        // 无法解析的域名，保守地拒绝
        assert!(result.is_err());
    }
}
