mod auth;
mod bus;
mod config;
mod db;
mod session;
mod state;

use crate::state::AppState;
use config::ServerConfig;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = ServerConfig::from_env();
    let pool = db::init_db(&config.database_url).await;

    let (outbound_tx, _outbound_rx) = mpsc::channel(1000);

    let state = Arc::new(AppState {
        db: pool,
        config: config.clone(),
        llm_config: Arc::new(RwLock::new(None)),
        devices: Default::default(),
        devices_by_user: Default::default(),
        pending: Default::default(),
        tool_schema_cache: Default::default(),
        rate_limiter: Default::default(),
        rate_limit_config: Arc::new(RwLock::new(0)),
        default_soul: Arc::new(RwLock::new(None)),
        sessions: Default::default(),
        web_fetch_semaphore: Arc::new(Semaphore::new(
            nexus_common::consts::WEB_FETCH_CONCURRENT_MAX,
        )),
        outbound_tx,
        shutdown: CancellationToken::new(),
    });

    let app = axum::Router::new()
        .merge(auth::auth_routes())
        .with_state(Arc::clone(&state));

    let addr = format!("0.0.0.0:{}", config.server_port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    info!("NEXUS Server listening on {addr}");
    axum::serve(listener, app).await.unwrap();
}
