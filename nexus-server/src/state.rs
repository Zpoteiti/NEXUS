//! Global application state shared across all handlers via Arc.

use crate::bus::OutboundEvent;
use crate::config::{LlmConfig, ServerConfig};
use crate::session::SessionHandle;
use axum::extract::ws::Message;
use dashmap::DashMap;
use futures_util::stream::SplitSink;
use nexus_common::protocol::ToolExecutionResult;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::time::Instant;
use tokio::sync::{Mutex, RwLock, Semaphore, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub type WsSink = SplitSink<axum::extract::ws::WebSocket, Message>;

pub struct DeviceConnection {
    pub user_id: String,
    pub device_name: String,
    pub sink: Arc<Mutex<WsSink>>,
    pub last_seen: Arc<AtomicI64>,
    pub tools: Vec<Value>,
}

pub struct AppState {
    pub db: PgPool,
    pub config: ServerConfig,

    // Hot-reloadable LLM config
    pub llm_config: Arc<RwLock<Option<LlmConfig>>>,

    // Online device routing: "user_id:device_name" -> connection
    pub devices: DashMap<String, DeviceConnection>,
    // user_id -> [device_keys]
    pub devices_by_user: DashMap<String, Vec<String>>,

    // Tool request/response matching: device_key -> { request_id -> sender }
    pub pending: DashMap<String, DashMap<String, oneshot::Sender<ToolExecutionResult>>>,

    // Per-user tool schema cache (Arc to avoid deep-cloning JSON on cache hits)
    pub tool_schema_cache: DashMap<String, Arc<Vec<Value>>>,

    // Rate limiting: user_id -> (remaining, last_refill)
    pub rate_limiter: DashMap<String, (u32, Instant)>,
    pub rate_limit_config: Arc<RwLock<u32>>,

    // Default soul cache
    pub default_soul: Arc<RwLock<Option<String>>>,

    // Session handles
    pub sessions: DashMap<String, SessionHandle>,

    // Web fetch concurrency limit
    pub web_fetch_semaphore: Arc<Semaphore>,

    // Outbound event channel (agent loop -> channel handlers)
    pub outbound_tx: mpsc::Sender<OutboundEvent>,

    // Shared HTTP client (connection pooling for LLM calls + skill install)
    pub http_client: reqwest::Client,

    // Pre-configured HTTP client for web_fetch (custom timeout/redirect)
    pub web_fetch_client: reqwest::Client,

    // Shutdown signal
    pub shutdown: CancellationToken,
}

impl AppState {
    pub fn device_key(user_id: &str, device_name: &str) -> String {
        format!("{user_id}:{device_name}")
    }
}
