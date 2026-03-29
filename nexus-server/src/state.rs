/// 职责边界：
/// 1. 定义和管理全局共享状态 `AppState`，包含三张核心数据表：
///
/// 【第一张】在线设备路由表
///   Arc<RwLock<HashMap<DeviceId, DeviceState>>>
///   扁平结构，按 device_id 索引。DeviceState 内含 user_id，需要按用户过滤时遍历 values 即可。
///   其中 DeviceState 包含：
///   - user_id: String         — 该设备归属的用户
///   - device_name: String     — 客户端上报的人类可读设备名称
///   - ws_tx: Sender           — 向该设备推送 ServerToClient 消息的 WebSocket Sender
///   - tools: Vec<Value>       — 该设备当前注册的工具 Schema 列表（JSON Array）
///   - last_seen: Instant      — 最后一次收到该设备心跳的时刻，用于超时剔除
///
/// 【第二张】设备名称索引（O(1) 路由查找）
///   Arc<RwLock<HashMap<UserId, HashMap<DeviceName, DeviceId>>>>
///   - 实现 user_id + device_name → device_id 的 O(1) 查找
///   - 设备注册时写入，设备注销时删除，重连时更新
///   - 用于 Server 解析 LLM 响应中的 device_name 并路由到对应 Client
///
/// 【第三张】工具调用挂起等待表
///   Arc<RwLock<HashMap<RequestId, oneshot::Sender<ToolExecutionResult>>>>
///   - 当 agent_loop 向 Client 下发 ExecuteToolRequest 后，
///     将对应的 oneshot::Sender 存入此表，随即 .await oneshot::Receiver 挂起。
///   - ws.rs 收到 Client 返回的 ToolExecutionResult 后，
///     根据 request_id 从此表取出 oneshot::Sender，调用 .send() 唤醒 agent_loop。
///   - request_id 格式为 "{device_id}:{uuid_v4()}"，按前缀可高效定位某设备的所有挂起请求。
///
/// 参考 nanobot：
/// - nanobot 的 SessionManager / MemoryStore 基于本地文件持久化（nanobot/agent/memory.py）。
/// - 这里 AppState 维护实时网络拓扑（内存），持久化记忆由 db.rs 通过 SQLx 写入 PostgreSQL。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::Message;
use nexus_common::protocol::ToolExecutionResult;
use sqlx::PgPool;
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::bus::MessageBus;
use crate::config::ServerConfig;
use crate::session::SessionManager;

pub struct DeviceState {
    pub user_id: String,
    pub device_name: String,
    pub ws_tx: mpsc::Sender<Message>,
    pub tools: Vec<serde_json::Value>,
    pub last_seen: Instant,
}

#[derive(Clone)]
pub struct AppState {
    pub config: ServerConfig,
    pub db: PgPool,
    /// 在线设备路由表：device_id → DeviceState
    pub devices: Arc<RwLock<HashMap<String, DeviceState>>>,
    /// 设备名称索引：user_id → { device_name → device_id }，实现 O(1) 路由查找
    pub devices_by_user: Arc<RwLock<HashMap<String, HashMap<String, String>>>>,
    /// 工具调用挂起等待表：request_id → oneshot::Sender
    pub pending: Arc<RwLock<HashMap<String, oneshot::Sender<ToolExecutionResult>>>>,
    pub bus: Arc<MessageBus>,
    pub session_manager: Arc<SessionManager>,
    /// ChannelManager 的 dispatch task handle，用于 graceful shutdown
    /// 使用 Mutex<Option<JoinHandle>> 而非直接存储 JoinHandle，因为 JoinHandle 不是 Clone
    pub channel_manager_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
}

impl AppState {
    pub fn new(db: PgPool, config: ServerConfig, bus: Arc<MessageBus>, session_manager: Arc<SessionManager>) -> Self {
        Self {
            config,
            db,
            devices: Arc::new(RwLock::new(HashMap::new())),
            devices_by_user: Arc::new(RwLock::new(HashMap::new())),
            pending: Arc::new(RwLock::new(HashMap::new())),
            bus,
            session_manager,
            channel_manager_handle: Arc::new(RwLock::new(None)),
        }
    }
}

/// 设备断线时由 ws.rs 调用，清理流程如下：
/// 1. 从 AppState devices 路由表中注销该 (user_id, device_id)。
/// 2. 从 AppState devices_by_user 索引中删除该设备名称。
/// 3. drop 该设备所有挂起的 oneshot::Sender，
/// 使 agent_loop 的 .await 立即返回 Err，避免永久挂起。
/// request_id 格式为 "{device_id}:{uuid}"，按前缀过滤。
pub async fn cancel_pending_requests_for_device(
    device_id: &str,
    pending: &RwLock<HashMap<String, oneshot::Sender<ToolExecutionResult>>>,
) {
    let prefix = format!("{device_id}:");
    let mut pending_map = pending.write().await;
    pending_map.retain(|request_id, _| !request_id.starts_with(&prefix));
}
