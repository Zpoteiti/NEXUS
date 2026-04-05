use async_trait::async_trait;
use nexus_common::error::{ErrorCode, NexusError};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::info;

use super::{ServerTool, ServerToolResult};
use crate::state::AppState;

pub struct SaveMemoryTool;

#[async_trait]
impl ServerTool for SaveMemoryTool {
    fn name(&self) -> &str { "save_memory" }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "save_memory",
                "description": "Save an important fact, preference, or context to long-term memory for future reference across sessions.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "The memory content to save."
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
        session_id: &str,
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

        // Generate embedding if configured
        let embedding = {
            let emb_config = state.config.embedding.read().await.clone();
            if let Some(ref cfg) = emb_config {
                let emb = crate::context::embed_text_throttled(cfg, &content, &state.embedding_semaphore).await;
                if emb.is_empty() { None } else { Some(emb) }
            } else {
                None
            }
        };

        // Dedup: skip if a very similar memory already exists
        if let Some(ref emb) = embedding {
            match crate::db::find_similar_memory(&state.db, user_id, emb, 0.92).await {
                Ok(true) => {
                    info!("save_memory: skipping duplicate (cosine > 0.92)");
                    return Ok(ServerToolResult {
                        output: "Memory already exists (similar content found).".into(),
                        media: vec![],
                    });
                }
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!("save_memory: dedup check failed: {}, proceeding with save", e);
                }
            }
        }

        let history_entry = format!(
            "[{}] Agent saved: {}",
            chrono::Utc::now().format("%Y-%m-%d %H:%M"),
            &content[..content.len().min(80)]
        );

        crate::db::save_memory_chunk(
            &state.db, session_id, user_id,
            &history_entry, &content,
            embedding.as_deref(),
        ).await.map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("failed to save memory: {}", e)))?;

        Ok(ServerToolResult {
            output: "Memory saved successfully.".into(),
            media: vec![],
        })
    }
}
