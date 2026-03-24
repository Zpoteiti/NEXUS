/// 职责边界：
/// 1. 负责在 Client 启动时，收集本地的”物理环境”信息 (OS 类型、架构、当前 workspace 等)。
/// 2. 负责扫描并聚合所有可用的工具：
///    - 收集内置的原生工具 (如 shell, fs)。
///    - 调用 `mcp_client.rs` 收集外部挂载的工具。
///    - (未来) 扫描特定目录下的自定义 .sh / .py 脚本并自动封装成工具。
/// 3. 将聚合后的 Schema 列表组装成 `RegisterTools` 消息发给 Server。
///
/// 参考 nanobot：
/// - 替代 `nanobot` 中启动时的工具注册表加载阶段。

// TODO: pub async fn gather_system_context() -> Value
// TODO: pub async fn discover_all_tools() -> Vec<Value>