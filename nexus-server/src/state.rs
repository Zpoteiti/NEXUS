/// 职责边界：
/// 定义和管理全局共享状态 `AppState`，包含三张核心数据表：
///
/// 【第一张】在线设备路由表
///   device_key（= auth token）→ DeviceState
///   扁平结构。DeviceState 内含 user_id，需要按用户过滤时遍历 values 即可。
///
/// 【第二张】设备名称索引（O(1) 路由查找）
///   user_id → { device_name → device_key }
///   用于 Server 解析 LLM 响应中的 device_name 并路由到对应 Client。
///
/// 【第三张】工具调用挂起等待表
///   request_id → oneshot::Sender<ToolExecutionResult>
///   agent_loop 下发 ExecuteToolRequest 后挂起；ws.rs 收到结果后唤醒。
///   request_id 格式为 "{device_key}:{uuid_v4()}"，按前缀可定位某设备的所有挂起请求。

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
    /// 在线设备路由表：device_key → DeviceState
    pub devices: Arc<RwLock<HashMap<String, DeviceState>>>,
    /// 设备名称索引：user_id → { device_name → device_key }
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

/// 设备断线时由 ws.rs 调用：drop 该设备所有挂起的 oneshot::Sender，
/// 使 agent_loop 的 .await 立即返回 Err，避免永久挂起。
pub async fn cancel_pending_requests_for_device(
    device_key: &str,
    pending: &RwLock<HashMap<String, oneshot::Sender<ToolExecutionResult>>>,
) {
    let prefix = format!("{device_key}:");
    let mut pending_map = pending.write().await;
    pending_map.retain(|request_id, _| !request_id.starts_with(&prefix));
}
