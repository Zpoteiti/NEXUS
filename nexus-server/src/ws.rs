/// 职责边界：
/// 1. 仅负责处理 `/ws` 路由的 WebSocket 升级请求。
/// 2. 维护单个 WebSocket 连接的收发大循环 (Split Stream & Sink)。
/// 3. 负责在连接时调用 state.rs 将新设备注册到 AppState，断开时注销。
/// 4. 收到 Client 消息时，进行反序列化并打印或分发。

// TODO: 定义 ws_upgrade_handler
// TODO: 定义 socket_receive_loop