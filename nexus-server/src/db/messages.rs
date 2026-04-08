use serde::Serialize;
use sqlx::PgPool;
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct MessageInfo {
    pub message_id: String,
    pub role: String,
    pub content: String,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_arguments: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn save_message(
    db: &PgPool,
    session_id: &str,
    role: &str,
    content: &str,
    tool_call_id: Option<&str>,
    tool_name: Option<&str>,
    tool_arguments: Option<&str>,
) -> Result<String, sqlx::Error> {
    let message_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO messages (message_id, session_id, role, content, tool_call_id, tool_name, tool_arguments) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&message_id)
    .bind(session_id)
    .bind(role)
    .bind(content)
    .bind(tool_call_id)
    .bind(tool_name)
    .bind(tool_arguments)
    .execute(db)
    .await?;
    Ok(message_id)
}

/// Reconstruct session history as OpenAI-compatible message JSON.
///
/// Uses manual `row.get()` instead of a `FromRow` struct because the logic
/// merges consecutive assistant tool-call rows into a single message with a
/// `tool_calls` array — a transform that doesn't map 1:1 to DB rows.
pub async fn get_session_history(
    db: &PgPool,
    session_id: &str,
) -> Result<Vec<serde_json::Value>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT role, content, tool_call_id, tool_name, tool_arguments
        FROM messages
        WHERE session_id = $1
          AND (compressed IS NULL OR compressed = FALSE)
        ORDER BY created_at ASC
        "#,
    )
    .bind(session_id)
    .fetch_all(db)
    .await?;

    // Reconstruct messages, merging consecutive assistant tool_call rows
    // into a single assistant message with a tool_calls array.
    let mut messages: Vec<serde_json::Value> = Vec::new();

    for row in &rows {
        let role: String = row.get("role");
        let content: String = row.get("content");
        let tool_call_id: Option<String> = row.get("tool_call_id");
        let tool_name: Option<String> = row.get("tool_name");
        let tool_arguments: Option<String> = row.get("tool_arguments");

        if role == "assistant" && tool_name.is_some() {
            let tc = serde_json::json!({
                "id": tool_call_id,
                "type": "function",
                "function": {
                    "name": tool_name,
                    "arguments": tool_arguments
                }
            });
            // Merge into previous assistant message if it has tool_calls
            if let Some(last) = messages.last_mut() {
                if last.get("role").and_then(|v| v.as_str()) == Some("assistant")
                    && last.get("tool_calls").is_some()
                {
                    if let Some(arr) = last.get_mut("tool_calls").and_then(|v| v.as_array_mut()) {
                        arr.push(tc);
                        continue;
                    }
                }
            }
            messages.push(serde_json::json!({
                "role": "assistant",
                "tool_calls": [tc]
            }));
        } else if role == "tool" {
            messages.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": tool_call_id,
                "content": content
            }));
        } else {
            messages.push(serde_json::json!({
                "role": role,
                "content": content
            }));
        }
    }

    Ok(messages)
}

pub async fn mark_messages_compressed(
    db: &PgPool,
    session_id: &str,
    before_created_at: chrono::DateTime<chrono::Utc>,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE messages SET compressed = TRUE WHERE session_id = $1 AND created_at < $2 AND (compressed IS NULL OR compressed = FALSE)"
    )
    .bind(session_id)
    .bind(before_created_at)
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}
