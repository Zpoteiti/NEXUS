use sqlx::PgPool;

#[derive(Debug, sqlx::FromRow)]
pub struct Checkpoint {
    pub session_id: String,
    pub user_id: String,
    pub channel: String,
    pub chat_id: String,
    pub messages: serde_json::Value,
    pub iteration: i32,
}

pub async fn save_checkpoint(
    db: &PgPool,
    session_id: &str,
    user_id: &str,
    messages: &[serde_json::Value],
    iteration: u32,
    channel: &str,
    chat_id: &str,
) -> Result<(), sqlx::Error> {
    let messages_json = serde_json::Value::Array(messages.to_vec());
    sqlx::query(
        "INSERT INTO agent_checkpoints (session_id, user_id, messages, iteration, channel, chat_id, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, NOW())
         ON CONFLICT (session_id) DO UPDATE SET messages = $3, iteration = $4, updated_at = NOW()"
    )
    .bind(session_id).bind(user_id)
    .bind(&messages_json).bind(iteration as i32)
    .bind(channel).bind(chat_id)
    .execute(db).await?;
    Ok(())
}

pub async fn delete_checkpoint(db: &PgPool, session_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM agent_checkpoints WHERE session_id = $1")
        .bind(session_id).execute(db).await?;
    Ok(())
}

pub async fn list_all_checkpoints(db: &PgPool) -> Result<Vec<Checkpoint>, sqlx::Error> {
    sqlx::query_as::<_, Checkpoint>(
        "SELECT session_id, user_id, channel, chat_id, messages, iteration FROM agent_checkpoints"
    )
    .fetch_all(db)
    .await
}
