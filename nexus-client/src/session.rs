/// 职责边界：
/// 1. 专门管理与 Server 的 WebSocket 长连接 (`tokio_tungstenite`)。
/// 2. 负责断线重连机制 (Exponential Backoff)。
/// 3. 负责维持心跳 (Heartbeat)，定期向 Server 报告 Client 的存活状态和当前工具 Hash。
/// 4. 将收到的 `ServerToClient` 消息推入内部的 MPSC Channel，供 executor 消费。
///
/// 心跳与工具热拔插流程：
/// - Client 每次发送心跳前，重新聚合【内置工具 + MCP 工具 + Skill 工具】的完整 Schema 列表，
///   计算其 tools_hash（对合并后的 Vec<Value> 序列化后哈希）。
/// - 若本次 tools_hash 与上次心跳记录的 hash 不同，说明工具集发生了变更
///   （例如用户挂载了新的 MCP Server，或在 skill 目录下新增/删除了 Skill）：
///   则在发出本次 Heartbeat 后，立即再发送一条 ClientToServer::RegisterTools，
///   其 schemas 字段包含三类工具的完整最新列表。
/// - Server 收到新的 RegisterTools 后，更新 AppState 中该设备的工具快照，
///   后续对该设备下发的 ExecuteToolRequest 将基于最新工具列表路由。
///
/// 参考 nanobot：
/// - 对应分布式的连接层抽象。保证手脚即使断网，恢复后也能无缝重新接上大脑。

// TODO: pub struct ClientSession { ... }
// TODO: pub async fn connect_and_loop(...)