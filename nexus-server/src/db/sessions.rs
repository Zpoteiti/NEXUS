use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Session {
    pub session_id: String,
    pub user_id: String,
    pub created_at: DateTime<Utc>,
}

pub async fn create_session(
    pool: &PgPool,
    session_id: &str,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sessions (session_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(session_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_by_user(pool: &PgPool, user_id: &str) -> Result<Vec<Session>, sqlx::Error> {
    sqlx::query_as::<_, Session>(
        "SELECT * FROM sessions WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn delete_session(pool: &PgPool, session_id: &str) -> Result<bool, sqlx::Error> {
    sqlx::query("DELETE FROM messages WHERE session_id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;
    let result = sqlx::query("DELETE FROM sessions WHERE session_id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn find_by_id(pool: &PgPool, session_id: &str) -> Result<Option<Session>, sqlx::Error> {
    sqlx::query_as::<_, Session>("SELECT * FROM sessions WHERE session_id = $1")
        .bind(session_id)
        .fetch_optional(pool)
        .await
}
