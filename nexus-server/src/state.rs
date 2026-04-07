/// Responsibility boundary:
/// Defines and manages the global shared state `AppState`, containing three core data structures:
///
/// [Table 1] Online device routing table
///   device_key (= auth token) -> DeviceState
///   Flat structure. DeviceState contains user_id; filter by user by iterating values.
///
/// [Table 2] Device name index (O(1) routing lookup)
///   user_id -> { device_name -> device_key }
///   Used by the server to resolve device_name from LLM responses and route to the corresponding client.
///
/// [Table 3] Pending tool call table
///   request_id -> oneshot::Sender<ToolExecutionResult>
///   agent_loop suspends after sending ExecuteToolRequest; ws.rs wakes it when the result arrives.
///   request_id format: "{device_key}:{uuid_v4()}", prefix-searchable by device.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::Message;
use dashmap::DashMap;
use nexus_common::protocol::{FileDownloadResponse, FileUploadResponse, FsPolicy, ToolExecutionResult};
use sqlx::PgPool;
use tokio::sync::{RwLock, mpsc, oneshot};

use crate::bus::MessageBus;
use crate::channels::ChannelManagerHandle;
use crate::config::ServerConfig;
use crate::server_tools::ServerToolRegistry;
use crate::session::SessionManager;

pub struct DeviceState {
    pub user_id: String,
    pub device_name: String,
    pub ws_tx: mpsc::Sender<Message>,
    pub tools: Vec<serde_json::Value>,
    pub fs_policy: FsPolicy,
    pub last_seen: Instant,
}

#[derive(Clone)]
pub struct AppState {
    pub config: ServerConfig,
    pub db: PgPool,
    /// Online device routing table: device_key -> DeviceState
    pub devices: Arc<RwLock<HashMap<String, DeviceState>>>,
    /// Device name index: user_id -> { device_name -> device_key }
    pub devices_by_user: Arc<RwLock<HashMap<String, HashMap<String, String>>>>,
    /// Pending tool call table: request_id -> oneshot::Sender
    pub pending: Arc<DashMap<String, oneshot::Sender<ToolExecutionResult>>>,
    /// Pending file upload table: request_id -> oneshot::Sender
    pub file_upload_pending: Arc<DashMap<String, oneshot::Sender<FileUploadResponse>>>,
    /// Pending file download table: request_id -> oneshot::Sender
    pub file_download_pending: Arc<DashMap<String, oneshot::Sender<FileDownloadResponse>>>,
    pub bus: Arc<MessageBus>,
    pub session_manager: Arc<SessionManager>,
    /// ChannelManager handle for graceful shutdown — calls stop() on all channels.
    pub channel_manager_handle: Arc<RwLock<Option<ChannelManagerHandle>>>,
    pub server_tools: Arc<ServerToolRegistry>,
    pub server_mcp: Arc<tokio::sync::RwLock<crate::server_mcp::ServerMcpManager>>,
}

impl AppState {
    pub fn new(db: PgPool, config: ServerConfig, bus: Arc<MessageBus>, session_manager: Arc<SessionManager>) -> Self {
        Self {
            config,
            db,
            devices: Arc::new(RwLock::new(HashMap::new())),
            devices_by_user: Arc::new(RwLock::new(HashMap::new())),
            pending: Arc::new(DashMap::new()),
            file_upload_pending: Arc::new(DashMap::new()),
            file_download_pending: Arc::new(DashMap::new()),
            bus,
            session_manager,
            channel_manager_handle: Arc::new(RwLock::new(None)),
            server_mcp: Arc::new(tokio::sync::RwLock::new(crate::server_mcp::ServerMcpManager::new())),
            server_tools: Arc::new({
                let mut reg = ServerToolRegistry::new();
                reg.register(Box::new(crate::server_tools::memory::SaveMemoryTool));
                reg.register(Box::new(crate::server_tools::memory::EditMemoryTool));
                reg.register(Box::new(crate::server_tools::send_file::SendFileTool));
                reg.register(Box::new(crate::server_tools::download_to_device::DownloadToDeviceTool));
                reg.register(Box::new(crate::server_tools::message::MessageTool));
                reg.register(Box::new(crate::server_tools::cron::CronCreateTool));
                reg.register(Box::new(crate::server_tools::cron::CronListTool));
                reg.register(Box::new(crate::server_tools::cron::CronRemoveTool));
                reg.register(Box::new(crate::server_tools::skills::ReadSkillTool));
                reg.register(Box::new(crate::server_tools::skills::ReadSkillFileTool));
                reg
            }),
        }
    }
}

/// Called by ws.rs when a device disconnects: drops all pending oneshot::Senders
/// for that device, causing agent_loop's .await to return Err immediately, preventing indefinite hang.
pub fn cancel_pending_requests_for_device(
    device_key: &str,
    pending: &DashMap<String, oneshot::Sender<ToolExecutionResult>>,
    file_upload_pending: &DashMap<String, oneshot::Sender<FileUploadResponse>>,
    file_download_pending: &DashMap<String, oneshot::Sender<FileDownloadResponse>>,
) {
    let prefix = format!("{device_key}:");
    pending.retain(|request_id, _| !request_id.starts_with(&prefix));
    file_upload_pending.retain(|request_id, _| !request_id.starts_with(&prefix));
    file_download_pending.retain(|request_id, _| !request_id.starts_with(&prefix));
}
