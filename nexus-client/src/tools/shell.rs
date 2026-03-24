/// 职责边界：
/// 1. 负责真正调用操作系统的 Shell (Windows 调 cmd/powershell，Linux 调 sh/bash)。
/// 2. 【核心】实现 tokio::time::timeout 控制（默认 60s），超时则 Kill 子进程。
/// 3. 【核心】实现输出的双端截断策略：超过 10000 字符时，只保留前 5000 和后 5000，中间插入 "... (X chars truncated) ..."。
///
/// 参考 nanobot：
/// - 对应 `nanobot/agent/tools/shell.py` 的底层执行与截断逻辑。

// TODO: pub async fn execute_with_timeout(cmd: &str) -> Result<String, String>