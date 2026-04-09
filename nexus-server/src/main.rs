mod agent_loop;
mod api;
mod auth;
mod bus;
mod config;
mod context;
mod cron;
mod db;
mod file_store;
mod memory;
mod providers;
mod server_tools;
mod session;
mod state;
mod tools_registry;
mod ws;

use crate::state::AppState;
use axum::routing::get;
use config::ServerConfig;
use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore, mpsc};
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

    let (outbound_tx, mut outbound_rx) = mpsc::channel::<crate::bus::OutboundEvent>(1000);

    // Drain outbound events (channel handlers in M2d will consume these)
    tokio::spawn(async move {
        while let Some(event) = outbound_rx.recv().await {
            tracing::debug!(
                "Outbound [{}]: {} chars to {:?}",
                event.channel,
                event.content.len(),
                event.chat_id
            );
        }
    });

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
        http_client: reqwest::Client::new(),
        web_fetch_client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(
                nexus_common::consts::WEB_FETCH_TIMEOUT_SEC,
            ))
            .connect_timeout(std::time::Duration::from_secs(
                nexus_common::consts::WEB_FETCH_CONNECT_TIMEOUT_SEC,
            ))
            .redirect(reqwest::redirect::Policy::limited(
                nexus_common::consts::WEB_FETCH_MAX_REDIRECTS,
            ))
            .build()
            .expect("Failed to create web_fetch client"),
        outbound_tx,
        shutdown: CancellationToken::new(),
    });

    // Background tasks
    file_store::spawn_cleanup_task();
    ws::spawn_heartbeat_reaper(Arc::clone(&state));
    bus::spawn_rate_limit_refresh(Arc::clone(&state));
    cron::spawn_cron_poller(Arc::clone(&state));

    // Load cached configs from DB
    if let Ok(Some(soul)) = crate::db::system_config::get(&state.db, "default_soul").await {
        *state.default_soul.write().await = Some(soul);
    }
    if let Ok(Some(llm_json)) = crate::db::system_config::get(&state.db, "llm_config").await {
        if let Ok(config) = serde_json::from_str::<crate::config::LlmConfig>(&llm_json) {
            *state.llm_config.write().await = Some(config);
        }
    }
    if let Ok(Some(rl)) = crate::db::system_config::get(&state.db, "rate_limit_per_min").await {
        if let Ok(limit) = rl.parse::<u32>() {
            *state.rate_limit_config.write().await = limit;
        }
    }

    let app = axum::Router::new()
        .merge(auth::auth_routes())
        .merge(auth::device::device_routes())
        .merge(auth::admin::admin_routes())
        .merge(auth::cron_api::cron_api_routes())
        .merge(auth::skills_api::skills_api_routes())
        .merge(api::api_routes())
        .route("/ws", get(ws::ws_handler))
        .with_state(Arc::clone(&state));

    let addr = format!("0.0.0.0:{}", config.server_port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    info!("NEXUS Server listening on {addr}");
    axum::serve(listener, app).await.unwrap();
}
