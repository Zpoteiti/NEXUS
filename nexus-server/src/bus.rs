use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};
use dashmap::DashMap;
use tracing::warn;

/// User message event (from any channel).
#[derive(Debug, Clone)]
pub struct InboundEvent {
    pub channel: String,                                      // "webui" | "discord" | "telegram"
    pub sender_id: String,                                    // user ID
    pub chat_id: String,                                      // conversation ID
    pub content: String,                                       // message content
    pub session_id: String,                                    // Nexus internal session identifier
    pub media: Vec<String>,                                    // media URL list
    pub metadata: HashMap<String, serde_json::Value>,         // runtime control fields
}

/// Agent response event (sent to any channel).
#[derive(Debug, Clone)]
pub struct OutboundEvent {
    pub channel: String,
    pub chat_id: String,
    pub content: String,
    pub media: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// MessageBus - in-process async message bus.
///
/// inbound: session-isolated routing. After `register_session`,
///          `publish_inbound` routes by session_id to the corresponding inbox.
/// outbound: global queue, single consumer (ChannelManager).
pub struct MessageBus {
    /// session_id -> inbox sender for that session (routing table)
    inbound_routes: Arc<DashMap<String, mpsc::Sender<InboundEvent>>>,
    /// outbound queue (single consumer: ChannelManager)
    outbound_tx: Arc<mpsc::Sender<OutboundEvent>>,
    outbound_rx: Arc<Mutex<mpsc::Receiver<OutboundEvent>>>,
    shutdown_tx: broadcast::Sender<()>,
}

impl MessageBus {
    /// Create a new MessageBus.
    pub fn new() -> Self {
        let (outbound_tx, outbound_rx) = mpsc::channel(4096);
        let (shutdown_tx, _) = broadcast::channel(1);
        Self {
            inbound_routes: Arc::new(DashMap::new()),
            outbound_tx: Arc::new(outbound_tx),
            outbound_rx: Arc::new(Mutex::new(outbound_rx)),
            shutdown_tx,
        }
    }

    /// Register a session's inbox sender into the routing table.
    /// Called by session_manager when creating a new session.
    pub fn register_session(&self, session_id: String, tx: mpsc::Sender<InboundEvent>) {
        self.inbound_routes.insert(session_id, tx);
    }

    /// Remove a session from the routing table.
    /// Called when the session ends.
    pub fn unregister_session(&self, session_id: &str) {
        self.inbound_routes.remove(session_id);
    }

    /// Publish an inbound event -- routes by session_id to the corresponding session inbox.
    pub async fn publish_inbound(&self, event: InboundEvent) {
        let session_id = event.session_id.clone();
        if let Some(tx) = self.inbound_routes.get(&session_id) {
            if tx.send(event).await.is_err() {
                warn!("bus: session {} inbox closed, removing from routes", session_id);
                drop(tx);
                self.inbound_routes.remove(&session_id);
            }
        } else {
            warn!("bus: no route for session_id={}, message dropped", session_id);
        }
    }

    /// Publish an outbound event to the queue.
    pub async fn publish_outbound(&self, event: OutboundEvent) {
        // If the ChannelManager consumer is closed, send will fail; ignore it.
        let _ = self.outbound_tx.send(event).await;
    }

    /// Consume an outbound event (blocks until an event arrives or shutdown is received).
    /// Returns None when a shutdown signal is received.
    pub async fn consume_outbound(&self) -> Option<OutboundEvent> {
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let mut rx = self.outbound_rx.lock().await;

        tokio::select! {
            event = rx.recv() => event,
            _ = shutdown_rx.recv() => None,
        }
    }

    /// Signal shutdown — unblocks consume_outbound so the dispatch loop exits.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }

}
