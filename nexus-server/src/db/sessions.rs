use serde::Serialize;
use sqlx::PgPool;

pub async fn ensure_session(
    db: &PgPool,
    session_id: &str,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sessions (session_id, user_id) VALUES ($1, $2) ON CONFLICT (session_id) DO NOTHING",
    )
    .bind(session_id)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
struct SessionOwner {
    user_id: String,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn list_sessions_by_user(
    db: &PgPool,
    user_id: &str,
) -> Result<Vec<SessionInfo>, sqlx::Error> {
    sqlx::query_as::<_, SessionInfo>(
        "SELECT session_id, created_at FROM sessions WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(db)
    .await
}

pub async fn delete_session(
    db: &PgPool,
    session_id: &str,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query("DELETE FROM messages WHERE session_id = $1")
        .bind(session_id)
        .execute(db)
        .await?;
    let result = sqlx::query(
        "DELETE FROM sessions WHERE session_id = $1 AND user_id = $2",
    )
    .bind(session_id)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn update_session_last_consolidated(
    db: &PgPool,
    session_id: &str,
    last_message_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE sessions SET last_consolidated = $1 WHERE session_id = $2")
        .bind(last_message_id)
        .bind(session_id)
        .execute(db)
        .await?;
    Ok(())
}

use super::messages::MessageInfo;

pub async fn get_session_messages(
    db: &PgPool,
    session_id: &str,
    user_id: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<MessageInfo>, sqlx::Error> {
    // Verify ownership first
    let owner: Option<SessionOwner> = sqlx::query_as(
        "SELECT user_id FROM sessions WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_optional(db)
    .await?;

    match owner {
        Some(ref o) if o.user_id == user_id => {}
        _ => return Ok(vec![]), // Not owner or not found
    }

    sqlx::query_as::<_, MessageInfo>(
        "SELECT message_id, role, content, tool_call_id, tool_name, tool_arguments, created_at FROM messages WHERE session_id = $1 ORDER BY created_at ASC LIMIT $2 OFFSET $3",
    )
    .bind(session_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(db)
    .await
}
