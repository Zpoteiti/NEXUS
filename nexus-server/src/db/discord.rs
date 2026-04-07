use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DiscordConfig {
    pub user_id: String,
    pub bot_token: String,
    pub bot_user_id: Option<String>,
    pub enabled: bool,
    pub allowed_users: Vec<String>,
    pub owner_discord_id: Option<String>,
}

pub async fn get_all_discord_configs(
    db: &PgPool,
) -> Result<Vec<DiscordConfig>, sqlx::Error> {
    sqlx::query_as::<_, DiscordConfig>(
        r#"
        SELECT user_id, bot_token, bot_user_id, enabled, allowed_users, owner_discord_id
        FROM discord_configs
        WHERE enabled = TRUE
        "#,
    )
    .fetch_all(db)
    .await
}

pub async fn get_discord_config_by_user_id(
    db: &PgPool,
    user_id: &str,
) -> Result<Option<DiscordConfig>, sqlx::Error> {
    sqlx::query_as::<_, DiscordConfig>(
        r#"
        SELECT user_id, bot_token, bot_user_id, enabled, allowed_users, owner_discord_id
        FROM discord_configs
        WHERE user_id = $1
        "#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
}

pub async fn update_bot_user_id(
    db: &PgPool,
    user_id: &str,
    bot_user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE discord_configs
        SET bot_user_id = $1, updated_at = NOW()
        WHERE user_id = $2
        "#,
    )
    .bind(bot_user_id)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn upsert_discord_config(
    db: &PgPool,
    user_id: &str,
    bot_token: &str,
    allowed_users: &[String],
    owner_discord_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO discord_configs (user_id, bot_token, allowed_users, owner_discord_id)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (user_id) DO UPDATE
        SET bot_token = $2, allowed_users = $3, owner_discord_id = $4, updated_at = NOW()
        "#,
    )
    .bind(user_id)
    .bind(bot_token)
    .bind(allowed_users)
    .bind(owner_discord_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete_discord_config(
    db: &PgPool,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM discord_configs WHERE user_id = $1")
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}
