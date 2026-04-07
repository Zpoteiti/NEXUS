use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::{RwLock, mpsc};

pub type SharedState = Arc<AppState>;

pub struct BrowserConnection {
    pub tx: mpsc::Sender<String>,
    pub user_id: String,
    pub session_id: String,
}

pub struct AppState {
    /// chat_id -> browser connection (push messages to browser)
    pub browser_conns: Arc<DashMap<String, BrowserConnection>>,
    /// nexus-server WS write end (push messages to nexus)
    pub nexus_tx: Arc<RwLock<Option<mpsc::Sender<String>>>>,
    /// Expected nexus-server authentication token
    pub gateway_token: String,
    /// JWT secret for validating browser connections
    pub jwt_secret: String,
    /// Base URL of nexus-server REST API (e.g. http://localhost:8080)
    pub server_api_url: String,
    /// Shared HTTP client for proxying requests (connection pooling)
    pub http_client: reqwest::Client,
}

impl AppState {
    pub fn new(gateway_token: String, jwt_secret: String, server_api_url: String) -> SharedState {
        Arc::new(Self {
            browser_conns: Arc::new(DashMap::new()),
            nexus_tx: Arc::new(RwLock::new(None)),
            gateway_token,
            jwt_secret,
            server_api_url,
            http_client: reqwest::Client::new(),
        })
    }
}
