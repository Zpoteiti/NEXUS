mod browser;
mod gateway;
mod protocol;
mod state;

use std::net::SocketAddr;
use axum::{Router, routing::get, http::StatusCode};
use tracing::info;

use state::AppState;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let port: u16 = std::env::var("GATEWAY_PORT")
        .unwrap_or_else(|_| "9090".to_string())
        .parse()
        .unwrap_or(9090);

    let gateway_token = std::env::var("NEXUS_GATEWAY_TOKEN")
        .expect("NEXUS_GATEWAY_TOKEN env var required");

    let jwt_secret = std::env::var("JWT_SECRET")
        .expect("JWT_SECRET env var required");

    let state = AppState::new(gateway_token, jwt_secret);

    let app = Router::new()
        .route("/ws/browser", get(browser::browser_ws_handler))
        .route("/ws/nexus", get(gateway::nexus_ws_handler))
        .fallback(|| async { (StatusCode::NOT_FOUND, "nexus-gateway") })
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind 0.0.0.0:{}: {}", port, e));

    info!("nexus-gateway listening on 0.0.0.0:{}", port);
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|e| panic!("server error: {}", e));
}
