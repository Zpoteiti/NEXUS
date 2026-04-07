pub mod memory;
pub mod send_file;
pub mod download_to_device;
pub mod message;
pub mod cron;
pub mod skills;
pub mod web_fetch;

use async_trait::async_trait;
use nexus_common::error::NexusError;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::state::AppState;

/// Result of executing a server-native tool.
pub struct ServerToolResult {
    pub output: String,
    /// Optional media file paths (e.g., from send_file).
    pub media: Vec<String>,
}

/// A tool that executes on the server, not on any client device.
/// Server-native tools do NOT have a device_name parameter (unless they
/// interact with a device, like send_file).
#[async_trait]
pub trait ServerTool: Send + Sync {
    fn name(&self) -> &str;
    fn schema(&self) -> Value;
    async fn execute(
        &self,
        state: &Arc<AppState>,
        user_id: &str,
        session_id: &str,
        arguments: Value,
        event_channel: &str,
        event_chat_id: &str,
    ) -> Result<ServerToolResult, NexusError>;
}

pub struct ServerToolRegistry {
    tools: HashMap<String, Box<dyn ServerTool>>,
}

impl ServerToolRegistry {
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    pub fn register(&mut self, tool: Box<dyn ServerTool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn ServerTool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn schemas(&self) -> Vec<Value> {
        self.tools.values().map(|t| t.schema()).collect()
    }
}
