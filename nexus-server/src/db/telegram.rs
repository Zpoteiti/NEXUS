use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct TelegramConfig {
    pub user_id: String,
    pub bot_token: String,
    pub owner_telegram_id: Option<String>,
    pub enabled: bool,
    pub allowed_users: Vec<String>,
    pub group_policy: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn upsert_config(
    pool: &PgPool,
    user_id: &str,
    bot_token: &str,
    owner_telegram_id: &str,
    allowed_users: &[String],
    group_policy: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO telegram_configs (user_id, bot_token, owner_telegram_id, allowed_users, group_policy, updated_at)
         VALUES ($1, $2, $3, $4, $5, NOW())
         ON CONFLICT (user_id) DO UPDATE SET
           bot_token = $2, owner_telegram_id = $3, allowed_users = $4, group_policy = $5, updated_at = NOW()",
    )
    .bind(user_id)
    .bind(bot_token)
    .bind(owner_telegram_id)
    .bind(allowed_users)
    .bind(group_policy)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_config(
    pool: &PgPool,
    user_id: &str,
) -> Result<Option<TelegramConfig>, sqlx::Error> {
    sqlx::query_as::<_, TelegramConfig>("SELECT * FROM telegram_configs WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
}

pub async fn delete_config(pool: &PgPool, user_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM telegram_configs WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_enabled(pool: &PgPool) -> Result<Vec<TelegramConfig>, sqlx::Error> {
    sqlx::query_as::<_, TelegramConfig>("SELECT * FROM telegram_configs WHERE enabled = true")
        .fetch_all(pool)
        .await
}
