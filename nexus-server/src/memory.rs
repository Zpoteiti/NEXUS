//! Context compression and memory consolidation.
//! Reference: nanobot/agent/memory.py

use std::sync::LazyLock;

use dashmap::DashMap;
use serde_json::{json, Value};
use sqlx::PgPool;
use tracing::{error, info, warn};

use crate::config::{EmbeddingConfig, LlmConfig};
use crate::providers::{call_with_retry, ChatCompletionRequest};

/// Per-session consecutive failure counter for consolidation
static FAILURE_COUNTS: LazyLock<DashMap<String, usize>> = LazyLock::new(DashMap::new);
const MAX_FAILURES: usize = 3;
const MAX_CONSOLIDATION_ROUNDS: usize = 5;
const SAFETY_BUFFER: usize = 1024;

/// save_memory tool definition (built-in, not registered to client)
fn save_memory_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "save_memory",
            "description": "Save the memory consolidation result.",
            "parameters": {
                "type": "object",
                "properties": {
                    "history_entry": {
                        "type": "string",
                        "description": "Timestamped summary: [YYYY-MM-DD HH:MM] key events and decisions."
                    },
                    "memory_update": {
                        "type": "string",
                        "description": "Full updated long-term memory as markdown. Merge new info with existing."
                    }
                },
                "required": ["history_entry", "memory_update"]
            }
        }
    })
}

/// Estimate token count for messages (chars / 3, matching nanobot)
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

/// Entry point: called by agent_loop before each LLM call.
/// Checks if consolidation is needed and runs it in a loop until within budget.
pub async fn maybe_consolidate(
    session_id: &str,
    user_id: &str,
    db: &PgPool,
    llm_config: &LlmConfig,
    embedding_config: &tokio::sync::RwLock<Option<EmbeddingConfig>>,
) {
    let budget = llm_config
        .context_window
        .saturating_sub(llm_config.max_output_tokens)
        .saturating_sub(SAFETY_BUFFER);
    let target = budget / 2;

    for round in 0..MAX_CONSOLIDATION_ROUNDS {
        // Get unconsolidated messages to estimate size
        let messages = match crate::db::get_unconsolidated_messages(db, session_id).await {
            Ok(msgs) => msgs,
            Err(e) => {
                warn!("maybe_consolidate: failed to get messages: {}", e);
                return;
            }
        };

        if messages.is_empty() {
            return;
        }

        // Convert to Value for token estimation
        let message_values: Vec<Value> = messages
            .iter()
            .map(|m| {
                json!({
                    "role": m.role,
                    "content": m.content,
                    "tool_calls": m.tool_arguments.as_deref().unwrap_or("")
                })
            })
            .collect();

        let estimated = estimate_tokens(&message_values);
        if estimated < budget {
            return; // Within budget, no consolidation needed
        }

        info!(
            "consolidation round {}: estimated {} tokens, budget {}, target {}",
            round + 1,
            estimated,
            budget,
            target
        );

        let tokens_to_remove = estimated.saturating_sub(target);
        let boundary = pick_consolidation_boundary(&messages, &message_values, tokens_to_remove);

        if boundary == 0 {
            warn!("consolidation: no valid boundary found");
            return;
        }

        let chunk = &messages[..boundary];
        if chunk.is_empty() {
            return;
        }

        let emb_config = embedding_config.read().await.clone();
        let success =
            consolidate_chunk(session_id, user_id, db, llm_config, emb_config.as_ref(), chunk)
                .await;

        if !success {
            let mut count = FAILURE_COUNTS.entry(session_id.to_string()).or_insert(0);
            *count += 1;
            if *count >= MAX_FAILURES {
                warn!(
                    "consolidation: {} consecutive failures, falling back to raw archive",
                    *count
                );
                raw_archive(session_id, user_id, db, chunk).await;
                *count = 0;
            }
            return; // Don't continue loop on failure
        }

        // Reset failure count on success
        FAILURE_COUNTS.insert(session_id.to_string(), 0);
    }
}

/// Pick consolidation boundary: find furthest user-turn boundary
/// where preceding messages have enough tokens to remove.
fn pick_consolidation_boundary(
    messages: &[crate::db::StoredMessage],
    message_values: &[Value],
    tokens_to_remove: usize,
) -> usize {
    let mut accumulated = 0;
    let mut last_user_boundary = 0;

    for (i, msg) in messages.iter().enumerate() {
        let token_count = estimate_tokens(&message_values[i..=i]);
        accumulated += token_count;

        // Mark boundaries at user messages (safe cut points)
        if msg.role == "user" && accumulated >= tokens_to_remove {
            last_user_boundary = i;
            break;
        }
        if msg.role == "user" {
            last_user_boundary = i;
        }
    }

    // If we couldn't find a boundary with enough tokens, use the last user boundary we found
    if last_user_boundary == 0 && !messages.is_empty() {
        // Find any user message
        for (i, msg) in messages.iter().enumerate() {
            if msg.role == "user" {
                return i + 1; // Include this user message
            }
        }
    }

    if last_user_boundary > 0 {
        last_user_boundary
    } else {
        0
    }
}

