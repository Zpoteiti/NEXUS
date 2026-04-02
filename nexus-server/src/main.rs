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
mod db;
mod memory;
mod providers;
mod session;
mod state;
mod tools_registry;
mod ws;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;

use bus::MessageBus;
use channels::ChannelManager;
use channels::discord::DiscordChannel;
use channels::gateway::GatewayChannel;
use session::SessionManager;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config = config::load_config();

    let pool = PgPool::connect(&config.database_url)
        .await
        .unwrap_or_else(|e| panic!("Failed to connect PostgreSQL: {e}"));

    db::init_db(&pool)
        .await
        .unwrap_or_else(|e| panic!("Failed to initialize database: {e}"));

    // Load persisted configs from DB
    if let Ok(Some(llm_json)) = db::get_system_config(&pool, "llm_config").await {
        if let Ok(llm) = serde_json::from_str::<config::LlmConfig>(&llm_json) {
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
    let state = state::AppState::new(pool, config.clone(), bus.clone(), session_manager);
    let state_arc = Arc::new(state);

    // 创建 ChannelManager，注册 Channel，然后启动
    let mut channel_manager = ChannelManager::new(bus);
    channel_manager.register(GatewayChannel::new(state_arc.clone()));
    channel_manager.register(DiscordChannel::new(state_arc.clone()));
    let channel_manager_handle = channel_manager.start();

    *state_arc.channel_manager_handle.write().await = Some(channel_manager_handle);

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
        // Memories
        .route("/api/memories", axum::routing::get(api::list_memories))
        // User soul & preferences
        .route("/api/user/soul", axum::routing::get(api::get_soul).patch(api::update_soul))
        .route("/api/user/preferences", axum::routing::get(api::get_preferences).patch(api::update_preferences))
        // Admin: default soul
        .route("/api/admin/default-soul", axum::routing::get(api::get_default_soul).put(api::set_default_soul))
        .layer(axum::middleware::from_fn_with_state(app_state.clone(), auth::jwt_middleware));

    let app = Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/api/auth/register", axum::routing::post(auth::register))
        .route("/api/auth/login", axum::routing::post(auth::login))
        .merge(protected)
        .fallback(|| async { (StatusCode::NOT_IMPLEMENTED, "Not Implemented") })
        .with_state(app_state);

    let addr = SocketAddr::from(([0, 0, 0, 0], config.server_port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind 0.0.0.0:{}: {e}", config.server_port));

    info!("Server listening on 0.0.0.0:{}", config.server_port);
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|e| panic!("Axum server error: {e}"));
}
