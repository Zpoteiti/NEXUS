mod config;
mod connection;
mod env;
mod guardrails;
mod heartbeat;
mod mcp;
mod sandbox;
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

    // Initialize MCP servers
    let mcp_manager = Arc::new(Mutex::new(mcp::McpManager::new()));
    {
        let cfg = config.read().await;
        mcp_manager.lock().await.initialize(&cfg.mcp_servers).await;
    }

    // Build tool registry
    let mut registry = tools::ToolRegistry::new();
    tools::register_builtin_tools(&mut registry);
    let registry = Arc::new(registry);

    // Collect and send tool schemas (built-in + MCP)
    {
        let mut schemas = registry.schemas();
        schemas.extend(mcp_manager.lock().await.all_tool_schemas());
        let msg = ClientToServer::RegisterTools { schemas };
        let mut s = sink.lock().await;
        send_message(&mut s, &msg).await?;
        info!(
            "Registered {} built-in + {} MCP tools",
            registry.tool_count(),
            mcp_manager.lock().await.session_count()
        );
    }

    let hb = spawn_heartbeat(Arc::clone(&sink), Arc::clone(&missed_acks));
    let result = message_loop(
        &mut stream,
        &sink,
        &config,
        &missed_acks,
        &registry,
        &mcp_manager,
    )
    .await;
    hb.cancel();
    result
}

async fn message_loop(
    stream: &mut connection::WsStream,
    sink: &Arc<Mutex<WsSink>>,
    config: &Arc<RwLock<config::ClientConfig>>,
    missed_acks: &Arc<AtomicU32>,
    registry: &Arc<tools::ToolRegistry>,
    mcp_manager: &Arc<Mutex<mcp::McpManager>>,
) -> Result<(), String> {
    loop {
        let msg = recv_message(stream).await?;
        match msg {
            ServerToClient::HeartbeatAck => {
                ack_heartbeat(missed_acks);
            }
            ServerToClient::ExecuteToolRequest(req) => {
                let sink = Arc::clone(sink);
                let config = Arc::clone(config);
                let registry = Arc::clone(registry);
                let mcp_mgr = Arc::clone(mcp_manager);

                // Spawn tool execution in background so message loop continues
                tokio::spawn(async move {
                    let result =
                        if mcp::McpManager::is_mcp_tool(&req.tool_name) {
                            match mcp_mgr
                                .lock()
                                .await
                                .call_tool(&req.tool_name, req.arguments)
                                .await
                            {
                                Ok(out) => tools::ToolResult::success(out),
                                Err(e) => tools::ToolResult::error(e),
                            }
                        } else {
                            let cfg = config.read().await;
                            registry
                                .dispatch(
                                    &req.tool_name,
                                    req.arguments,
                                    &cfg,
                                )
                                .await
                        };

                    let msg = ClientToServer::ToolExecutionResult(
                        ToolExecutionResult {
                            request_id: req.request_id,
                            exit_code: result.exit_code,
                            output: result.output,
                        },
                    );
                    if let Err(e) =
                        send_message(&mut *sink.lock().await, &msg).await
                    {
                        warn!("send result failed: {e}");
                    }
                });
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
                    mcp_servers.clone(),
                    workspace_path,
                    shell_timeout,
                    ssrf_whitelist,
                );
                if mcp_changed
                    && let Some(new_servers) = mcp_servers
                {
                    let mut mgr = mcp_manager.lock().await;
                    mgr.apply_config(&new_servers).await;
                    // Re-register tools with updated MCP schemas
                    let mut schemas = registry.schemas();
                    schemas.extend(mgr.all_tool_schemas());
                    let msg = ClientToServer::RegisterTools { schemas };
                    let _ =
                        send_message(&mut *sink.lock().await, &msg).await;
                }
            }
            other => {
                warn!("Unexpected message: {other:?}");
            }
        }
    }
}
