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
