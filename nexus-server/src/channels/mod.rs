//! Channel abstraction and outbound dispatch loop.
//! Each channel converts incoming messages to InboundEvents and delivers OutboundEvents.

pub mod discord;
pub mod gateway;
pub mod telegram;

use crate::bus::OutboundEvent;
use crate::state::AppState;
use nexus_common::consts::{CHANNEL_DISCORD, CHANNEL_GATEWAY};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Channel name for Telegram.
pub const CHANNEL_TELEGRAM: &str = "telegram";

/// Spawn the outbound dispatch loop. Routes OutboundEvents to the correct channel handler.
pub fn spawn_outbound_dispatch(state: Arc<AppState>, mut rx: mpsc::Receiver<OutboundEvent>) {
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event.channel.as_str() {
                CHANNEL_GATEWAY => {
                    gateway::deliver(&state, &event).await;
                }
                CHANNEL_DISCORD => {
                    discord::deliver(&state, &event).await;
                }
                CHANNEL_TELEGRAM => {
                    telegram::deliver(&state, &event).await;
                }
                other => {
                    warn!("Unknown outbound channel: {other}");
                }
            }
            debug!(
                "Outbound [{}]: {} chars to {:?}",
                event.channel,
                event.content.len(),
                event.chat_id
            );
        }
    });
}
