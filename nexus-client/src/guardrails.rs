/// 职责边界：
/// 1. 纯粹的静态校验模块。在任何命令被传递给 OS 之前，必须通过这里的严格审查。
/// 2. 实现高危命令正则拦截 (rm -rf, format, shutdown, fork bomb)。
/// 3. 实现路径穿越防护 (Path Traversal)，确保命令不越出指定的 workspace。
/// 4. 实现 SSRF 网络拦截，校验 URL 是否指向 127.0.0.1 或 10.0.0.0/8 等内网段。
///
/// 参考 nanobot：
/// - 对应 `nanobot/agent/tools/shell.py` 的 `_guard_command` 函数。
/// - 对应 `nanobot/security/network.py` 的 CIDR 黑名单检测。

// TODO: pub fn check_shell_command(cmd: &str, workspace: &Path) -> Result<(), String>
//   除正则拦截高危命令外，还需从命令字符串中提取所有 URL (contains_internal_url)，
//   对每个提取到的 URL 调用 check_network_ssrf() 做 SSRF 扫描，
//   防止通过 `curl http://169.254.169.254/...` 等命令绕过防护。
//   参考 nanobot：nanobot/security/network.py contains_internal_url()；
//                nanobot/agent/tools/shell.py _guard_command 的跨文件调用。

// TODO: pub fn check_network_ssrf(url: &str) -> Result<(), String>
//   校验必须覆盖两个层面：
//   1. 静态校验：在请求发出前，对 URL 字符串做内网 IP/域名正则检查。
//   2. 动态校验：若请求发生 HTTP 重定向，对最终落点 URL 做 DNS 解析，
//      检查解析后的 IP 是否属于内网段（防御 DNS rebinding / redirect-to-internal 攻击）。
//   参考 nanobot：nanobot/security/network.py validate_url_target()（L30-62）、
//                validate_resolved_url()（L65-94）；
//                nanobot/agent/tools/web.py 重定向后校验（约 L295-299）。