/// 职责边界：
/// 1. 专门管理与 Server 的 WebSocket 长连接 (`tokio_tungstenite`)。
/// 2. 负责断线重连机制 (Exponential Backoff)。
/// 3. 负责维持心跳 (Heartbeat)，定期向 Server 报告 Client 的存活状态和当前工具 Hash。
/// 4. 将收到的 `ServerToClient` 消息推入内部的 MPSC Channel，供 executor 消费。
///
/// 参考 nanobot：
/// - 对应分布式的连接层抽象。保证手脚即使断网，恢复后也能无缝重新接上大脑。

// TODO: pub struct ClientSession { ... }
// TODO: pub async fn connect_and_loop(...)