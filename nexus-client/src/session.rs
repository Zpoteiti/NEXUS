/// 职责边界：
/// 1. 专门管理与 Server 的 WebSocket 长连接 (`tokio_tungstenite`)。
/// 2. 负责断线重连机制 (Exponential Backoff)。
/// 3. 负责维持心跳 (Heartbeat)，定期向 Server 报告 Client 的存活状态和当前工具 Hash。
/// 4. 将收到的 `ServerToClient` 消息推入内部的 MPSC Channel，供 executor 消费。
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【心跳与工具热拔插流程】
/// ─────────────────────────────────────────────────────────────────────────────
/// - Client 每次发送心跳前，重新聚合【内置工具 + MCP 工具 + Skill 工具】的完整 Schema 列表，
///   计算其 tools_hash（对合并后的 Vec<Value> 序列化后哈希）。
/// - 若本次 tools_hash 与上次心跳记录的 hash 不同，说明工具集发生了变更
///   （例如用户挂载了新的 MCP Server，或在 skill 目录下新增/删除了 Skill）：
///   则在发出本次 Heartbeat 后，立即再发送一条 ClientToServer::RegisterTools，
///   其 schemas 字段包含三类工具的完整最新列表。
/// - Server 收到新的 RegisterTools 后，更新 AppState 中该设备的工具快照，
///   后续对该设备下发的 ExecuteToolRequest 将基于最新工具列表路由。
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【重连后恢复流程（断线重连 → 状态恢复）】
/// ─────────────────────────────────────────────────────────────────────────────
/// 断线重连成功（WebSocket TCP 连接重新建立）后，不能直接恢复心跳，
/// 必须按以下顺序重走完整握手序列：
///
/// 步骤 1 — 重新完成登录认证：
///   等待 Server 发出 ServerToClient::RequireLogin，
///   回复 ClientToServer::SubmitCredentials（使用 config.auth_token 或缓存的凭据）。
///   Server 断线时已从 AppState 注销该设备，重连后视为全新连接，必须重新认证。
///
/// 步骤 2 — 重新注册工具：
///   收到 ServerToClient::LoginSuccess 后，立即发送一条完整的
///   ClientToServer::RegisterTools（含内置工具 + MCP 工具 + Skill 工具的最新列表），
///   重建 AppState 中该设备的工具快照。
///   （不能等心跳 hash 变更触发，因为 Server 侧工具列表已清空）
///
/// 步骤 3 — 恢复心跳循环：
///   RegisterTools 发送完成后，启动心跳定时器，进入正常运行状态。
///
/// 【重连期间的 ExecuteToolRequest 处理】
/// 重连期间 Server 若有待处理的 ExecuteToolRequest（来自重连前仍在运行的 agent_loop），
/// Server 端 ws.rs 会在设备断线时调用 cancel_pending_requests_for_device，
/// 将对应 oneshot::Sender 全部 drop，agent_loop 收到 Err 后将错误包装为 Tool Result
/// 喂回 LLM 触发自我纠正，无需 Client 侧做额外处理。
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 参考 nanobot：
/// ─────────────────────────────────────────────────────────────────────────────
/// 对应分布式的连接层抽象。nanobot 是单机，无此重连恢复问题；
/// Nexus 的重连序列是 Nexus 架构独有的复杂性来源。

// TODO: pub struct ClientSession { ... }
// TODO: pub async fn connect_and_loop(...)