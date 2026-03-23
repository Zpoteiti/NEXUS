/// 职责边界：
/// 1. 负责程序的启动、环境变量读取 (.env)、数据库连接池 (PgPool) 的初始化。
/// 2. 调用 bus::init() 创建消息管道，初始化 AppState，启动 ChannelManager。
/// 3. 挂载 Axum 的路由（HTTP API 路由来自 api.rs，WebSocket 路由来自 ws.rs）。
/// 4. 绝对不要在这里写具体的 WebSocket 收发逻辑或 LLM 提示词逻辑。

// mod agent_loop;
// mod api;
// mod auth;
// mod bus;
// mod channels;
mod config;
// mod context;
mod db;
// mod memory;
// mod providers;
mod state;
// mod tools_registry;
mod ws;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use sqlx::PgPool;
use std::net::SocketAddr;
use tracing::info;

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

    let state = state::AppState::new(pool);

    let app = Router::new()
        .route("/ws", get(ws::ws_handler))
        .fallback(|| async { (StatusCode::NOT_IMPLEMENTED, "Not Implemented") })
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], config.server_port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind 0.0.0.0:{}: {e}", config.server_port));

    info!("Server listening on 0.0.0.0:{}", config.server_port);
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|e| panic!("Axum server error: {e}"));
}
