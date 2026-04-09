use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Skill {
    pub skill_id: String,
    pub user_id: String,
    pub name: String,
    pub description: String,
    pub always_on: bool,
    pub skill_path: String,
    pub created_at: DateTime<Utc>,
}

pub async fn upsert_skill(
    pool: &PgPool,
    skill_id: &str,
    user_id: &str,
    name: &str,
    description: &str,
    always_on: bool,
    skill_path: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO skills (skill_id, user_id, name, description, always_on, skill_path)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (user_id, name) DO UPDATE SET
           description = $4, always_on = $5, skill_path = $6",
    )
    .bind(skill_id)
    .bind(user_id)
    .bind(name)
    .bind(description)
    .bind(always_on)
    .bind(skill_path)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_by_user(pool: &PgPool, user_id: &str) -> Result<Vec<Skill>, sqlx::Error> {
    sqlx::query_as::<_, Skill>("SELECT * FROM skills WHERE user_id = $1 ORDER BY name")
        .bind(user_id)
        .fetch_all(pool)
        .await
}

pub async fn find_by_name(
    pool: &PgPool,
    user_id: &str,
    name: &str,
) -> Result<Option<Skill>, sqlx::Error> {
    sqlx::query_as::<_, Skill>("SELECT * FROM skills WHERE user_id = $1 AND name = $2")
        .bind(user_id)
        .bind(name)
        .fetch_optional(pool)
        .await
}

pub async fn delete_skill(pool: &PgPool, user_id: &str, name: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM skills WHERE user_id = $1 AND name = $2")
        .bind(user_id)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn list_all(pool: &PgPool) -> Result<Vec<Skill>, sqlx::Error> {
    sqlx::query_as::<_, Skill>("SELECT * FROM skills ORDER BY user_id, name")
        .fetch_all(pool)
        .await
}
