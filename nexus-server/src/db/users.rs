use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub user_id: String,
    pub email: String,
    pub password_hash: String,
    pub is_admin: bool,
    pub soul: Option<String>,
    pub memory_text: String,
    pub created_at: DateTime<Utc>,
}

pub async fn create_user(
    pool: &PgPool,
    user_id: &str,
    email: &str,
    password_hash: &str,
    is_admin: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO users (user_id, email, password_hash, is_admin) VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(email)
    .bind(password_hash)
    .bind(is_admin)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn find_by_email(pool: &PgPool, email: &str) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = $1")
        .bind(email)
        .fetch_optional(pool)
        .await
}

pub async fn find_by_id(pool: &PgPool, user_id: &str) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>("SELECT * FROM users WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
}

pub async fn update_soul(
    pool: &PgPool,
    user_id: &str,
    soul: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET soul = $1 WHERE user_id = $2")
        .bind(soul)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_memory(pool: &PgPool, user_id: &str, memory: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET memory_text = $1 WHERE user_id = $2")
        .bind(memory)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}
