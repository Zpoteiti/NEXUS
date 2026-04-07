//! Context compression — compress history when context window runs low.
//!
//! New algorithm: when remaining context space drops below 16K tokens,
//! compress all messages between the system prompt and the latest user turn
//! into a single assistant summary message.

use serde_json::{json, Value};
use sqlx::PgPool;
use tracing::{info, warn};

use crate::config::LlmConfig;
use crate::providers::{call_with_retry, ChatCompletionRequest};

/// Minimum remaining tokens before compression triggers.
const COMPRESSION_THRESHOLD: usize = 16384;

/// Estimate token count for messages (chars / 3, matching nanobot heuristic).
pub fn estimate_tokens(messages: &[Value]) -> usize {
    messages
        .iter()
        .map(|m| {
            let content_len = m
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| s.len())
                .unwrap_or(0);
            let tool_args_len = m
                .get("tool_calls")
                .map(|v| v.to_string().len())
                .unwrap_or(0);
            content_len + tool_args_len
        })
        .sum::<usize>()
        / 3
}

/// Entry point: called by agent_loop after building messages but before calling LLM.
///
/// Returns `Some(compressed_messages)` if compression was performed,
/// `None` if no compression was needed.
pub async fn maybe_consolidate(
    session_id: &str,
    _user_id: &str,
    db: &PgPool,
    llm_config: &LlmConfig,
    current_messages: &[Value],
    context_window: usize,
) -> Option<Vec<Value>> {
    let total_tokens = estimate_tokens(current_messages);
    let remaining = context_window.saturating_sub(total_tokens);

    if remaining >= COMPRESSION_THRESHOLD {
        return None; // Plenty of room, no compression needed
    }

    info!(
        "consolidation: remaining {} tokens < {} threshold, compressing (session={})",
        remaining, COMPRESSION_THRESHOLD, session_id
    );

    // Find the latest user turn boundary (last message with role=user)
    let latest_user_idx = find_latest_user_turn(current_messages);
    if latest_user_idx <= 1 {
        warn!("consolidation: nothing to compress (only system + latest turn)");
        return None;
    }

    // Messages to compress: everything between system prompt and latest user turn
    let to_compress = &current_messages[1..latest_user_idx];
    if to_compress.is_empty() {
        return None;
    }

    info!(
        "consolidation: compressing {} messages (indices 1..{})",
        to_compress.len(),
        latest_user_idx
    );

    // Call LLM to compress
    let compressed = compress_messages(llm_config, to_compress).await?;

    // Mark old messages as compressed in DB
    if !to_compress.is_empty() {
        let _ = mark_compressed_in_db(db, session_id).await;
    }

    // Build new message list: system + compressed summary + latest turn onwards
    let mut new_messages = vec![current_messages[0].clone()];
    new_messages.push(json!({
        "role": "assistant",
        "content": compressed
    }));
    new_messages.extend_from_slice(&current_messages[latest_user_idx..]);

    let new_tokens = estimate_tokens(&new_messages);
    info!(
        "consolidation: compressed {} -> {} tokens ({} messages -> {})",
        total_tokens,
        new_tokens,
        current_messages.len(),
        new_messages.len()
    );

    Some(new_messages)
}

/// Find the index of the latest user message in the array.
/// Returns 0 if no user message found.
fn find_latest_user_turn(messages: &[Value]) -> usize {
    for i in (0..messages.len()).rev() {
        let role = messages[i]
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if role == "user" {
            return i;
        }
    }
    0
}

/// Compress a slice of messages into a summary using the LLM.
async fn compress_messages(
    llm_config: &LlmConfig,
    messages: &[Value],
) -> Option<String> {
    let system_prompt = "You are a conversation compressor. Faithfully summarize the following conversation history. \
        Preserve all important facts, decisions, tool results, file paths, and error messages. \
        Be concise but complete. Do not lose any actionable information. \
        Output the summary as a clear, structured text.";

    let content = messages
        .iter()
        .map(|m| {
            let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("?");
            let text = m
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let tool_calls = m
                .get("tool_calls")
                .map(|tc| format!(" [tool_calls: {}]", tc))
                .unwrap_or_default();
            format!("{}: {}{}", role, text, tool_calls)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let compress_messages = vec![
        json!({"role": "system", "content": system_prompt}),
        json!({"role": "user", "content": content}),
    ];

    let request = ChatCompletionRequest {
        messages: compress_messages,
        tools: vec![],
        model: llm_config.model.clone(),
        max_tokens: Some(12288),
    };

    match call_with_retry(llm_config, request).await {
        Ok(response) => {
            let summary = response
                .choices
                .first()?
                .message
                .content
                .clone()?;
            if summary.trim().is_empty() {
                warn!("consolidation: LLM returned empty summary");
                None
            } else {
                Some(format!("[Compressed conversation summary]\n{}", summary))
            }
        }
        Err(e) => {
            warn!("consolidation: LLM compression call failed: {}", e);
            None
        }
    }
}

/// Mark messages in the DB as compressed.
/// This is best-effort — we mark everything that's currently unconsolidated
/// and not already compressed for this session.
async fn mark_compressed_in_db(db: &PgPool, session_id: &str) -> Option<()> {
    let now = chrono::Utc::now();
    match crate::db::mark_messages_compressed(db, session_id, now).await {
        Ok(count) => {
            info!("consolidation: marked {} messages as compressed", count);
            Some(())
        }
        Err(e) => {
            warn!("consolidation: failed to mark messages compressed: {}", e);
            None
        }
    }
}
