/// 职责边界：
/// 1. 负责程序的启动、环境变量读取 (.env)、数据库连接池 (PgPool) 的初始化。
/// 2. 调用 bus::init() 创建消息管道，初始化 AppState，启动 ChannelManager。
/// 3. 挂载 Axum 的路由（HTTP API 路由来自 api.rs，WebSocket 路由来自 ws.rs）。
/// 4. 绝对不要在这里写具体的 WebSocket 收发逻辑或 LLM 提示词逻辑。

mod agent_loop;
mod api;
mod auth;
mod bus;
mod channels;
mod config;
mod context;
mod cron;
mod db;
mod litellm;
mod memory;
mod providers;
mod server_mcp;
mod server_tools;
mod session;
mod state;
mod tools_registry;
mod ws;

use axum::Router;
use axum::response::IntoResponse;
use axum::routing::get;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, info};

use bus::MessageBus;
use channels::ChannelManager;
use channels::discord::DiscordChannel;
use channels::gateway::GatewayChannel;
use session::SessionManager;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config = config::load_config();

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(50)
        .connect(&config.database_url)
        .await
        .unwrap_or_else(|e| panic!("Failed to connect PostgreSQL: {e}"));

    db::init_db(&pool)
        .await
        .unwrap_or_else(|e| panic!("Failed to initialize database: {e}"));

    // Initialize LiteLLM proxy manager
    let litellm_mgr = Arc::new(litellm::LiteLlmManager::new(
        config.litellm_port,
        Some(config.database_url.clone()),
    ));

    // Set up LiteLLM: create venv, install, and start proxy
    if let Err(e) = litellm_mgr.ensure_setup().await {
        error!("LiteLLM setup failed: {}", e);
        error!("LLM functionality will be unavailable until LiteLLM is set up.");
    } else if let Err(e) = litellm_mgr.start().await {
        error!("LiteLLM start failed: {}", e);
        error!("LLM functionality will be unavailable until LiteLLM is running.");
    }

    // Load persisted configs from DB
    if let Ok(Some(llm_json)) = db::get_system_config(&pool, "llm_config").await {
        if let Ok(llm) = serde_json::from_str::<config::LlmConfig>(&llm_json) {
            // Register the model with LiteLLM proxy
            if let Err(e) = litellm_mgr.add_model(
                &llm.provider,
                &llm.model,
                &llm.api_key,
                llm.api_base.as_deref(),
            ).await {
                error!("Failed to register LLM model with LiteLLM: {}", e);
            }
            *config.llm.write().await = Some(llm);
            info!("Loaded LLM config from database");
        }
    }
    if let Ok(Some(emb_json)) = db::get_system_config(&pool, "embedding_config").await {
        if let Ok(emb) = serde_json::from_str::<config::EmbeddingConfig>(&emb_json) {
            *config.embedding.write().await = Some(emb);
            info!("Loaded embedding config from database");
        }
    }

    // 创建 MessageBus
    let bus = Arc::new(MessageBus::new());

    // 创建 SessionManager
    let session_manager = Arc::new(SessionManager::new());

    // 创建 AppState
    let state = state::AppState::new(pool, config.clone(), bus.clone(), session_manager, litellm_mgr.clone());
    let state_arc = Arc::new(state);

    // Load server MCP config from DB and initialize
    if let Ok(Some(mcp_json)) = db::get_system_config(&state_arc.db, "server_mcp_config").await {
        if let Ok(entries) = serde_json::from_str::<Vec<nexus_common::protocol::McpServerEntry>>(&mcp_json) {
            if !entries.is_empty() {
                let mut manager = state_arc.server_mcp.write().await;
                manager.initialize(&entries).await;
                info!("Loaded server MCP config: {} servers", entries.len());
            }
        }
    }

    // 创建 ChannelManager，注册 Channel，然后启动
    let mut channel_manager = ChannelManager::new(bus);
    channel_manager.register(GatewayChannel::new(state_arc.clone()));
    channel_manager.register(DiscordChannel::new(state_arc.clone()));
    let channel_manager_handle = channel_manager.start();

    *state_arc.channel_manager_handle.write().await = Some(channel_manager_handle);

    // Start cron scheduler
    let state_for_cron = state_arc.clone();
    tokio::spawn(cron::run_cron_scheduler(state_for_cron));

    // Resume in-flight agent loops from checkpoints
    if let Ok(checkpoints) = db::list_all_checkpoints(&state_arc.db).await {
        if !checkpoints.is_empty() {
            info!("Found {} orphaned checkpoints, resuming", checkpoints.len());
            for cp in checkpoints {
                let mut metadata = std::collections::HashMap::new();
                metadata.insert("resume_messages".into(), cp.messages);
                metadata.insert("resume_iteration".into(), serde_json::json!(cp.iteration));
                let event = bus::InboundEvent {
                    channel: cp.channel,
                    sender_id: cp.user_id,
                    chat_id: cp.chat_id,
                    content: "[System] Resuming interrupted task...".to_string(),
                    session_id: cp.session_id,
                    timestamp: Some(chrono::Utc::now()),
                    media: vec![],
                    metadata,
                };
                state_arc.bus.publish_inbound(event).await;
            }
        }
    }

    // AppState is Clone (all fields are Arc), deref + clone for axum state
    let app_state = (*state_arc).clone();

    // Protected routes (require JWT)
    let protected = Router::new()
        // Device tokens
        .route("/api/device-tokens", axum::routing::post(auth::create_device_token).get(auth::list_device_tokens))
        .route("/api/device-tokens/{token}", axum::routing::delete(auth::revoke_device_token))
        // Discord config
        .route("/api/discord-config", axum::routing::post(auth::upsert_discord_config).get(auth::get_discord_config).delete(auth::delete_discord_config))
        // Sessions
        .route("/api/sessions", axum::routing::get(auth::list_sessions))
        .route("/api/sessions/{session_id}", axum::routing::delete(auth::delete_session))
        // LLM config (admin only)
        .route("/api/llm-config", axum::routing::get(auth::get_llm_config).put(auth::update_llm_config))
        // Embedding config (admin only)
        .route("/api/embedding-config", axum::routing::get(auth::get_embedding_config).put(auth::update_embedding_config))
        // Session messages
        .route("/api/sessions/{session_id}/messages", axum::routing::get(api::get_session_messages))
        // Devices
        .route("/api/devices", axum::routing::get(api::list_devices))
        .route("/api/devices/{device_name}/policy", axum::routing::get(auth::get_device_policy).patch(auth::update_device_policy))
        .route("/api/devices/{device_name}/mcp", axum::routing::get(auth::get_device_mcp).put(auth::update_device_mcp))
        // Memories
        .route("/api/memories", axum::routing::get(api::list_memories))
        // User profile
        .route("/api/user/profile", axum::routing::get(api::get_user_profile))
        // User soul & preferences
        .route("/api/user/soul", axum::routing::get(api::get_soul).patch(api::update_soul))
        .route("/api/user/preferences", axum::routing::get(api::get_preferences).patch(api::update_preferences))
        // User memory
        .route("/api/user/memory", axum::routing::get(api::get_memory).patch(api::update_memory))
        // Admin: default soul
        .route("/api/admin/default-soul", axum::routing::get(api::get_default_soul).put(api::set_default_soul))
        // Skills
        .route("/api/skills", axum::routing::get(auth::list_skills).post(auth::create_skill))
        .route("/api/skills/{name}", axum::routing::delete(auth::delete_skill))
        // Admin: all skills
        .route("/api/admin/skills", axum::routing::get(auth::admin_list_skills))
        // Cron jobs
        .route("/api/cron-jobs", axum::routing::get(auth::list_cron_jobs_api).post(auth::create_cron_job_api))
        .route("/api/cron-jobs/{job_id}", axum::routing::delete(auth::delete_cron_job_api).patch(auth::update_cron_job_api))
        // File upload/download
        .route("/api/files", axum::routing::post(api::upload_file))
        .route("/api/files/{file_id}", axum::routing::get(api::download_file))
        // Admin: server MCP config
        .route("/api/server-mcp", axum::routing::get(auth::get_server_mcp).put(auth::update_server_mcp))
        .layer(axum::middleware::from_fn_with_state(app_state.clone(), auth::jwt_middleware));

    let app = Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/api/auth/register", axum::routing::post(auth::register))
        .route("/api/auth/login", axum::routing::post(auth::login))
        .merge(protected)
        .fallback(|| async {
            nexus_common::error::ApiError::new(nexus_common::error::ErrorCode::NotFound, "endpoint not found").into_response()
        })
        .with_state(app_state);

    let addr = SocketAddr::from(([0, 0, 0, 0], config.server_port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind 0.0.0.0:{}: {e}", config.server_port));

    info!("Server listening on 0.0.0.0:{}", config.server_port);

    // Set up graceful shutdown on SIGINT (Ctrl+C) or SIGTERM
    let shutdown_signal = async {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler");
        #[cfg(unix)]
        let terminate = sigterm.recv();
        #[cfg(not(unix))]
        let terminate = std::future::pending::<Option<()>>();

        tokio::select! {
            _ = ctrl_c => info!("Received SIGINT (Ctrl+C), initiating graceful shutdown..."),
            _ = terminate => info!("Received SIGTERM, initiating graceful shutdown..."),
        }
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await
        .unwrap_or_else(|e| panic!("Axum server error: {e}"));

    info!("HTTP server stopped, cleaning up...");

    // Stop LiteLLM proxy
    litellm_mgr.stop().await;

    // Signal the bus to shut down so the dispatch loop exits
    state_arc.bus.shutdown();

    // Stop all channels (Discord bots, gateway connections)
    if let Some(handle) = state_arc.channel_manager_handle.write().await.take() {
        info!("Stopping channels...");
        // Give channels up to 10 seconds to shut down
        match tokio::time::timeout(
            std::time::Duration::from_secs(10),
            handle.stop_all(),
        )
        .await
        {
            Ok(()) => info!("All channels stopped"),
            Err(_) => error!("Channel shutdown timed out after 10s, forcing exit"),
        }
    }

    // Close DB pool gracefully
    info!("Closing database pool...");
    state_arc.db.close().await;

    info!("Shutdown complete");
}
