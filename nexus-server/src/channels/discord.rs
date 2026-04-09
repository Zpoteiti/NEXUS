//! Discord per-user bot channel.
//! Each user configures their own Discord bot. Server spawns serenity clients.

use crate::bus::{self, InboundEvent, OutboundEvent};
use crate::context::ChannelIdentity;
use crate::state::AppState;
use nexus_common::consts::CHANNEL_DISCORD;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// Active Discord bots, keyed by user_id.
type BotRegistry = Arc<RwLock<HashMap<String, BotHandle>>>;

struct BotHandle {
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

/// Global bot registry — initialized once, bots added/removed dynamically.
static BOT_REGISTRY: std::sync::LazyLock<BotRegistry> =
    std::sync::LazyLock::new(|| Arc::new(RwLock::new(HashMap::new())));

/// Start a Discord bot for a user. Called when discord config is created/updated.
pub async fn start_bot(state: Arc<AppState>, user_id: String, bot_token: String) {
    // Stop existing bot if any
    stop_bot(&user_id).await;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    BOT_REGISTRY
        .write()
        .await
        .insert(user_id.clone(), BotHandle { shutdown_tx });

    let state_clone = state;
    tokio::spawn(async move {
        info!("Discord bot starting for user {user_id}");
        // TODO: Full serenity client implementation
        // For now, log and wait for shutdown
        let _ = shutdown_rx.await;
        info!("Discord bot stopped for user {user_id}");
    });
}

/// Stop a Discord bot for a user.
pub async fn stop_bot(user_id: &str) {
    if let Some(handle) = BOT_REGISTRY.write().await.remove(user_id) {
        let _ = handle.shutdown_tx.send(());
    }
}

/// Deliver an outbound event via Discord.
pub async fn deliver(_state: &AppState, event: &OutboundEvent) {
    // TODO: Find the user's active bot, send to the target channel/DM
    warn!(
        "Discord deliver stub: {} chars to {:?}",
        event.content.len(),
        event.chat_id
    );
}
