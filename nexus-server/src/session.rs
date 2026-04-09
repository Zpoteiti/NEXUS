//! Per-session handle: inbox channel + mutex for DB write serialization.

use crate::bus::InboundEvent;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

pub struct SessionHandle {
    pub user_id: String,
    pub inbox_tx: mpsc::Sender<InboundEvent>,
    pub lock: Arc<Mutex<()>>,
}
