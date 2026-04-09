//! Telegram per-user bot channel via teloxide.
//! Each user configures their own Telegram bot. Long polling. Group @mention detection.

use crate::bus::{self, InboundEvent, OutboundEvent};
use crate::state::AppState;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

type BotRegistry = Arc<RwLock<HashMap<String, BotHandle>>>;

struct BotHandle {
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

static BOT_REGISTRY: std::sync::LazyLock<BotRegistry> =
    std::sync::LazyLock::new(|| Arc::new(RwLock::new(HashMap::new())));

/// Start a Telegram bot for a user.
pub async fn start_bot(state: Arc<AppState>, user_id: String, bot_token: String) {
    stop_bot(&user_id).await;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    BOT_REGISTRY
        .write()
        .await
        .insert(user_id.clone(), BotHandle { shutdown_tx });

    tokio::spawn(async move {
        info!("Telegram bot starting for user {user_id}");
        // TODO: Full teloxide client implementation
        // Long polling, @mention detection, access control
        let _ = shutdown_rx.await;
        info!("Telegram bot stopped for user {user_id}");
    });
}

/// Stop a Telegram bot for a user.
pub async fn stop_bot(user_id: &str) {
    if let Some(handle) = BOT_REGISTRY.write().await.remove(user_id) {
        let _ = handle.shutdown_tx.send(());
    }
}

/// Deliver an outbound event via Telegram.
pub async fn deliver(_state: &AppState, event: &OutboundEvent) {
    // TODO: Find the user's active bot, send to the target chat
    warn!(
        "Telegram deliver stub: {} chars to {:?}",
        event.content.len(),
        event.chat_id
    );
}
