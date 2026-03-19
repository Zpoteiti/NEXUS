/// 职责边界：
/// 1. 彻底超越 nanobot 的长任务限制，支持后台常驻服务 (Daemon/Background Tasks)。
/// 2. 维护全局进程表：`Lazy<RwLock<HashMap<String, tokio::process::Child>>>`。
/// 3. 【核心特技】使用 `tokio::spawn` 挂起子进程，并通过 `stdout.lines()` 实时将日志流式包装成 `ToolStdoutStream` 发回给 Server。
/// 4. 提供精准的 `kill_process(id)` 接口，让 Agent 可以在未来的对话中随时关闭之前开的服务。

// TODO: pub struct ProcessRegistry { ... }
// TODO: pub async fn spawn_and_stream(cmd: &str, args: &[&str], ws_tx: Sender) -> String
// TODO: pub async fn kill_process(process_id: &str) -> Result<(), String>