//! Message bus: InboundEvent routing to sessions, rate limiting, OutboundEvent dispatch.

use crate::session::SessionHandle;
use crate::state::AppState;
use nexus_common::consts::RATE_LIMIT_CACHE_TTL_SEC;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex, mpsc};
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct InboundEvent {
    pub session_id: String,
    pub user_id: String,
    pub content: String,
    pub channel: String,
    pub chat_id: Option<String>,
    pub sender_id: Option<String>,
    pub media: Vec<String>,
    pub cron_job_id: Option<String>,
    pub identity: Option<crate::context::ChannelIdentity>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct OutboundEvent {
    pub channel: String,
    pub chat_id: Option<String>,
    pub session_id: String,
    pub user_id: String,
    pub content: String,
    pub media: Vec<String>,
    pub is_progress: bool,
    pub metadata: HashMap<String, String>,
}

/// Publish an inbound event: rate limit check → find/create session → send to inbox.
/// If the session doesn't exist yet, spawns a new agent loop.
pub async fn publish_inbound(state: &Arc<AppState>, event: InboundEvent) -> Result<(), String> {
    // Rate limit check (cron events exempt)
    if event.cron_job_id.is_none() {
        check_rate_limit(state, &event.user_id).await?;
    }

    // Find or create session (atomic via DashMap entry API to avoid TOCTOU)
    if let Some(handle) = state.sessions.get(&event.session_id) {
        // Session exists — send to its inbox
        handle
            .inbox_tx
            .send(event)
            .await
            .map_err(|_| "Session inbox closed".to_string())?;
    } else {
        // Create new session — ensure DB row exists
        crate::db::sessions::create_session(&state.db, &event.session_id, &event.user_id)
            .await
            .map_err(|e| format!("Create session: {e}"))?;

        // Create inbox channel
        let (tx, rx) = mpsc::channel(100);

        let handle = SessionHandle {
            user_id: event.user_id.clone(),
            inbox_tx: tx.clone(),
            lock: Arc::new(Mutex::new(())),
        };

        // Use entry API to atomically insert only if still absent (prevents double-spawn)
        let session_id = event.session_id.clone();
        let inserted = state
            .sessions
            .entry(session_id.clone())
            .or_insert(handle)
            .inbox_tx
            .clone();

        // Send the event
        inserted
            .send(event.clone())
            .await
            .map_err(|_| "Send to new inbox failed".to_string())?;

        // Only spawn if we were the one who inserted (tx matches inserted)
        // Since DashMap::entry().or_insert() returns the existing value if present,
        // we check if our tx was the one inserted by comparing channel capacity
        // Actually, simpler: just try_send on our original tx — if it fails, another task won
        // The safest approach: spawn unconditionally but the agent loop handles duplicate gracefully
        // via the mpsc receiver — only one rx exists per session.
        // Since we only create rx once above, spawn only if this is a fresh session:
        if !state
            .sessions
            .get(&session_id)
            .unwrap()
            .inbox_tx
            .same_channel(&tx)
        {
            // Another task beat us — our rx will never receive, drop it
            drop(rx);
        } else {
            let state_clone = Arc::clone(state);
            let user_id = event.user_id.clone();
            tokio::spawn(async move {
                crate::agent_loop::run_session(state_clone, session_id, user_id, rx).await;
            });
            info!("New session spawned: {}", event.session_id);
        }
    }

    Ok(())
}

/// Check rate limit for a user. Returns Err if rate limited.
async fn check_rate_limit(state: &AppState, user_id: &str) -> Result<(), String> {
    let limit = *state.rate_limit_config.read().await;
    if limit == 0 {
        return Ok(()); // Unlimited
    }

    let now = Instant::now();
    let mut entry = state
        .rate_limiter
        .entry(user_id.to_string())
        .or_insert((limit, now));

    let (remaining, last_refill) = entry.value_mut();

    // Refill if stale
    let elapsed = now.duration_since(*last_refill).as_secs();
    if elapsed >= RATE_LIMIT_CACHE_TTL_SEC {
        *remaining = limit;
        *last_refill = now;
    }

    if *remaining == 0 {
        let wait = RATE_LIMIT_CACHE_TTL_SEC.saturating_sub(elapsed);
        return Err(format!(
            "Rate limit exceeded. Please wait {wait} seconds before sending another message."
        ));
    }

    *remaining -= 1;
    Ok(())
}

/// Refresh rate limit config from DB. Call periodically (every 60s).
pub async fn refresh_rate_limit_config(state: &Arc<AppState>) {
    match crate::db::system_config::get(&state.db, "rate_limit_per_min").await {
        Ok(Some(val)) => {
            if let Ok(limit) = val.parse::<u32>() {
                *state.rate_limit_config.write().await = limit;
            }
        }
        Ok(None) => {} // Not configured, keep current
        Err(e) => warn!("Failed to load rate limit: {e}"),
    }
}

/// Spawn periodic rate limit config refresh (every 60s).
pub fn spawn_rate_limit_refresh(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(RATE_LIMIT_CACHE_TTL_SEC));
        loop {
            interval.tick().await;
            refresh_rate_limit_config(&state).await;
        }
    });
}
