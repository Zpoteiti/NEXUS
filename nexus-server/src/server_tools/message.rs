use async_trait::async_trait;
use nexus_common::error::{ErrorCode, NexusError};
use serde_json::{json, Value};
use std::sync::Arc;

use super::{ServerTool, ServerToolResult};
use crate::bus::OutboundEvent;
use crate::state::AppState;

pub struct MessageTool;

#[async_trait]
impl ServerTool for MessageTool {
    fn name(&self) -> &str { "message" }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "message",
                "description": "Send a message to a specific channel/chat proactively. Use this to deliver results, reminders, or notifications to the user.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "channel": {
                            "type": "string",
                            "description": "The channel type (e.g., 'discord')."
                        },
                        "chat_id": {
                            "type": "string",
                            "description": "The target chat/channel ID."
                        },
                        "content": {
                            "type": "string",
                            "description": "The message content to send."
                        }
                    },
                    "required": ["channel", "chat_id", "content"]
                }
            }
        })
    }

    async fn execute(
        &self,
        state: &Arc<AppState>,
        _user_id: &str,
        _session_id: &str,
        arguments: Value,
        event_channel: &str,
        event_chat_id: &str,
    ) -> Result<ServerToolResult, NexusError> {
        // Default to current session's channel/chat_id for safety
        let channel = arguments.get("channel").and_then(|v| v.as_str())
            .unwrap_or(event_channel);
        let chat_id = arguments.get("chat_id").and_then(|v| v.as_str())
            .unwrap_or(event_chat_id);
        let content = arguments.get("content").and_then(|v| v.as_str())
            .ok_or_else(|| NexusError::new(ErrorCode::ToolInvalidParams, "missing content"))?;

        let event = OutboundEvent {
            channel: channel.to_string(),
            chat_id: chat_id.to_string(),
            content: content.to_string(),
            media: vec![],
            metadata: Default::default(),
        };

        state.bus.publish_outbound(event).await;

        Ok(ServerToolResult {
            output: format!("Message sent to {}:{}", channel, chat_id),
            media: vec![],
        })
    }
}
