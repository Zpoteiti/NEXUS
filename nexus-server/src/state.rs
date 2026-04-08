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
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::extract::ws::Message;
use dashmap::DashMap;
use nexus_common::protocol::{FileDownloadResponse, FileUploadResponse, FsPolicy, McpServerEntry, ToolExecutionResult};
use sqlx::PgPool;
use tokio::sync::{mpsc, oneshot, RwLock};

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
    pub mcp_servers: Vec<McpServerEntry>,
    pub last_seen: Instant,
}

#[derive(Clone)]
pub struct AppState {
    pub config: ServerConfig,
    pub db: PgPool,
    /// Online device routing table: device_key -> DeviceState
    pub devices: Arc<DashMap<String, DeviceState>>,
    /// Device name index: user_id -> { device_name -> device_key }
    pub devices_by_user: Arc<DashMap<String, HashMap<String, String>>>,
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
    /// Per-device dirty flag: when true, heartbeat re-queries DB for policy/MCP config.
    pub config_dirty: Arc<DashMap<String, bool>>,
    /// Per-user tool schema cache: user_id -> (generation, cached_schemas)
    pub tool_schema_cache: Arc<DashMap<String, (u64, Vec<serde_json::Value>)>>,
    /// Global generation counter for tool schema changes (bumped on tool register/device disconnect)
    pub tool_schema_generation: Arc<AtomicU64>,
    /// Per-user rate limiter: user_id -> (remaining_tokens, last_refill_time)
    pub rate_limiter: Arc<DashMap<String, (u32, Instant)>>,
    /// Cached rate limit per minute (from system_config). 0 = unlimited. (value, last_check)
    pub rate_limit_cache: Arc<tokio::sync::RwLock<(u32, Instant)>>,
}

impl AppState {
    pub fn new(db: PgPool, config: ServerConfig, bus: Arc<MessageBus>, session_manager: Arc<SessionManager>) -> Self {
        Self {
            config,
            db,
            devices: Arc::new(DashMap::new()),
            devices_by_user: Arc::new(DashMap::new()),
            pending: Arc::new(DashMap::new()),
            file_upload_pending: Arc::new(DashMap::new()),
            file_download_pending: Arc::new(DashMap::new()),
            bus,
            session_manager,
            channel_manager_handle: Arc::new(RwLock::new(None)),
            server_mcp: Arc::new(tokio::sync::RwLock::new(crate::server_mcp::ServerMcpManager::new())),
            config_dirty: Arc::new(DashMap::new()),
            tool_schema_cache: Arc::new(DashMap::new()),
            tool_schema_generation: Arc::new(AtomicU64::new(0)),
            rate_limiter: Arc::new(DashMap::new()),
            rate_limit_cache: Arc::new(tokio::sync::RwLock::new((0, Instant::now()))),
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
                reg.register(Box::new(crate::server_tools::web_fetch::WebFetchTool));
                reg
            }),
        }
    }

    /// Read rate_limit_per_min from DB, cached for 60 seconds. Returns 0 if not configured (unlimited).
    pub async fn get_rate_limit(&self) -> u32 {
        {
            let cache = self.rate_limit_cache.read().await;
            let (limit, last_check) = *cache;
            if last_check.elapsed() < Duration::from_secs(60) {
                return limit;
            }
        }
        let limit = match crate::db::get_system_config(&self.db, "rate_limit_per_min").await {
            Ok(Some(v)) => v.parse::<u32>().unwrap_or(0),
            _ => 0,
        };
        *self.rate_limit_cache.write().await = (limit, Instant::now());
        limit
    }

    /// Check rate limit for a user. Returns Ok(()) if allowed, Err(retry_after_secs) if rate-limited.
    pub async fn check_rate_limit(&self, user_id: &str) -> Result<(), u64> {
        let limit = self.get_rate_limit().await;
        if limit == 0 {
            return Ok(());
        }

        let now = Instant::now();
        let window = Duration::from_secs(60);

        let mut entry = self.rate_limiter.entry(user_id.to_string()).or_insert((limit, now));
        let (ref mut remaining, ref mut last_refill) = *entry;

        // Refill tokens if window has passed
        if now.duration_since(*last_refill) >= window {
            *remaining = limit;
            *last_refill = now;
        }

        if *remaining > 0 {
            *remaining -= 1;
            Ok(())
        } else {
            let elapsed = now.duration_since(*last_refill).as_secs();
            Err(60u64.saturating_sub(elapsed))
        }
    }

    /// Bump the tool schema generation counter, invalidating all cached tool schemas.
    /// Called when devices register/unregister tools or connect/disconnect.
    pub fn bump_tool_schema_generation(&self) {
        self.tool_schema_generation.fetch_add(1, Ordering::Release);
    }

    /// Mark a device's config as dirty so the next heartbeat re-queries DB.
    /// Called after API updates to policy or MCP config.
    pub fn mark_device_config_dirty(&self, user_id: &str, device_name: &str) {
        if let Some(user_devices) = self.devices_by_user.get(user_id) {
            if let Some(device_key) = user_devices.get(device_name) {
                self.config_dirty.insert(device_key.clone(), true);
            }
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
