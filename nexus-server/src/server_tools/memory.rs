use async_trait::async_trait;
use nexus_common::error::{ErrorCode, NexusError};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::info;

use super::{ServerTool, ServerToolResult};
use crate::state::AppState;

/// Maximum memory size in characters (4K).
const MEMORY_MAX_CHARS: usize = 4096;

pub struct SaveMemoryTool;

#[async_trait]
impl ServerTool for SaveMemoryTool {
    fn name(&self) -> &str { "save_memory" }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "save_memory",
                "description": "Append a fact, preference, or important context to persistent memory. Memory persists across sessions. If memory is full (4K chars), use edit_memory to make room first.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "The text to append to memory."
                        }
                    },
                    "required": ["content"]
                }
            }
        })
    }

    async fn execute(
        &self,
        state: &Arc<AppState>,
        user_id: &str,
        _session_id: &str,
        arguments: Value,
        _event_channel: &str,
        _event_chat_id: &str,
    ) -> Result<ServerToolResult, NexusError> {
        let content = arguments.get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if content.is_empty() {
            return Err(NexusError::new(ErrorCode::ToolInvalidParams, "save_memory: content is empty"));
        }

        let current = crate::db::get_user_memory(&state.db, user_id)
            .await
            .map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("failed to read memory: {}", e)))?;

        let new_memory = if current.is_empty() {
            content.clone()
        } else {
            format!("{}\n{}", current, content)
        };

        if new_memory.len() > MEMORY_MAX_CHARS {
            return Err(NexusError::new(
                ErrorCode::ValidationFailed,
                format!(
                    "Memory would exceed 4K limit ({}/{} chars). Use edit_memory to remove or replace outdated entries first.",
                    new_memory.len(),
                    MEMORY_MAX_CHARS
                ),
            ));
        }

        crate::db::update_user_memory(&state.db, user_id, &new_memory)
            .await
            .map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("failed to save memory: {}", e)))?;

        info!("save_memory: appended {} chars for user {}", content.len(), user_id);

        Ok(ServerToolResult {
            output: format!("Memory saved. Total size: {}/{} chars.", new_memory.len(), MEMORY_MAX_CHARS),
            media: vec![],
        })
    }
}

pub struct EditMemoryTool;

#[async_trait]
impl ServerTool for EditMemoryTool {
    fn name(&self) -> &str { "edit_memory" }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "edit_memory",
                "description": "Find and replace text in persistent memory. Use to update outdated info or remove entries. Set new_string to empty string to delete.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "old_string": {
                            "type": "string",
                            "description": "The exact text to find in memory."
                        },
                        "new_string": {
                            "type": "string",
                            "description": "The replacement text. Use empty string to delete the matched text."
                        }
                    },
                    "required": ["old_string", "new_string"]
                }
            }
        })
    }

    async fn execute(
        &self,
        state: &Arc<AppState>,
        user_id: &str,
        _session_id: &str,
        arguments: Value,
        _event_channel: &str,
        _event_chat_id: &str,
    ) -> Result<ServerToolResult, NexusError> {
        let old_string = arguments.get("old_string")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let new_string = arguments.get("new_string")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if old_string.is_empty() {
            return Err(NexusError::new(ErrorCode::ToolInvalidParams, "edit_memory: old_string is empty"));
        }

        let current = crate::db::get_user_memory(&state.db, user_id)
            .await
            .map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("failed to read memory: {}", e)))?;

        // Validate unique match
        let match_count = current.matches(&old_string).count();
        if match_count == 0 {
            return Err(NexusError::new(
                ErrorCode::ValidationFailed,
                format!("edit_memory: old_string not found in memory. Current memory ({} chars):\n{}", current.len(), current),
            ));
        }
        if match_count > 1 {
            return Err(NexusError::new(
                ErrorCode::ValidationFailed,
                format!("edit_memory: old_string matches {} times. Provide a more specific string to match exactly once.", match_count),
            ));
        }

        let updated = current.replacen(&old_string, &new_string, 1);

        // Trim any resulting double-newlines from deletions
        let updated = updated.trim().to_string();

        if updated.len() > MEMORY_MAX_CHARS {
            return Err(NexusError::new(
                ErrorCode::ValidationFailed,
                format!("edit_memory: result would exceed 4K limit ({}/{} chars).", updated.len(), MEMORY_MAX_CHARS),
            ));
        }

        crate::db::update_user_memory(&state.db, user_id, &updated)
            .await
            .map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("failed to update memory: {}", e)))?;

        info!("edit_memory: replaced text for user {} ({} -> {} chars)", user_id, current.len(), updated.len());

        Ok(ServerToolResult {
            output: format!("Memory updated. Total size: {}/{} chars.", updated.len(), MEMORY_MAX_CHARS),
            media: vec![],
        })
    }
}