/// Consolidate a chunk of messages via LLM
async fn consolidate_chunk(
    session_id: &str,
    user_id: &str,
    db: &PgPool,
    llm_config: &LlmConfig,
    embedding_config: Option<&EmbeddingConfig>,
    chunk: &[crate::db::StoredMessage],
) -> bool {
    // Format messages as text
    let formatted = chunk
        .iter()
        .map(|m| {
            let ts = m
                .created_at
                .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default();
            format!("[{}] {}: {}", ts, m.role.to_uppercase(), m.content)
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Get current memory text
    let current_memory = crate::db::get_latest_memory_text(db, session_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "(empty)".to_string());

    // Build consolidation prompt
    let prompt = format!(
        "Process this conversation and call the save_memory tool with your consolidation.\n\n\
        ## Current Long-term Memory\n{}\n\n\
        ## Conversation to Process\n{}",
        current_memory, formatted
    );

    let messages = vec![
        json!({"role": "system", "content": "You are a memory consolidation agent. Call the save_memory tool with your consolidation of the conversation."}),
        json!({"role": "user", "content": prompt}),
    ];

    let request = ChatCompletionRequest {
        messages,
        tools: vec![save_memory_tool()],
        model: llm_config.model.clone(),
    };

    let response = match call_with_retry(llm_config, request).await {
        Ok(resp) => resp,
        Err(e) => {
            error!("consolidation LLM call failed: {}", e);
            return false;
        }
    };

    // Parse save_memory tool call from response
    let choice = match response.choices.first() {
        Some(c) => c,
        None => {
            warn!("consolidation: no choices in response");
            return false;
        }
    };

    let tool_calls = match &choice.message.tool_calls {
        Some(tcs) if !tcs.is_empty() => tcs,
        _ => {
            warn!("consolidation: LLM did not call save_memory");
            return false;
        }
    };

    let tc = &tool_calls[0];
    if tc.function.name != "save_memory" {
        warn!("consolidation: unexpected tool call: {}", tc.function.name);
        return false;
    }

    let args: Value = match serde_json::from_str(&tc.function.arguments) {
        Ok(v) => v,
        Err(e) => {
            warn!("consolidation: failed to parse tool args: {}", e);
            return false;
        }
    };

    let history_entry = match args.get("history_entry").and_then(|v| v.as_str()) {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => {
            warn!("consolidation: missing or empty history_entry");
            return false;
        }
    };

    let memory_update = match args.get("memory_update").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        _ => {
            warn!("consolidation: missing memory_update");
            return false;
        }
    };

    // Generate embedding if config available
    let embedding = if let Some(emb_config) = embedding_config {
        let emb = crate::context::embed_text(emb_config, &memory_update).await;
        if emb.is_empty() {
            None
        } else {
            Some(emb)
        }
    } else {
        None
    };

    // Save to DB
    if let Err(e) = crate::db::save_memory_chunk(
        db,
        session_id,
        user_id,
        &history_entry,
        &memory_update,
        embedding.as_deref(),
    )
    .await
    {
        error!("consolidation: failed to save memory chunk: {}", e);
        return false;
    }

    // Mark messages as consolidated
    let message_ids: Vec<String> = chunk.iter().map(|m| m.message_id.clone()).collect();
    if let Err(e) = crate::db::mark_messages_consolidated(db, &message_ids).await {
        error!("consolidation: failed to mark messages: {}", e);
        // Don't return false -- chunk is saved, this is a minor issue
    }

    // Update session cursor
    if let Some(last) = message_ids.last() {
        let _ = crate::db::update_session_last_consolidated(db, session_id, last).await;
    }

    info!(
        "consolidation: saved {} messages, history_entry={}",
        chunk.len(),
        &history_entry[..history_entry.len().min(80)]
    );
    true
}

/// Raw archive fallback: dump messages without LLM summarization
async fn raw_archive(
    session_id: &str,
    user_id: &str,
    db: &PgPool,
    chunk: &[crate::db::StoredMessage],
) {
    let ts = chrono::Utc::now().format("%Y-%m-%d %H:%M").to_string();
    let history_entry = format!(
        "[{}] [RAW] {} messages archived without summarization",
        ts,
        chunk.len()
    );
    let memory_text = chunk
        .iter()
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n");

    let _ =
        crate::db::save_memory_chunk(db, session_id, user_id, &history_entry, &memory_text, None)
            .await;

    let message_ids: Vec<String> = chunk.iter().map(|m| m.message_id.clone()).collect();
    let _ = crate::db::mark_messages_consolidated(db, &message_ids).await;
    if let Some(last) = message_ids.last() {
        let _ = crate::db::update_session_last_consolidated(db, session_id, last).await;
    }

    info!(
        "consolidation: raw archived {} messages for session {}",
        chunk.len(),
        session_id
    );
}
