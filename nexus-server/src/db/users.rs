use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)] // Fields populated by sqlx FromRow, used in auth logic
pub struct User {
    pub user_id: String,
    pub email: String,
    pub password_hash: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct UserProfile {
    pub user_id: String,
    pub email: String,
    pub is_admin: bool,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn create_user(
    db: &PgPool,
    email: &str,
    password_hash: &str,
    is_admin: bool,
) -> Result<String, sqlx::Error> {
    let user_id = Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO users (user_id, email, password_hash, is_admin)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(&user_id)
    .bind(email)
    .bind(password_hash)
    .bind(is_admin)
    .execute(db)
    .await?;
    Ok(user_id)
}

pub async fn get_user_by_email(
    db: &PgPool,
    email: &str,
) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>(
        r#"
        SELECT user_id, email, password_hash, is_admin
        FROM users
        WHERE email = $1
        "#,
    )
    .bind(email)
    .fetch_optional(db)
    .await
}

pub async fn get_user_profile(
    db: &PgPool,
    user_id: &str,
) -> Result<Option<UserProfile>, sqlx::Error> {
    sqlx::query_as::<_, UserProfile>(
        r#"
        SELECT user_id, email, is_admin, created_at
        FROM users
        WHERE user_id = $1
        "#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
}

// ============================================================================
// Soul & Preferences
// ============================================================================

pub async fn get_user_soul(db: &PgPool, user_id: &str) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query_as::<_, (Option<String>,)>(
        "SELECT soul FROM users WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    Ok(row.and_then(|r| r.0))
}

pub async fn update_user_soul(db: &PgPool, user_id: &str, soul: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET soul = $1 WHERE user_id = $2")
        .bind(soul)
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(())
}



// ============================================================================
// User Memory (simple text string, 4K cap)
// ============================================================================

pub async fn get_user_memory(db: &PgPool, user_id: &str) -> Result<String, sqlx::Error> {
    let row = sqlx::query_as::<_, (String,)>(
        "SELECT memory_text FROM users WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.0).unwrap_or_default())
}

pub async fn update_user_memory(
    db: &PgPool,
    user_id: &str,
    memory: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET memory_text = $1 WHERE user_id = $2")
        .bind(memory)
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(())
}
