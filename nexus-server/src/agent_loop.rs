//! Per-session ReAct agent loop. Full implementation in Task 8.

use crate::bus::InboundEvent;
use crate::state::AppState;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

pub async fn run_session(
    state: Arc<AppState>,
    session_id: String,
    user_id: String,
    mut inbox: mpsc::Receiver<InboundEvent>,
) {
    info!("Agent loop started for session {session_id}");

    while let Some(event) = inbox.recv().await {
        info!(
            "Session {session_id} received: {} chars from {}",
            event.content.len(),
            event.channel
        );
        // TODO: Full agent loop implementation in Task 8
        // For now, just acknowledge receipt
        let _ = state
            .outbound_tx
            .send(crate::bus::OutboundEvent {
                channel: event.channel.clone(),
                chat_id: event.chat_id.clone(),
                session_id: session_id.clone(),
                user_id: user_id.clone(),
                content: format!(
                    "[Agent loop stub] Received: {}",
                    &event.content[..event.content.len().min(100)]
                ),
                media: vec![],
                is_progress: false,
                metadata: Default::default(),
            })
            .await;
    }

    info!("Agent loop ended for session {session_id}");
    state.sessions.remove(&session_id);
}
