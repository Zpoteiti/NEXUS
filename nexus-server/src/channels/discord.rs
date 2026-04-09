//! Discord per-user bot channel via serenity.
//! Each user configures their own Discord bot. Server spawns serenity clients.
//! Group policy: only respond when @mentioned or replied to.
//! Access control: owner + allowed_users list.

use crate::bus::{self, InboundEvent, OutboundEvent};
use crate::state::AppState;
use nexus_common::consts::CHANNEL_DISCORD;
use serenity::all::{
    ChannelId, Context, CreateMessage, EventHandler, GatewayIntents, Message as DiscordMessage,
    Ready,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

const DISCORD_MSG_LIMIT: usize = 2000;

/// Active Discord bots, keyed by user_id.
type BotRegistry = Arc<RwLock<HashMap<String, BotHandle>>>;

struct BotHandle {
    /// Map of chat_id → ChannelId for outbound delivery
    channels: Arc<RwLock<HashMap<String, ChannelId>>>,
    /// Serenity HTTP client for sending messages
    http: Arc<RwLock<Option<Arc<serenity::http::Http>>>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

static BOT_REGISTRY: std::sync::LazyLock<BotRegistry> =
    std::sync::LazyLock::new(|| Arc::new(RwLock::new(HashMap::new())));

/// Start a Discord bot for a user. Called when discord config is created/updated.
pub async fn start_bot(state: Arc<AppState>, user_id: String, bot_token: String) {
    stop_bot(&user_id).await;

    let channels: Arc<RwLock<HashMap<String, ChannelId>>> = Arc::new(RwLock::new(HashMap::new()));
    let http: Arc<RwLock<Option<Arc<serenity::http::Http>>>> = Arc::new(RwLock::new(None));

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

    BOT_REGISTRY.write().await.insert(
        user_id.clone(),
        BotHandle {
            channels: Arc::clone(&channels),
            http: Arc::clone(&http),
            shutdown_tx: Some(shutdown_tx),
        },
    );

    // Load discord config from DB for access control
    let config = crate::db::discord::get_config(&state.db, &user_id)
        .await
        .ok()
        .flatten();
    let owner_discord_id = config
        .as_ref()
        .and_then(|c| c.owner_discord_id.clone())
        .unwrap_or_default();
    let allowed_users: Vec<String> = config
        .as_ref()
        .map(|c| c.allowed_users.clone())
        .unwrap_or_default();

    let state_clone = Arc::clone(&state);
    let channels_clone = Arc::clone(&channels);
    let http_clone = Arc::clone(&http);
    let user_id_clone = user_id.clone();

    tokio::spawn(async move {
        info!("Discord bot starting for user {user_id_clone}");

        let handler = DiscordHandler {
            nexus_user_id: user_id_clone.clone(),
            owner_discord_id,
            allowed_users,
            state: state_clone,
            channels: channels_clone,
        };

        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT;

        let mut client = match serenity::Client::builder(&bot_token, intents)
            .event_handler(handler)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                error!("Discord bot build failed for {user_id_clone}: {e}");
                return;
            }
        };

        // Store HTTP client for outbound delivery
        *http_clone.write().await = Some(Arc::clone(&client.http));

        tokio::select! {
            result = client.start() => {
                if let Err(e) = result {
                    error!("Discord bot error for {user_id_clone}: {e}");
                }
            }
            _ = &mut shutdown_rx => {
                info!("Discord bot shutdown for {user_id_clone}");
                client.shard_manager.shutdown_all().await;
            }
        }
    });
}

