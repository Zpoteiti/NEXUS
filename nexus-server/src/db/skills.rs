use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Skill {
    pub skill_id: String,
    pub user_id: String,
    pub name: String,
    pub description: String,
    pub always_on: bool,
    pub skill_path: String,
}

pub async fn create_skill(
    db: &PgPool,
    user_id: &str,
    name: &str,
    description: &str,
    always_on: bool,
    skill_path: &str,
) -> Result<String, sqlx::Error> {
    let skill_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO skills (skill_id, user_id, name, description, always_on, skill_path)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (user_id, name) DO UPDATE SET description = $4, always_on = $5, skill_path = $6"
    )
    .bind(&skill_id)
    .bind(user_id)
    .bind(name)
    .bind(description)
    .bind(always_on)
    .bind(skill_path)
    .execute(db)
    .await?;
    Ok(skill_id)
}

pub async fn list_skills(db: &PgPool, user_id: &str) -> Result<Vec<Skill>, sqlx::Error> {
    sqlx::query_as::<_, Skill>(
        "SELECT skill_id, user_id, name, description, always_on, skill_path FROM skills WHERE user_id = $1 ORDER BY name"
    )
    .bind(user_id)
    .fetch_all(db)
    .await
}

pub async fn list_all_skills(db: &PgPool) -> Result<Vec<Skill>, sqlx::Error> {
    sqlx::query_as::<_, Skill>(
        "SELECT skill_id, user_id, name, description, always_on, skill_path FROM skills ORDER BY user_id, name"
    )
    .fetch_all(db)
    .await
}

pub async fn get_skill(db: &PgPool, user_id: &str, name: &str) -> Result<Option<Skill>, sqlx::Error> {
    sqlx::query_as::<_, Skill>(
        "SELECT skill_id, user_id, name, description, always_on, skill_path FROM skills WHERE user_id = $1 AND name = $2"
    )
    .bind(user_id)
    .bind(name)
    .fetch_optional(db)
    .await
}

pub async fn delete_skill(db: &PgPool, user_id: &str, name: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM skills WHERE user_id = $1 AND name = $2")
        .bind(user_id)
        .bind(name)
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}
