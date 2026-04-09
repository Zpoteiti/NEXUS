//! Context compression via LLM summarization.
//! Triggered when context_window - total_tokens < 16K.

use crate::config::LlmConfig;
use crate::db::messages::Message;
use crate::providers::openai::{self, ChatMessage};
use crate::state::AppState;
use tracing::{info, warn};

/// Compress conversation history by summarizing via LLM.
/// Messages between system prompt and latest user turn get compressed.
pub async fn compress(
    state: &AppState,
    session_id: &str,
    history: &[Message],
    llm_config: &LlmConfig,
) {
    if history.len() < 3 {
        return; // Nothing meaningful to compress
    }

    // Find messages to compress: everything except the last user message and
    // any tool calls/results after it (current turn)
    let mut compress_end = history.len();
    for i in (0..history.len()).rev() {
        if history[i].role == nexus_common::consts::ROLE_USER {
            compress_end = i;
            break;
        }
    }

    if compress_end == 0 {
        return; // Nothing to compress
    }

    let to_compress = &history[..compress_end];
    if to_compress.is_empty() {
        return;
    }

    // Build summary prompt
    let mut summary_messages = vec![ChatMessage::system(
        "Summarize the following conversation concisely, preserving key decisions, \
         facts, and context. This summary will replace the original messages in the \
         agent's context window."
            .to_string(),
    )];

    let mut conversation = String::new();
    for msg in to_compress {
        let role = &msg.role;
        let content = &msg.content;
        if !content.is_empty() {
            conversation += &format!("{role}: {content}\n\n");
        }
        if let Some(ref tool_name) = msg.tool_name {
            conversation += &format!("assistant [tool_call: {tool_name}]\n\n");
        }
    }
    summary_messages.push(ChatMessage::user(conversation));

    // Call LLM for summary (reuse shared HTTP client)
    let summary =
        match openai::call_llm(&state.http_client, llm_config, summary_messages, None).await {
            Ok(openai::LlmResponse::Text(text)) => text,
            Ok(openai::LlmResponse::ToolCalls(_)) => {
                warn!("Compression LLM returned tool calls instead of summary");
                return;
            }
            Err(e) => {
                warn!("Compression failed: {e}");
                return;
            }
        };

    // Mark compressed messages in DB
    let ids: Vec<String> = to_compress.iter().map(|m| m.message_id.clone()).collect();
    if let Err(e) = crate::db::messages::mark_compressed(&state.db, &ids).await {
        warn!("Failed to mark compressed: {e}");
        return;
    }

    // Insert summary as new assistant message
    let summary_id = uuid::Uuid::new_v4().to_string();
    if let Err(e) = crate::db::messages::insert(
        &state.db,
        &summary_id,
        session_id,
        "assistant",
        &format!("[Conversation summary]\n{summary}"),
        None,
        None,
        None,
    )
    .await
    {
        warn!("Failed to save summary: {e}");
        return;
    }

    info!(
        "Compressed {} messages into summary for session {session_id}",
        ids.len()
    );
}
