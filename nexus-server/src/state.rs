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
use dashmap::DashMap;
use nexus_common::protocol::{FileDownloadResponse, FileUploadResponse, FsPolicy, ToolExecutionResult};
use sqlx::PgPool;
use tokio::sync::{RwLock, Semaphore, mpsc, oneshot};

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
    /// 在线设备路由表：device_key → DeviceState
    pub devices: Arc<RwLock<HashMap<String, DeviceState>>>,
    /// 设备名称索引：user_id → { device_name → device_key }
    pub devices_by_user: Arc<RwLock<HashMap<String, HashMap<String, String>>>>,
    /// 工具调用挂起等待表：request_id → oneshot::Sender
    pub pending: Arc<DashMap<String, oneshot::Sender<ToolExecutionResult>>>,
    /// 文件上传挂起等待表：request_id → oneshot::Sender
    pub file_upload_pending: Arc<DashMap<String, oneshot::Sender<FileUploadResponse>>>,
    /// 文件下载挂起等待表：request_id → oneshot::Sender
    pub file_download_pending: Arc<DashMap<String, oneshot::Sender<FileDownloadResponse>>>,
    pub bus: Arc<MessageBus>,
    pub session_manager: Arc<SessionManager>,
    /// ChannelManager handle for graceful shutdown — calls stop() on all channels.
    pub channel_manager_handle: Arc<RwLock<Option<ChannelManagerHandle>>>,
    pub embedding_semaphore: Arc<Semaphore>,
    pub server_tools: Arc<ServerToolRegistry>,
    pub server_mcp: Arc<tokio::sync::RwLock<crate::server_mcp::ServerMcpManager>>,
    pub litellm: Arc<crate::litellm::LiteLlmManager>,
}

impl AppState {
    pub fn new(db: PgPool, config: ServerConfig, bus: Arc<MessageBus>, session_manager: Arc<SessionManager>, litellm: Arc<crate::litellm::LiteLlmManager>) -> Self {
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
            embedding_semaphore: Arc::new(Semaphore::new(10)),
            server_mcp: Arc::new(tokio::sync::RwLock::new(crate::server_mcp::ServerMcpManager::new())),
            litellm,
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

impl AppState {
    /// Returns a LlmConfig that routes through the LiteLLM proxy.
    /// The model is set to "default" (LiteLLM's model_name), and
    /// api_base/api_key point to the local LiteLLM instance.
    pub fn litellm_llm_config(&self, base_config: &crate::config::LlmConfig) -> crate::config::LlmConfig {
        crate::config::LlmConfig {
            provider: base_config.provider.clone(),
            model: "default".to_string(),
            api_key: self.litellm.api_key().to_string(),
            api_base: Some(self.litellm.api_base()),
            context_window: base_config.context_window,
            max_output_tokens: base_config.max_output_tokens,
        }
    }
}

/// 设备断线时由 ws.rs 调用：drop 该设备所有挂起的 oneshot::Sender，
/// 使 agent_loop 的 .await 立即返回 Err，避免永久挂起。
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
