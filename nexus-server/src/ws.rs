/// 职责边界：
/// 1. 仅负责处理 `/ws` 路由的 WebSocket 升级请求。
/// 2. 维护单个 WebSocket 连接的收发大循环 (Split Stream & Sink)。
/// 3. 负责在连接时调用 state.rs 将新设备注册到 AppState，断开时注销并清理挂起请求。
/// 4. 收到 Client 消息时，进行反序列化并分发。
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【Client 握手认证流程】
/// ─────────────────────────────────────────────────────────────────────────────
/// WebSocket 连接建立后，在进入正常消息循环之前，必须先完成以下握手序列：
///
/// 1. ws.rs 向 Client 发送 ServerToClient::RequireLogin { message: "Please authenticate" }
/// 2. 等待 Client 回复 ClientToServer::SubmitCredentials { email, password_hash }
///    （或 Client 直接上报 JWT token，由实现侧选择认证方式）
/// 3. 调用 auth::verify_jwt(token) 或 auth::login(payload, db) 验证凭据：
///    - 成功：取得 (user_id, device_id)，
///            向 Client 发送 ServerToClient::LoginSuccess { user_id, device_id }，
///            将设备注册到 AppState 在线设备路由表（含 ws_tx、last_seen 等字段），
///            随后进入正常消息收发循环。
///    - 失败：向 Client 发送 ServerToClient::LoginFailed { reason }，
///            立即关闭 WebSocket 连接，不注册到 AppState。
///
/// 握手超时处理：若在 HEARTBEAT_TIMEOUT_SEC 内未收到 SubmitCredentials，
/// 直接关闭连接（防止恶意空连接耗尽连接池）。
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【断线时清理挂起请求（盲区 6-A）】
/// ─────────────────────────────────────────────────────────────────────────────
/// 当 WebSocket 收发循环退出（无论正常断开还是网络异常），必须执行以下清理：
///
/// 1. 从 AppState 在线设备路由表中注销该 (user_id, device_id)。
/// 2. 调用 state::cancel_pending_requests_for_device(device_id, &pending_table)：
///    遍历工具调用挂起等待表，找出所有 request_id 归属该 device_id 的条目，
///    将对应的 oneshot::Sender 全部 drop 掉（Receiver 端会收到 Err(RecvError)）。
/// 3. agent_loop.rs 中 .await oneshot::Receiver 处理 Err 时，
///    将错误包装为 "Tool execution failed: device disconnected" 的 Tool Result，
///    以 tool_result 角色消息喂回 LLM，触发自我纠正机制（agent_loop.rs 已有描述）。
///
/// 若不执行步骤 2，agent_loop 将永久 .await 挂起，该 session 的 LLM 调用链永远无法继续。

// TODO: pub async fn ws_upgrade_handler(
//           ws: WebSocketUpgrade,
//           state: AppState,
//           db: PgPool,
//       ) -> impl IntoResponse
//   执行 WebSocket 升级，将连接传入 socket_receive_loop。

// TODO: pub async fn socket_receive_loop(
//           socket: WebSocket,
//           state: AppState,
//           db: PgPool,
//       )
//   完成握手认证 → 注册设备 → 进入消息收发大循环 → 退出时清理挂起请求。