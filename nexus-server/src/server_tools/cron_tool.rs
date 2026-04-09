//! Cron server tool: unified add/list/remove interface.

use crate::server_tools::ToolContext;
use crate::state::AppState;
use serde_json::Value;
use std::sync::Arc;

pub async fn cron(state: &Arc<AppState>, ctx: &ToolContext, args: &Value) -> (i32, String) {
    // Nested prevention: refuse if running inside a cron session
    if ctx.is_cron {
        return (
            1,
            "Cannot create cron jobs from within a cron job execution.".into(),
        );
    }

    let action = match args.get("action").and_then(Value::as_str) {
        Some(a) => a,
        None => return (1, "Missing required parameter: action".into()),
    };

    match action {
        "add" => add_job(state, ctx, args).await,
        "list" => list_jobs(state, &ctx.user_id).await,
        "remove" => remove_job(state, &ctx.user_id, args).await,
        _ => (
            1,
            format!("Unknown action: {action}. Use add, list, or remove."),
        ),
    }
}

async fn add_job(state: &Arc<AppState>, ctx: &ToolContext, args: &Value) -> (i32, String) {
    let message = match args.get("message").and_then(Value::as_str) {
        Some(m) => m.to_string(),
        None => return (1, "Missing required parameter: message".into()),
    };
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .unwrap_or_else(|| message.chars().take(30).collect());

    let channel = args
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or(&ctx.channel)
        .to_string();
    let chat_id = args
        .get("chat_id")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .or_else(|| ctx.chat_id.clone())
        .unwrap_or_default();
    let timezone = args
        .get("timezone")
        .and_then(Value::as_str)
        .unwrap_or("UTC")
        .to_string();
    let delete_after_run = args
        .get("delete_after_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let deliver = args.get("deliver").and_then(Value::as_bool).unwrap_or(true);

    let cron_expr = args.get("cron_expr").and_then(Value::as_str);
    let every_seconds = args.get("every_seconds").and_then(Value::as_i64);
    let at = args.get("at").and_then(Value::as_str);

    // Validate exactly one scheduling mode
    let modes = [cron_expr.is_some(), every_seconds.is_some(), at.is_some()];
    if modes.iter().filter(|&&b| b).count() != 1 {
        return (
            1,
            "Exactly one of cron_expr, every_seconds, or at must be specified.".into(),
        );
    }

    // Validate timezone
    if timezone.parse::<chrono_tz::Tz>().is_err() {
        return (1, format!("Invalid timezone: {timezone}"));
    }

    let job_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let now = chrono::Utc::now();

    // Compute next_run_at
    let next_run_at = if let Some(expr) = cron_expr {
        match compute_next_cron(expr, &timezone) {
            Ok(t) => Some(t),
            Err(e) => return (1, format!("Invalid cron expression: {e}")),
        }
    } else if let Some(secs) = every_seconds {
        Some(now + chrono::Duration::seconds(secs))
    } else if let Some(at_str) = at {
        match chrono::DateTime::parse_from_rfc3339(at_str) {
            Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
            Err(_) => {
                // Try naive datetime with timezone
                match chrono::NaiveDateTime::parse_from_str(at_str, "%Y-%m-%dT%H:%M:%S") {
                    Ok(naive) => {
                        let tz: chrono_tz::Tz = timezone.parse().unwrap();
                        let dt = naive
                            .and_local_timezone(tz)
                            .single()
                            .map(|d| d.with_timezone(&chrono::Utc))
                            .unwrap_or_else(|| naive.and_utc());
                        Some(dt)
                    }
                    Err(e) => return (1, format!("Invalid datetime: {e}")),
                }
            }
        }
    } else {
        None
    };

    // For at mode, default delete_after_run to true
    let delete_after_run = if at.is_some() && !args.get("delete_after_run").is_some() {
        true
    } else {
        delete_after_run
    };

    match crate::db::cron::create_job(
        &state.db,
        &job_id,
        &ctx.user_id,
        &name,
        cron_expr.map(|s| s.to_string()),
        every_seconds.map(|s| s as i32),
        &timezone,
        &message,
        &channel,
        &chat_id,
        delete_after_run,
        deliver,
        next_run_at,
    )
    .await
    {
        Ok(()) => (
            0,
            format!("Cron job '{name}' created (ID: {job_id}). Next run: {next_run_at:?}"),
        ),
        Err(e) => (1, format!("Failed to create cron job: {e}")),
    }
}

async fn list_jobs(state: &Arc<AppState>, user_id: &str) -> (i32, String) {
    match crate::db::cron::list_by_user(&state.db, user_id).await {
        Ok(jobs) => {
            if jobs.is_empty() {
                return (0, "No cron jobs.".into());
            }
            let mut out = String::new();
            for job in &jobs {
                let schedule = if let Some(ref expr) = job.cron_expr {
                    format!("cron: {expr}")
                } else if let Some(secs) = job.every_seconds {
                    format!("every {secs}s")
                } else {
                    "one-shot".into()
                };
                let status = if job.enabled { "enabled" } else { "disabled" };
                out += &format!(
                    "- {} [{}] ({}) — {} | next: {:?} | runs: {}\n",
                    job.name, job.job_id, status, schedule, job.next_run_at, job.run_count
                );
            }
            (0, out)
        }
        Err(e) => (1, format!("Failed to list jobs: {e}")),
    }
}

async fn remove_job(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let job_id = match args.get("job_id").and_then(Value::as_str) {
        Some(id) => id,
        None => return (1, "Missing required parameter: job_id".into()),
    };
    match crate::db::cron::delete_job(&state.db, job_id, user_id).await {
        Ok(true) => (0, format!("Cron job {job_id} deleted.")),
        Ok(false) => (1, format!("Cron job {job_id} not found.")),
        Err(e) => (1, format!("Failed to delete: {e}")),
    }
}

pub fn compute_next_cron_pub(
    expr: &str,
    timezone: &str,
) -> Result<chrono::DateTime<chrono::Utc>, String> {
    compute_next_cron(expr, timezone)
}

fn compute_next_cron(expr: &str, timezone: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    use cron::Schedule;
    use std::str::FromStr;

    let tz: chrono_tz::Tz = timezone
        .parse()
        .map_err(|_| format!("Bad timezone: {timezone}"))?;

    // Convert 5-field to 7-field (cron crate needs seconds + year)
    let parts: Vec<&str> = expr.split_whitespace().collect();
    let full_expr = match parts.len() {
        5 => format!("0 {expr} *"),
        6 => format!("{expr} *"),
        7 => expr.to_string(),
        _ => return Err(format!("Invalid cron expression: {expr}")),
    };

    let schedule = Schedule::from_str(&full_expr).map_err(|e| format!("Parse cron: {e}"))?;
    let now = chrono::Utc::now().with_timezone(&tz);
    schedule
        .after(&now)
        .next()
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .ok_or_else(|| "No next occurrence".to_string())
}
