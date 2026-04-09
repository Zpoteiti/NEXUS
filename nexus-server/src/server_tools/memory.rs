//! save_memory and edit_memory server tools.

use crate::state::AppState;
use nexus_common::consts::MEMORY_TEXT_MAX_CHARS;
use serde_json::Value;
use std::sync::Arc;

pub async fn save_memory(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let text = match args.get("text").and_then(Value::as_str) {
        Some(t) => t,
        None => return (1, "Missing required parameter: text".into()),
    };
    if text.len() > MEMORY_TEXT_MAX_CHARS {
        return (
            1,
            format!("Memory exceeds {MEMORY_TEXT_MAX_CHARS} character limit"),
        );
    }
    match crate::db::users::update_memory(&state.db, user_id, text).await {
        Ok(()) => (0, "Memory saved.".into()),
        Err(e) => (1, format!("Failed to save memory: {e}")),
    }
}

pub async fn edit_memory(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let op = match args.get("operation").and_then(Value::as_str) {
        Some(o) => o,
        None => return (1, "Missing required parameter: operation".into()),
    };
    let text = match args.get("text").and_then(Value::as_str) {
        Some(t) => t,
        None => return (1, "Missing required parameter: text".into()),
    };

    let user = match crate::db::users::find_by_id(&state.db, user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return (1, "User not found".into()),
        Err(e) => return (1, format!("DB error: {e}")),
    };

    let new_memory = match op {
        "append" => format!("{}{text}", user.memory_text),
        "prepend" => format!("{text}{}", user.memory_text),
        "replace" => text.to_string(),
        _ => {
            return (
                1,
                format!("Unknown operation: {op}. Use append, prepend, or replace."),
            );
        }
    };

    if new_memory.len() > MEMORY_TEXT_MAX_CHARS {
        return (
            1,
            format!(
                "Result would exceed {MEMORY_TEXT_MAX_CHARS} chars ({} chars). Trim content first.",
                new_memory.len()
            ),
        );
    }

    match crate::db::users::update_memory(&state.db, user_id, &new_memory).await {
        Ok(()) => (0, format!("Memory updated ({} chars).", new_memory.len())),
        Err(e) => (1, format!("Failed to update memory: {e}")),
    }
}
