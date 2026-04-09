//! 15-second heartbeat task. Tracks missed acks — 4 missed = force reconnect.

use crate::connection::{WsSink, send_message};
use nexus_common::consts::HEARTBEAT_INTERVAL_SEC;
use nexus_common::protocol::{ClientToServer, DeviceStatus};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

const MAX_MISSED_ACKS: u32 = 4;

pub struct HeartbeatHandle {
    task: tokio::task::JoinHandle<()>,
    cancel: CancellationToken,
}

impl HeartbeatHandle {
    pub fn cancel(self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

/// Spawn heartbeat task. Returns handle to cancel it on disconnect.
/// When 4 acks are missed, the dead_signal token is cancelled to force
/// the message loop to exit.
pub fn spawn_heartbeat(
    sink: Arc<Mutex<WsSink>>,
    missed_acks: Arc<AtomicU32>,
    dead_signal: CancellationToken,
) -> HeartbeatHandle {
    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let dead = dead_signal.clone();

    let task = tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(HEARTBEAT_INTERVAL_SEC));
        loop {
            tokio::select! {
                _ = cancel_clone.cancelled() => break,
                _ = interval.tick() => {}
            }
            let missed = missed_acks.fetch_add(1, Ordering::SeqCst);
            if missed >= MAX_MISSED_ACKS {
                warn!("Missed {missed} heartbeat acks — connection dead");
                dead.cancel();
                break;
            }
            let msg = ClientToServer::Heartbeat {
                status: DeviceStatus::Online,
            };
            let mut sink = sink.lock().await;
            if let Err(e) = send_message(&mut sink, &msg).await {
                warn!("Heartbeat send failed: {e}");
                dead.cancel();
                break;
            }
            debug!("Heartbeat sent (missed={missed})");
        }
    });
    HeartbeatHandle { task, cancel }
}

/// Call this when HeartbeatAck is received to reset the missed counter.
pub fn ack_heartbeat(missed_acks: &AtomicU32) {
    missed_acks.store(0, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ack_resets_counter() {
        let counter = AtomicU32::new(3);
        ack_heartbeat(&counter);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }
}
