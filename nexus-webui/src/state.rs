use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::{RwLock, mpsc};

pub type SharedState = Arc<AppState>;

pub struct AppState {
    /// chat_id → browser WS 写端（推消息给浏览器）
    pub browser_conns: Arc<DashMap<String, mpsc::Sender<String>>>,
    /// nexus-server WS 写端（推消息给 nexus）
    pub nexus_tx: Arc<RwLock<Option<mpsc::Sender<String>>>>,
    /// 预期的 nexus-server 认证 token
    pub gateway_token: String,
}

impl AppState {
    pub fn new(gateway_token: String) -> SharedState {
        Arc::new(Self {
            browser_conns: Arc::new(DashMap::new()),
            nexus_tx: Arc::new(RwLock::new(None)),
            gateway_token,
        })
    }
}
