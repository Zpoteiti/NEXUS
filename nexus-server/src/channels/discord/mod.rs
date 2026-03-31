//! Discord Channel — multi-bot architecture.
//! DiscordChannel implements the Channel trait. Its start() spawns a DiscordConnectionManager
//! that reads bot configs from DB and manages one DiscordGatewayConn per enabled bot.

pub mod protocol;
pub mod rest;
pub mod gateway_conn;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::channels::Channel;
use crate::db;
use crate::state::AppState;

use gateway_conn::ChannelTokenMap;

struct ConnHandle {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}

pub struct DiscordChannel {
    state: Arc<AppState>,
    channel_tokens: ChannelTokenMap,
    shutdown: CancellationToken,
}

impl DiscordChannel {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            channel_tokens: Arc::new(DashMap::new()),
            shutdown: CancellationToken::new(),
        }
    }
}

#[async_trait::async_trait]
impl Channel for DiscordChannel {
    fn name(&self) -> &str {
        "discord"
    }

    async fn start(&self) {
        info!("DiscordChannel: starting connection manager");
        run_connection_manager(
            self.state.clone(),
            self.channel_tokens.clone(),
            self.shutdown.clone(),
        )
        .await;
    }

    async fn stop(&self) {
        info!("DiscordChannel: stopping all connections");
        self.shutdown.cancel();
    }

    async fn send(&self, chat_id: &str, content: &str) -> Result<(), String> {
        let bot_token = self
            .channel_tokens
            .get(chat_id)
            .map(|v| v.value().clone())
            .ok_or_else(|| format!("no bot token mapped for channel_id {}", chat_id))?;

        rest::send_message(&bot_token, chat_id, content).await
    }
}

async fn run_connection_manager(
    state: Arc<AppState>,
    channel_tokens: ChannelTokenMap,
    shutdown: CancellationToken,
) {
    let mut connections: HashMap<String, ConnHandle> = HashMap::new();
    let poll_interval = Duration::from_secs(30);
    let identify_semaphore = Arc::new(tokio::sync::Semaphore::new(1));

    loop {
        let configs = match db::get_all_discord_configs(&state.db).await {
            Ok(c) => c,
            Err(e) => {
                error!("DiscordConnectionManager: failed to load configs: {}", e);
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = tokio::time::sleep(poll_interval) => continue,
                }
            }
        };

        let active_user_ids: std::collections::HashSet<String> =
            configs.iter().map(|c| c.user_id.clone()).collect();

        connections.retain(|user_id, conn| {
            if !active_user_ids.contains(user_id) {
                info!("DiscordConnectionManager: stopping connection for user {}", user_id);
                conn.cancel.cancel();
                false
            } else {
                true
            }
        });

        for config in configs {
            if connections.contains_key(&config.user_id) {
                continue;
            }

            info!("DiscordConnectionManager: spawning connection for user {}", config.user_id);

            let cancel = CancellationToken::new();
            let state_clone = state.clone();
            let ct_clone = channel_tokens.clone();
            let cancel_clone = cancel.clone();
            let sem_clone = identify_semaphore.clone();
            let user_id = config.user_id.clone();

            let handle = tokio::spawn(async move {
                // Rate-limit IDENTIFY calls across bots (1 per 5 seconds)
                if let Ok(_permit) = sem_clone.acquire().await {
                    let sem_for_release = sem_clone.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        sem_for_release.add_permits(1);
                    });
                    // Don't drop permit, let the spawned task release it after 5s
                    std::mem::forget(_permit);
                }

                gateway_conn::run(config, state_clone, ct_clone, cancel_clone).await;
            });

            connections.insert(
                user_id,
                ConnHandle {
                    cancel,
                    handle,
                },
            );
        }

        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = tokio::time::sleep(poll_interval) => {}
        }
    }

    for (user_id, conn) in &connections {
        info!("DiscordConnectionManager: shutting down connection for user {}", user_id);
        conn.cancel.cancel();
    }
}
