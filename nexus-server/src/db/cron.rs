use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CronJob {
    pub job_id: String,
    pub user_id: String,
    pub name: String,
    pub enabled: bool,
    pub cron_expr: Option<String>,
    pub every_seconds: Option<i32>,
    pub timezone: String,
    pub message: String,
    pub channel: String,
    pub chat_id: String,
    pub delete_after_run: bool,
    pub deliver: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub run_count: i32,
    pub created_at: DateTime<Utc>,
}

#[allow(clippy::too_many_arguments)]
pub async fn create_job(
    pool: &PgPool,
    job_id: &str,
    user_id: &str,
    name: &str,
    cron_expr: Option<String>,
    every_seconds: Option<i32>,
    timezone: &str,
    message: &str,
    channel: &str,
    chat_id: &str,
    delete_after_run: bool,
    deliver: bool,
    next_run_at: Option<DateTime<Utc>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO cron_jobs (job_id, user_id, name, cron_expr, every_seconds, timezone, message, channel, chat_id, delete_after_run, deliver, next_run_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(job_id)
    .bind(user_id)
    .bind(name)
    .bind(cron_expr)
    .bind(every_seconds)
    .bind(timezone)
    .bind(message)
    .bind(channel)
    .bind(chat_id)
    .bind(delete_after_run)
    .bind(deliver)
    .bind(next_run_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_by_user(pool: &PgPool, user_id: &str) -> Result<Vec<CronJob>, sqlx::Error> {
    sqlx::query_as::<_, CronJob>("SELECT * FROM cron_jobs WHERE user_id = $1 ORDER BY created_at")
        .bind(user_id)
        .fetch_all(pool)
        .await
}

pub async fn delete_job(pool: &PgPool, job_id: &str, user_id: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM cron_jobs WHERE job_id = $1 AND user_id = $2")
        .bind(job_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn find_due_jobs(pool: &PgPool) -> Result<Vec<CronJob>, sqlx::Error> {
    sqlx::query_as::<_, CronJob>(
        "SELECT * FROM cron_jobs WHERE enabled = true AND next_run_at <= NOW()",
    )
    .fetch_all(pool)
    .await
}

pub async fn update_after_run(
    pool: &PgPool,
    job_id: &str,
    next_run_at: Option<DateTime<Utc>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE cron_jobs SET last_run_at = NOW(), run_count = run_count + 1, next_run_at = $1 WHERE job_id = $2",
    )
    .bind(next_run_at)
    .bind(job_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn disable_job(pool: &PgPool, job_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE cron_jobs SET enabled = false, next_run_at = NULL WHERE job_id = $1")
        .bind(job_id)
        .execute(pool)
        .await?;
    Ok(())
}
