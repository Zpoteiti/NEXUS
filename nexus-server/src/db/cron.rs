use sqlx::PgPool;

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
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
    pub next_run_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_run_at: Option<chrono::DateTime<chrono::Utc>>,
    pub run_count: i32,
}

#[derive(Debug, sqlx::FromRow)]
struct CronJobSchedule {
    cron_expr: Option<String>,
    every_seconds: Option<i32>,
}

pub async fn create_cron_job(
    db: &PgPool,
    user_id: &str,
    name: &str,
    cron_expr: Option<&str>,
    every_seconds: Option<i32>,
    at: Option<&str>,
    timezone: &str,
    message: &str,
    channel: &str,
    chat_id: &str,
    delete_after_run: bool,
) -> Result<String, sqlx::Error> {
    let job_id = uuid::Uuid::new_v4().to_string()[..8].to_string();

    let next_run_at: Option<chrono::DateTime<chrono::Utc>> = if let Some(at_str) = at {
        chrono::DateTime::parse_from_rfc3339(at_str)
            .ok()
            .map(|dt| dt.with_timezone(&chrono::Utc))
    } else if let Some(secs) = every_seconds {
        Some(chrono::Utc::now() + chrono::Duration::seconds(secs as i64))
    } else if let Some(ref expr) = cron_expr {
        compute_next_cron_run(expr)
    } else {
        None
    };

    sqlx::query(
        "INSERT INTO cron_jobs (job_id, user_id, name, cron_expr, every_seconds, timezone, message, channel, chat_id, delete_after_run, next_run_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"
    )
    .bind(&job_id).bind(user_id).bind(name)
    .bind(cron_expr).bind(every_seconds).bind(timezone)
    .bind(message).bind(channel).bind(chat_id)
    .bind(delete_after_run).bind(next_run_at)
    .execute(db)
    .await?;

    Ok(job_id)
}

pub async fn list_cron_jobs(db: &PgPool, user_id: &str) -> Result<Vec<CronJob>, sqlx::Error> {
    sqlx::query_as::<_, CronJob>(
        "SELECT job_id, user_id, name, enabled, cron_expr, every_seconds, timezone, message, channel, chat_id, delete_after_run, next_run_at, last_run_at, run_count
         FROM cron_jobs WHERE user_id = $1 ORDER BY created_at"
    )
    .bind(user_id)
    .fetch_all(db)
    .await
}

pub async fn delete_cron_job(db: &PgPool, user_id: &str, job_id: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM cron_jobs WHERE job_id = $1 AND user_id = $2")
        .bind(job_id).bind(user_id)
        .execute(db).await?;
    Ok(result.rows_affected() > 0)
}

pub async fn get_due_cron_jobs(db: &PgPool) -> Result<Vec<CronJob>, sqlx::Error> {
    sqlx::query_as::<_, CronJob>(
        "SELECT job_id, user_id, name, enabled, cron_expr, every_seconds, timezone, message, channel, chat_id, delete_after_run, next_run_at, last_run_at, run_count
         FROM cron_jobs WHERE enabled = TRUE AND next_run_at IS NOT NULL AND next_run_at <= NOW()"
    )
    .fetch_all(db)
    .await
}

pub async fn update_cron_job_after_run(db: &PgPool, job_id: &str, delete_after_run: bool) -> Result<(), sqlx::Error> {
    if delete_after_run {
        sqlx::query("DELETE FROM cron_jobs WHERE job_id = $1")
            .bind(job_id).execute(db).await?;
    } else {
        // Fetch job to compute next_run_at properly
        let row = sqlx::query_as::<_, CronJobSchedule>(
            "SELECT cron_expr, every_seconds FROM cron_jobs WHERE job_id = $1"
        ).bind(job_id).fetch_optional(db).await?;

        let next_run_at = if let Some(CronJobSchedule { cron_expr, every_seconds }) = row {
            if let Some(secs) = every_seconds {
                Some(chrono::Utc::now() + chrono::Duration::seconds(secs as i64))
            } else if let Some(ref expr) = cron_expr {
                compute_next_cron_run(expr)
            } else {
                None
            }
        } else {
            None
        };

        sqlx::query(
            "UPDATE cron_jobs SET last_run_at = NOW(), run_count = run_count + 1, next_run_at = $2 WHERE job_id = $1"
        )
        .bind(job_id).bind(next_run_at)
        .execute(db).await?;
    }
    Ok(())
}

pub async fn update_cron_job(
    db: &PgPool,
    user_id: &str,
    job_id: &str,
    enabled: Option<bool>,
    message: Option<&str>,
) -> Result<bool, sqlx::Error> {
    // Build dynamic update
    let mut set_clauses = Vec::new();
    let mut param_idx = 3u32; // $1 = job_id, $2 = user_id

    if enabled.is_some() {
        set_clauses.push(format!("enabled = ${param_idx}"));
        param_idx += 1;
    }
    if message.is_some() {
        set_clauses.push(format!("message = ${param_idx}"));
        // param_idx += 1;
    }

    if set_clauses.is_empty() {
        return Ok(false);
    }

    let sql = format!(
        "UPDATE cron_jobs SET {} WHERE job_id = $1 AND user_id = $2",
        set_clauses.join(", ")
    );

    let mut query = sqlx::query(&sql).bind(job_id).bind(user_id);
    if let Some(e) = enabled {
        query = query.bind(e);
    }
    if let Some(m) = message {
        query = query.bind(m);
    }

    let result = query.execute(db).await?;
    Ok(result.rows_affected() > 0)
}

/// Compute the next run time from a cron expression.
/// Accepts both standard 5-field crontab (min hour day month dow) and
/// the cron crate's 7-field format (sec min hour day month dow year).
pub fn compute_next_cron_run(expr: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use std::str::FromStr;
    // Convert 5-field standard crontab to 7-field by adding seconds (0) and year (*)
    let full_expr = match expr.split_whitespace().count() {
        5 => format!("0 {} *", expr),  // prepend seconds=0, append year=*
        6 => format!("0 {}", expr),    // prepend seconds=0
        _ => expr.to_string(),         // assume 7-field or let cron crate handle
    };
    let schedule = cron::Schedule::from_str(&full_expr).ok()?;
    schedule.upcoming(chrono::Utc).next()
}