/// Stop a Discord bot for a user.
pub async fn stop_bot(user_id: &str) {
    if let Some(mut handle) = BOT_REGISTRY.write().await.remove(user_id) {
        if let Some(tx) = handle.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Deliver an outbound event via Discord.
pub async fn deliver(state: &AppState, event: &OutboundEvent) {
    let registry = BOT_REGISTRY.read().await;
    let handle = match registry.get(&event.user_id) {
        Some(h) => h,
        None => {
            warn!("Discord: no bot for user {}", event.user_id);
            return;
        }
    };

    let http = handle.http.read().await;
    let Some(http) = http.as_ref() else {
        warn!("Discord: bot not ready for user {}", event.user_id);
        return;
    };
    let http = Arc::clone(http);

    // Parse chat_id to get ChannelId
    let channel_id = match event.chat_id.as_deref() {
        Some(chat_id) => {
            // chat_id format: "guild_id/channel_id" or "dm/user_id"
            let parts: Vec<&str> = chat_id.split('/').collect();
            if parts.len() == 2 {
                if let Ok(id) = parts[1].parse::<u64>() {
                    ChannelId::new(id)
                } else {
                    warn!("Discord: invalid channel_id in {chat_id}");
                    return;
                }
            } else {
                warn!("Discord: invalid chat_id format: {chat_id}");
                return;
            }
        }
        None => {
            // Try to find a cached channel
            let channels = handle.channels.read().await;
            match channels.values().next() {
                Some(id) => *id,
                None => {
                    warn!("Discord: no channel available for user {}", event.user_id);
                    return;
                }
            }
        }
    };

    // Split and send message
    let chunks = split_message(&event.content, DISCORD_MSG_LIMIT);
    for chunk in &chunks {
        let msg = CreateMessage::new().content(chunk);
        if let Err(e) = channel_id.send_message(&http, msg).await {
            error!("Discord send error: {e}");
        }
    }

    // Send media as file URLs
    for media_url in &event.media {
        let msg = CreateMessage::new().content(media_url);
        if let Err(e) = channel_id.send_message(&http, msg).await {
            error!("Discord media send error: {e}");
        }
    }
}

// -- Event Handler --

struct DiscordHandler {
    nexus_user_id: String,
    owner_discord_id: String,
    allowed_users: Vec<String>,
    state: Arc<AppState>,
    channels: Arc<RwLock<HashMap<String, ChannelId>>>,
}

#[serenity::async_trait]
impl EventHandler for DiscordHandler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        info!("Discord bot ready: {} ({})", ready.user.name, ready.user.id);
        // Store bot user ID in DB
        let _ = crate::db::discord::set_bot_user_id(
            &self.state.db,
            &self.nexus_user_id,
            &ready.user.id.to_string(),
        )
        .await;
    }

    async fn message(&self, ctx: Context, msg: DiscordMessage) {
        // Ignore bot messages
        if msg.author.bot {
            return;
        }

        let sender_id = msg.author.id.to_string();
        let sender_name = msg.author.name.clone();

        // Access control: owner or allowed_users
        let is_owner = sender_id == self.owner_discord_id;
        if !is_owner && !self.allowed_users.contains(&sender_id) {
            return;
        }

        // Group check: in guilds, only respond when @mentioned or replied to
        if msg.guild_id.is_some() {
            let bot_id = ctx.cache.current_user().id;
            let is_mentioned = msg.mentions.iter().any(|u| u.id == bot_id);
            let is_reply_to_bot = msg
                .referenced_message
                .as_ref()
                .map(|r| r.author.id == bot_id)
                .unwrap_or(false);

            if !is_mentioned && !is_reply_to_bot {
                return;
            }
        }

        // Build chat_id
        let chat_id = if let Some(guild_id) = msg.guild_id {
            format!("{}/{}", guild_id, msg.channel_id)
        } else {
            format!("dm/{}", msg.channel_id)
        };

        // Cache channel for outbound delivery
        self.channels
            .write()
            .await
            .insert(chat_id.clone(), msg.channel_id);

        // Strip bot mention from content
        let content = strip_mentions(&msg.content, &ctx).await;
        if content.trim().is_empty() {
            return;
        }

        let session_id = format!("discord:{}", msg.channel_id);

        let event = InboundEvent {
            session_id,
            user_id: self.nexus_user_id.clone(),
            content,
            channel: CHANNEL_DISCORD.to_string(),
            chat_id: Some(chat_id),
            sender_id: Some(sender_id.clone()),
            media: vec![],
            cron_job_id: None,
            identity: Some(crate::context::ChannelIdentity {
                sender_name: sender_name.clone(),
                sender_id,
                is_owner,
                owner_name: self.owner_discord_id.clone(),
                owner_id: self.owner_discord_id.clone(),
                channel_type: CHANNEL_DISCORD.to_string(),
            }),
            metadata: Default::default(),
        };

        if let Err(e) = bus::publish_inbound(&self.state, event).await {
            error!("Discord inbound error: {e}");
        }
    }
}

/// Strip bot mentions (<@BOT_ID> or <@!BOT_ID>) from message content.
async fn strip_mentions(content: &str, ctx: &Context) -> String {
    let bot_id = ctx.cache.current_user().id;
    content
        .replace(&format!("<@{bot_id}>"), "")
        .replace(&format!("<@!{bot_id}>"), "")
        .trim()
        .to_string()
}

/// Split a message into chunks that fit within the character limit.
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
        // Try to split at newline
        let split_at = remaining[..limit].rfind('\n').unwrap_or(limit);
        chunks.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..].trim_start();
    }
    chunks
}
