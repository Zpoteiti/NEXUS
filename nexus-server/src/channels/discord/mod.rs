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

/// Shared map of channel_id → CancellationToken for typing indicators
pub type TypingTokenMap = Arc<DashMap<String, CancellationToken>>;

struct ConnHandle {
    cancel: CancellationToken,
    _handle: JoinHandle<()>,
}

pub struct DiscordChannel {
    state: Arc<AppState>,
    channel_tokens: ChannelTokenMap,
    typing_tokens: TypingTokenMap,
    shutdown: CancellationToken,
}

impl DiscordChannel {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            channel_tokens: Arc::new(DashMap::new()),
            typing_tokens: Arc::new(DashMap::new()),
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
            self.typing_tokens.clone(),
            self.shutdown.clone(),
        )
        .await;
    }

    async fn stop(&self) {
        info!("DiscordChannel: stopping all connections");
        self.shutdown.cancel();
    }

    async fn send_progress(&self, chat_id: &str, content: &str) -> Result<(), nexus_common::error::NexusError> {
        use nexus_common::error::{ErrorCode, NexusError};
        let bot_token = self
            .channel_tokens
            .get(chat_id)
            .map(|v| v.value().clone())
            .ok_or_else(|| NexusError::new(ErrorCode::ChannelError, format!("no bot token mapped for channel_id {}", chat_id)))?;

        // Send progress message without cancelling typing indicator
        rest::send_message(&bot_token, chat_id, content).await
    }

    async fn send(&self, chat_id: &str, content: &str) -> Result<(), nexus_common::error::NexusError> {
        use nexus_common::error::{ErrorCode, NexusError};
        let bot_token = self
            .channel_tokens
            .get(chat_id)
            .map(|v| v.value().clone())
            .ok_or_else(|| NexusError::new(ErrorCode::ChannelError, format!("no bot token mapped for channel_id {}", chat_id)))?;

        let result = rest::send_message(&bot_token, chat_id, content).await;

        // Cancel typing indicator after sending reply
        if let Some((_, token)) = self.typing_tokens.remove(chat_id) {
            token.cancel();
        }

        result
    }

    async fn send_with_media(
        &self,
        chat_id: &str,
        content: &str,
        media: &[String],
    ) -> Result<(), nexus_common::error::NexusError> {
        use nexus_common::error::{ErrorCode, NexusError};
        let bot_token = self
            .channel_tokens
            .get(chat_id)
            .map(|v| v.value().clone())
            .ok_or_else(|| NexusError::new(ErrorCode::ChannelError, format!("no bot token mapped for channel_id {}", chat_id)))?;

        let result = if media.is_empty() {
            rest::send_message(&bot_token, chat_id, content).await
        } else {
            rest::send_message_with_files(&bot_token, chat_id, content, media).await
        };

        // Cancel typing indicator after sending reply
        if let Some((_, token)) = self.typing_tokens.remove(chat_id) {
            token.cancel();
        }

        result
    }
}

async fn run_connection_manager(
    state: Arc<AppState>,
    channel_tokens: ChannelTokenMap,
    typing_tokens: TypingTokenMap,
    shutdown: CancellationToken,
) {
    let mut connections: HashMap<String, ConnHandle> = HashMap::new();
    let poll_interval = Duration::from_secs(30);
    // Rate-limit IDENTIFY calls: track last identify time (Discord allows 1 per 5s)
    let last_identify = Arc::new(tokio::sync::Mutex::new(
        tokio::time::Instant::now() - Duration::from_secs(5),
    ));

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
            let tt_clone = typing_tokens.clone();
            let cancel_clone = cancel.clone();
            let last_id_clone = last_identify.clone();
            let user_id = config.user_id.clone();

            let handle = tokio::spawn(async move {
                // Stagger IDENTIFY calls across bots on startup (1 per second).
                // Discord allows ~1000 IDENTIFYs/24h per IP; 1/s is well within limits
                // while preventing thundering herd on server restart.
                {
                    let mut guard = last_id_clone.lock().await;
                    let elapsed = guard.elapsed();
                    if elapsed < Duration::from_secs(1) {
                        tokio::time::sleep(Duration::from_secs(1) - elapsed).await;
                    }
                    *guard = tokio::time::Instant::now();
                }

                gateway_conn::run(config, state_clone, ct_clone, tt_clone, cancel_clone).await;
            });

            connections.insert(
                user_id,
                ConnHandle {
                    cancel,
                    _handle: handle,
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
