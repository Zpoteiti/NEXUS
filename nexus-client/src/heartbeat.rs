//! 15-second heartbeat task. Tracks missed acks — 4 missed = force reconnect.

use crate::connection::{send_message, WsSink};
use nexus_common::consts::HEARTBEAT_INTERVAL_SEC;
use nexus_common::protocol::{ClientToServer, DeviceStatus};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, warn};

const MAX_MISSED_ACKS: u32 = 4;

pub struct HeartbeatHandle {
    task: tokio::task::JoinHandle<()>,
}

impl HeartbeatHandle {
    pub fn cancel(self) {
        self.task.abort();
    }
}

/// Spawn heartbeat task. Returns handle to cancel it on disconnect.
pub fn spawn_heartbeat(
    sink: Arc<Mutex<WsSink>>,
    missed_acks: Arc<AtomicU32>,
) -> HeartbeatHandle {
    let task = tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(HEARTBEAT_INTERVAL_SEC));
        loop {
            interval.tick().await;
            let missed = missed_acks.fetch_add(1, Ordering::SeqCst);
            if missed >= MAX_MISSED_ACKS {
                warn!("Missed {missed} heartbeat acks — connection dead");
                break;
            }
            let msg = ClientToServer::Heartbeat {
                status: DeviceStatus::Online,
            };
            let mut sink = sink.lock().await;
            if let Err(e) = send_message(&mut sink, &msg).await {
                warn!("Heartbeat send failed: {e}");
                break;
            }
            debug!("Heartbeat sent (missed={missed})");
        }
    });
    HeartbeatHandle { task }
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
