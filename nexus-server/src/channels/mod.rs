//! Channel abstraction: each channel is an active WS client connecting to an external gateway.
//! ChannelManager spawns each channel's start() task and runs the outbound dispatch loop.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use nexus_common::error::NexusError;

use crate::bus::MessageBus;

pub mod discord;
pub mod gateway;

// ============================================================================
// Channel Trait - each platform channel (gateway/telegram/discord) must implement this trait
// ============================================================================

/// Active channel trait — each implementation is a self-managing WS client.
#[async_trait::async_trait]
pub trait Channel: Send + Sync {
    /// Channel name, e.g. "gateway", "discord".
    fn name(&self) -> &str;
    /// Long-running task: connect to external gateway, receive inbound events, publish to bus.
    /// Must implement auto-reconnect with exponential backoff internally.
    async fn start(&self);
    /// Stop the connection and clean up resources.
    async fn stop(&self);
    /// Send an outbound message to the gateway (called by ChannelManager dispatch loop).
    async fn send(&self, chat_id: &str, content: &str) -> Result<(), NexusError>;
    /// Send a progress message (e.g. tool call hints) without cancelling typing indicators.
    /// Default implementation falls back to `send`.
    async fn send_progress(&self, chat_id: &str, content: &str) -> Result<(), NexusError> {
        self.send(chat_id, content).await
    }
    /// Send an outbound message with media attachments.
    /// Default implementation ignores media and falls back to `send`.
    async fn send_with_media(
        &self,
        chat_id: &str,
        content: &str,
        media: &[String],
    ) -> Result<(), NexusError> {
        self.send(chat_id, content).await
    }
}

// ============================================================================
// ChannelManager - consumes OutboundEvents and routes them to the correct Channel
// ============================================================================

/// Handle returned by `ChannelManager::start()`, providing access to channels for shutdown.
pub struct ChannelManagerHandle {
    dispatch_handle: JoinHandle<()>,
    channels: Arc<HashMap<String, Arc<dyn Channel>>>,
}

impl ChannelManagerHandle {
    /// Stop all channels gracefully, then abort the dispatch loop.
    pub async fn stop_all(self) {
        for (name, channel) in self.channels.iter() {
            info!("ChannelManagerHandle: stopping channel \"{}\"", name);
            channel.stop().await;
        }
        self.dispatch_handle.abort();
        let _ = self.dispatch_handle.await;
        info!("ChannelManagerHandle: all channels stopped");
    }
}

/// ChannelManager — spawns each channel's start() task and runs the outbound dispatch loop.
pub struct ChannelManager {
    bus: Arc<MessageBus>,
    channels: HashMap<String, Arc<dyn Channel>>,
}

impl ChannelManager {
    pub fn new(bus: Arc<MessageBus>) -> Self {
        Self {
            bus,
            channels: HashMap::new(),
        }
    }

    /// Register a channel.
    pub fn register<C: Channel + 'static>(&mut self, channel: C) {
        let name = channel.name().to_string();
        info!("ChannelManager: registering channel \"{}\"", name);
        self.channels.insert(name, Arc::new(channel));
    }

    /// Spawn all channel start() tasks + outbound dispatch loop.
    /// Returns a ChannelManagerHandle that can be used to stop all channels gracefully.
    pub fn start(self) -> ChannelManagerHandle {
        let channels: Arc<HashMap<String, Arc<dyn Channel>>> = Arc::new(self.channels);
        let bus = self.bus;

        for (name, channel) in channels.iter() {
            let ch = channel.clone();
            let n = name.clone();
            info!("ChannelManager: starting channel \"{}\"", n);
            tokio::spawn(async move { ch.start().await });
        }

        let dispatch_channels = channels.clone();
        let dispatch_handle = tokio::spawn(async move {
            loop {
                let event = match bus.consume_outbound().await {
                    Some(e) => e,
                    None => {
                        info!("ChannelManager: bus closed, shutting down");
                        break;
                    }
                };

                let ch_name = event.channel.as_str();
                let is_progress = event.metadata.get("_progress")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                if let Some(channel) = dispatch_channels.get(ch_name) {
                    let result = if is_progress {
                        channel.send_progress(&event.chat_id, &event.content).await
                    } else if event.media.is_empty() {
                        channel.send(&event.chat_id, &event.content).await
                    } else {
                        channel
                            .send_with_media(&event.chat_id, &event.content, &event.media)
                            .await
                    };
                    if let Err(e) = result {
                        warn!("ChannelManager: send to \"{}\" failed: {}", ch_name, e);
                    }
                } else {
                    warn!(
                        "ChannelManager: no channel \"{}\" registered, dropping event (chat_id={})",
                        ch_name, event.chat_id
                    );
                }
            }
        });

        ChannelManagerHandle {
            dispatch_handle,
            channels,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::sync::RwLock;

    struct MockChannel {
        name: &'static str,
        last_sent: Arc<RwLock<Option<(String, String)>>>,
    }

    #[async_trait::async_trait]
    impl Channel for MockChannel {
        fn name(&self) -> &str {
            self.name
        }
        async fn start(&self) {
            tokio::time::sleep(tokio::time::Duration::from_secs(9999)).await;
        }
        async fn stop(&self) {}
        async fn send(&self, chat_id: &str, content: &str) -> Result<(), NexusError> {
            *self.last_sent.write().await = Some((chat_id.to_string(), content.to_string()));
            Ok(())
        }
    }

    #[tokio::test]
    async fn dispatch_outbound_to_registered_channel() {
        let bus = Arc::new(MessageBus::new());
        let last_sent: Arc<RwLock<Option<(String, String)>>> = Arc::new(RwLock::new(None));
        let channel = MockChannel {
            name: "mock",
            last_sent: last_sent.clone(),
        };

        let mut mgr = ChannelManager::new(bus.clone());
        mgr.register(channel);
        let _handle = mgr.start();

        // Give the dispatch loop time to start
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Publish an outbound event
        bus.publish_outbound(crate::bus::OutboundEvent {
            channel: "mock".to_string(),
            chat_id: "chat1".to_string(),
            content: "hello".to_string(),
            media: vec![],
            metadata: HashMap::new(),
        })
        .await;

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let sent = last_sent.read().await;
        assert_eq!(sent.as_ref().unwrap().0, "chat1");
        assert_eq!(sent.as_ref().unwrap().1, "hello");
    }
}
