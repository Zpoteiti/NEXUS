mod config;
mod connection;
mod env;
mod heartbeat;
mod tools;

use connection::{recv_message, send_message, WsSink};
use heartbeat::{ack_heartbeat, spawn_heartbeat};
use nexus_common::consts::DEVICE_TOKEN_PREFIX;
use nexus_common::protocol::{ClientToServer, ServerToClient, ToolExecutionResult};
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let ws_url = std::env::var("NEXUS_SERVER_WS_URL")
        .or_else(|_| std::env::var("NEXUS_WS_URL"))
        .expect("NEXUS_SERVER_WS_URL or NEXUS_WS_URL must be set");

    let token = std::env::var("NEXUS_AUTH_TOKEN")
        .or_else(|_| std::env::var("NEXUS_DEVICE_TOKEN"))
        .expect("NEXUS_AUTH_TOKEN or NEXUS_DEVICE_TOKEN must be set");

    if !token.starts_with(DEVICE_TOKEN_PREFIX) {
        error!("Token must start with '{DEVICE_TOKEN_PREFIX}'");
        std::process::exit(1);
    }

    info!("NEXUS Client starting...");
    reconnect_loop(&ws_url, &token).await;
}

async fn reconnect_loop(ws_url: &str, token: &str) {
    let mut backoff = 1u64;
    loop {
        match run_session(ws_url, token).await {
            Ok(()) => {
                info!("Session ended cleanly");
                backoff = 1;
            }
            Err(e) => {
                warn!("Session error: {e}");
            }
        }
        info!("Reconnecting in {backoff}s...");
        tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(30);
    }
}

async fn run_session(ws_url: &str, token: &str) -> Result<(), String> {
    let (sink, mut stream, initial_config) =
        connection::connect_and_auth(ws_url, token).await?;
    let config = Arc::new(RwLock::new(initial_config));
    let sink = Arc::new(Mutex::new(sink));
    let missed_acks = Arc::new(AtomicU32::new(0));

    // TODO (Section 6): Initialize MCP servers
    // TODO (Section 3): Build tool registry and send RegisterTools

    let hb = spawn_heartbeat(Arc::clone(&sink), Arc::clone(&missed_acks));
    let result = message_loop(&mut stream, &sink, &config, &missed_acks).await;
    hb.cancel();
    result
}

async fn message_loop(
    stream: &mut connection::WsStream,
    sink: &Arc<Mutex<WsSink>>,
    config: &Arc<RwLock<config::ClientConfig>>,
    missed_acks: &Arc<AtomicU32>,
) -> Result<(), String> {
    loop {
        let msg = recv_message(stream).await?;
        match msg {
            ServerToClient::HeartbeatAck => {
                ack_heartbeat(missed_acks);
            }
            ServerToClient::ExecuteToolRequest(req) => {
                // TODO (Section 7): dispatch to tool handler
                warn!("Tool execution not yet implemented: {}", req.tool_name);
                let result = ClientToServer::ToolExecutionResult(ToolExecutionResult {
                    request_id: req.request_id,
                    exit_code: 1,
                    output: "Tool execution not yet implemented".into(),
                });
                let mut s = sink.lock().await;
                send_message(&mut s, &result).await?;
            }
            ServerToClient::ConfigUpdate {
                fs_policy,
                mcp_servers,
                workspace_path,
                shell_timeout,
                ssrf_whitelist,
            } => {
                let mut cfg = config.write().await;
                let mcp_changed = cfg.merge_update(
                    fs_policy,
                    mcp_servers,
                    workspace_path,
                    shell_timeout,
                    ssrf_whitelist,
                );
                if mcp_changed {
                    info!("MCP servers config changed — reinit needed");
                }
            }
            other => {
                warn!("Unexpected message: {other:?}");
            }
        }
    }
}
