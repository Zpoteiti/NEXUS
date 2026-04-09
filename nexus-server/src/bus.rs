//! Message bus types. Full routing implementation in M2b.

use std::collections::HashMap;

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
