//! Telegram per-user bot channel via teloxide.
//! Long polling. Group @mention detection. Access control via allowed_users.

use crate::bus::{self, InboundEvent, OutboundEvent};
use crate::state::AppState;
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::ChatKind;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

const TELEGRAM_MSG_LIMIT: usize = 4096;

type BotRegistry = Arc<RwLock<HashMap<String, BotHandle>>>;

struct BotHandle {
    /// teloxide bot instance for outbound delivery
    bot: Arc<RwLock<Option<Bot>>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

static BOT_REGISTRY: std::sync::LazyLock<BotRegistry> =
    std::sync::LazyLock::new(|| Arc::new(RwLock::new(HashMap::new())));

/// Start a Telegram bot for a user.
pub async fn start_bot(state: Arc<AppState>, user_id: String, bot_token: String) {
    stop_bot(&user_id).await;

    let bot_holder: Arc<RwLock<Option<Bot>>> = Arc::new(RwLock::new(None));
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

    BOT_REGISTRY.write().await.insert(
        user_id.clone(),
        BotHandle {
            bot: Arc::clone(&bot_holder),
            shutdown_tx: Some(shutdown_tx),
        },
    );

    // Load telegram config for access control
    let config = crate::db::telegram::get_config(&state.db, &user_id)
        .await
        .ok()
        .flatten();
    let owner_telegram_id = config
        .as_ref()
        .and_then(|c| c.owner_telegram_id.clone())
        .unwrap_or_default();
    let allowed_users: Vec<String> = config
        .as_ref()
        .map(|c| c.allowed_users.clone())
        .unwrap_or_default();
    let group_policy = config
        .as_ref()
        .map(|c| c.group_policy.clone())
        .unwrap_or_else(|| "mention".into());

    let state_clone = Arc::clone(&state);
    let bot_holder_clone = Arc::clone(&bot_holder);

    tokio::spawn(async move {
        info!("Telegram bot starting for user {user_id}");

        let bot = Bot::new(&bot_token);
        *bot_holder_clone.write().await = Some(bot.clone());

        // Get bot username for mention detection
        let bot_username = match bot.get_me().await {
            Ok(me) => me.username.clone().unwrap_or_default(),
            Err(e) => {
                error!("Telegram getMe failed for {user_id}: {e}");
                return;
            }
        };
        info!("Telegram bot ready: @{bot_username} for user {user_id}");

        let user_id_log = user_id.clone();
        let handler = Update::filter_message().endpoint(move |bot: Bot, msg: Message| {
            let state = Arc::clone(&state_clone);
            let nexus_user_id = user_id.clone();
            let owner_id = owner_telegram_id.clone();
            let allowed = allowed_users.clone();
            let policy = group_policy.clone();
            let username = bot_username.clone();

            async move {
                handle_message(
                    &state,
                    &bot,
                    &msg,
                    &nexus_user_id,
                    &owner_id,
                    &allowed,
                    &policy,
                    &username,
                )
                .await;
                respond(())
            }
        });

        let mut dispatcher = Dispatcher::builder(bot, handler)
            .enable_ctrlc_handler()
            .build();

        tokio::select! {
            _ = dispatcher.dispatch() => {
                info!("Telegram bot stopped for {}", &user_id_log);
            }
            _ = &mut shutdown_rx => {
                info!("Telegram bot shutdown requested");
                dispatcher.shutdown_token().shutdown().expect("shutdown");
            }
        }
    });
}

async fn handle_message(
    state: &Arc<AppState>,
    _bot: &Bot,
    msg: &Message,
    nexus_user_id: &str,
    owner_telegram_id: &str,
    allowed_users: &[String],
    group_policy: &str,
    bot_username: &str,
) {
    // Extract text content
    let text = match msg.text() {
        Some(t) => t.to_string(),
        None => return,
    };

    // Get sender info
    let sender = match msg.from.as_ref() {
        Some(u) => u,
        None => return,
    };
    let sender_id = sender.id.0.to_string();
    let sender_name = sender.first_name.clone();

    // Access control
    let is_owner = sender_id == owner_telegram_id;
    if !is_owner && !allowed_users.contains(&sender_id) {
        return;
    }

    // Group check: only respond when @mentioned (if policy is "mention")
    let is_group = matches!(msg.chat.kind, ChatKind::Public(_));
    if is_group && group_policy == "mention" {
        let mention_tag = format!("@{bot_username}");
        if !text.contains(&mention_tag) {
            // Also check if it's a reply to the bot
            let is_reply_to_bot = msg
                .reply_to_message()
                .and_then(|r| r.from.as_ref())
                .map(|u| u.is_bot)
                .unwrap_or(false);
            if !is_reply_to_bot {
                return;
            }
        }
    }

    // Strip @mention from content
    let content = text
        .replace(&format!("@{bot_username}"), "")
        .trim()
        .to_string();
    if content.is_empty() {
        return;
    }

    let chat_id = format!("tg:{}", msg.chat.id);
    let session_id = format!("telegram:{}", msg.chat.id);

    let event = InboundEvent {
        session_id,
        user_id: nexus_user_id.to_string(),
        content,
        channel: crate::channels::CHANNEL_TELEGRAM.to_string(),
        chat_id: Some(chat_id),
        sender_id: Some(sender_id.clone()),
        media: vec![],
        cron_job_id: None,
        identity: Some(crate::context::ChannelIdentity {
            sender_name: sender_name.clone(),
            sender_id,
            is_owner,
            owner_name: owner_telegram_id.to_string(),
            owner_id: owner_telegram_id.to_string(),
            channel_type: crate::channels::CHANNEL_TELEGRAM.to_string(),
        }),
        metadata: Default::default(),
    };

    if let Err(e) = bus::publish_inbound(state, event).await {
        error!("Telegram inbound error: {e}");
    }
}

/// Stop a Telegram bot for a user.
pub async fn stop_bot(user_id: &str) {
    if let Some(mut handle) = BOT_REGISTRY.write().await.remove(user_id) {
        if let Some(tx) = handle.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Deliver an outbound event via Telegram.
pub async fn deliver(_state: &AppState, event: &OutboundEvent) {
    let registry = BOT_REGISTRY.read().await;
    let handle = match registry.get(&event.user_id) {
        Some(h) => h,
        None => {
            warn!("Telegram: no bot for user {}", event.user_id);
            return;
        }
    };

    let bot_lock = handle.bot.read().await;
    let Some(bot) = bot_lock.as_ref() else {
        warn!("Telegram: bot not ready for user {}", event.user_id);
        return;
    };
    let bot = bot.clone();

    // Parse chat_id (format: "tg:{chat_id}")
    let chat_id_str = match event.chat_id.as_deref() {
        Some(id) => id.strip_prefix("tg:").unwrap_or(id),
        None => {
            warn!("Telegram: no chat_id for outbound");
            return;
        }
    };
    let chat_id: ChatId = match chat_id_str.parse::<i64>() {
        Ok(id) => ChatId(id),
        Err(_) => {
            warn!("Telegram: invalid chat_id: {chat_id_str}");
            return;
        }
    };

    // Split and send message
    let chunks = split_message(&event.content, TELEGRAM_MSG_LIMIT);
    for chunk in &chunks {
        if let Err(e) = bot.send_message(chat_id, chunk).await {
            error!("Telegram send error: {e}");
        }
    }

    // Send media URLs
    for media_url in &event.media {
        if let Err(e) = bot.send_message(chat_id, media_url).await {
            error!("Telegram media send error: {e}");
        }
    }
}

fn split_message(text: &str, limit: usize) -> Vec<String> {
    if text.len() <= limit {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.len() <= limit {
            chunks.push(remaining.to_string());
            break;
        }
        let split_at = remaining[..limit].rfind('\n').unwrap_or(limit);
        chunks.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start();
    }
    chunks
}
