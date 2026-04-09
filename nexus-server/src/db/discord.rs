use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct DiscordConfig {
    pub user_id: String,
    pub bot_token: String,
    pub bot_user_id: Option<String>,
    pub owner_discord_id: Option<String>,
    pub enabled: bool,
    pub allowed_users: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn upsert_config(
    pool: &PgPool,
    user_id: &str,
    bot_token: &str,
    owner_discord_id: &str,
    allowed_users: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO discord_configs (user_id, bot_token, owner_discord_id, allowed_users, updated_at)
         VALUES ($1, $2, $3, $4, NOW())
         ON CONFLICT (user_id) DO UPDATE SET
           bot_token = $2, owner_discord_id = $3, allowed_users = $4, updated_at = NOW()",
    )
    .bind(user_id)
    .bind(bot_token)
    .bind(owner_discord_id)
    .bind(allowed_users)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_config(
    pool: &PgPool,
    user_id: &str,
) -> Result<Option<DiscordConfig>, sqlx::Error> {
    sqlx::query_as::<_, DiscordConfig>("SELECT * FROM discord_configs WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
}

pub async fn delete_config(pool: &PgPool, user_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM discord_configs WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_enabled(pool: &PgPool) -> Result<Vec<DiscordConfig>, sqlx::Error> {
    sqlx::query_as::<_, DiscordConfig>("SELECT * FROM discord_configs WHERE enabled = true")
        .fetch_all(pool)
        .await
}
