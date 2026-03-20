/// 职责边界：
/// 1. 定义和管理全局共享状态 `AppState`，包含两张核心数据表：
///
/// 【第一张】在线设备路由表
///   Arc<RwLock<HashMap<UserId, HashMap<DeviceId, DeviceState>>>>
///   其中 DeviceState 包含：
///   - device_name: String     — 客户端上报的主机名（用于展示和日志）
///   - ws_tx: Sender           — 向该设备推送 ServerToClient 消息的 WebSocket Sender
///   - tools: Vec<Value>       — 该设备当前注册的工具 Schema 列表（JSON Array）
///   - tools_hash: String      — 工具列表的哈希值，用于心跳时快速比对工具是否变更
///   - last_seen: Instant      — 最后一次收到该设备心跳的时刻，用于超时剔除
///
/// 【第二张】工具调用挂起等待表
///   Arc<RwLock<HashMap<RequestId, oneshot::Sender<ToolExecutionResult>>>>
///   - 当 agent_loop 向 Client 下发 ExecuteToolRequest 后，
///     将对应的 oneshot::Sender 存入此表，随即 .await oneshot::Receiver 挂起。
///   - ws.rs 收到 Client 返回的 ToolExecutionResult 后，
///     根据 request_id 从此表取出 oneshot::Sender，调用 .send() 唤醒 agent_loop。
///
/// 两张表均用 Arc<RwLock<...>> 包裹，以便在 ws.rs、agent_loop.rs、api.rs 等多个模块间安全共享。
///
/// 参考 nanobot：
/// - nanobot 的 SessionManager / MemoryStore 基于本地文件持久化（nanobot/agent/memory.py）。
/// - 这里 AppState 维护实时网络拓扑（内存），持久化记忆由 db.rs 通过 SQLx 写入 PostgreSQL。

// TODO: 定义 DeviceState struct
// TODO: 定义 AppState struct（在线设备路由表 + 挂起等待表）
// TODO: 实现 AppState::new() 构造函数

// TODO: pub fn cancel_pending_requests_for_device(
//           device_id: &str,
//           pending: &RwLock<HashMap<RequestId, oneshot::Sender<ToolExecutionResult>>>,
//       )
//   由 ws.rs 在设备断线（WebSocket 循环退出）时调用。
//   遍历挂起等待表，找出所有 request_id 归属 device_id 的条目并将其 Sender drop 掉。
//   Sender 被 drop 后，agent_loop.rs 中对应的 oneshot::Receiver.await 会立即返回
//   Err(RecvError)，agent_loop 将其包装为工具执行失败的 Tool Result 喂回 LLM，
//   触发自我纠正机制，避免 agent_loop 永久挂起（盲区 6-A）。
//
//   实现提示：
//   - RequestId 需要携带归属 device_id 的信息（建议格式："{device_id}:{uuid}"），
//     或在挂起等待表的 value 中额外存储 device_id 字段，以便此函数高效过滤。
//   - 调用方式：ws.rs 在 socket_receive_loop 退出前（defer/drop guard 模式）调用。
