use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use crate::bus::InboundEvent;
use crate::state::AppState;

/// Handle for each session.
pub struct SessionHandle {
    pub lock: Arc<tokio::sync::Mutex<()>>,    // prevent concurrent DB writes
}

pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<String, SessionHandle>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get or create a session.
    /// - New session: returns (true, Some((inbox_tx, inbox_rx)))
    ///   - tx is registered to the Bus by the caller
    ///   - rx is passed to the spawned agent_loop
    /// - Existing session: returns (false, None)
    pub async fn get_or_create_session(&self, session_id: &str) -> (bool, Option<(mpsc::Sender<InboundEvent>, mpsc::Receiver<InboundEvent>)>) {
        let mut sessions = self.sessions.write().await;

        if sessions.contains_key(session_id) {
            return (false, None);
        }

        // Create new session
        let (inbox_tx, inbox_rx) = mpsc::channel(256);
        let handle = SessionHandle {
            lock: Arc::new(tokio::sync::Mutex::new(())),
        };
        sessions.insert(session_id.to_string(), handle);

        (true, Some((inbox_tx, inbox_rx)))
    }

    /// Get the session lock (for DB write operations).
    pub async fn get_session_lock(&self, session_id: &str) -> Option<Arc<tokio::sync::Mutex<()>>> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).map(|h| h.lock.clone())
    }

    /// Remove a session.
    pub async fn remove_session(&self, session_id: &str) {
        let mut sessions = self.sessions.write().await;
        sessions.remove(session_id);
    }

}

/// Ensure a session exists and publish the inbound event.
/// Channels just construct an InboundEvent and call this function -- session creation logic is centralized here.
pub async fn ensure_session_and_publish(
    state: &Arc<AppState>,
    event: InboundEvent,
) {
    let session_id = event.session_id.clone();
    let (is_new, channels) = state.session_manager.get_or_create_session(&session_id).await;
    if is_new {
        if let Some((inbox_tx, inbox_rx)) = channels {
            state.bus.register_session(session_id.clone(), inbox_tx);
            let state_clone = state.clone();
            let sid = session_id.clone();
            tokio::spawn(async move {
                crate::agent_loop::run_session(sid, inbox_rx, state_clone).await;
            });
        }
    }
    state.bus.publish_inbound(event).await;
}
