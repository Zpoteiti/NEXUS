//! Cron job CRUD API endpoints.

use crate::auth::extract_claims;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use nexus_common::error::{ApiError, ErrorCode};
use serde::Deserialize;
use std::sync::Arc;

fn claims(headers: &HeaderMap, state: &AppState) -> Result<crate::auth::Claims, ApiError> {
    extract_claims(headers, &state.config.jwt_secret)
}

async fn list_jobs(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let jobs = crate::db::cron::list_by_user(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(serde_json::json!({ "cron_jobs": jobs })))
}

#[derive(Deserialize)]
struct CreateJobRequest {
    name: Option<String>,
    message: String,
    cron_expr: Option<String>,
    every_seconds: Option<i32>,
    at: Option<String>,
    channel: Option<String>,
    chat_id: Option<String>,
    timezone: Option<String>,
    delete_after_run: Option<bool>,
    deliver: Option<bool>,
}

async fn create_job(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateJobRequest>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), ApiError> {
    let c = claims(&headers, &state)?;

    let name = req
        .name
        .unwrap_or_else(|| req.message.chars().take(30).collect());
    let channel = req.channel.unwrap_or_else(|| "gateway".into());
    let chat_id = req.chat_id.unwrap_or_default();
    let timezone = req.timezone.unwrap_or_else(|| "UTC".into());
    let delete_after_run = req.delete_after_run.unwrap_or(req.at.is_some());
    let deliver = req.deliver.unwrap_or(true);

    // Validate timezone
    if timezone.parse::<chrono_tz::Tz>().is_err() {
        return Err(ApiError::new(
            ErrorCode::ValidationFailed,
            format!("Invalid timezone: {timezone}"),
        ));
    }

    // Validate scheduling mode
    let modes = [
        req.cron_expr.is_some(),
        req.every_seconds.is_some(),
        req.at.is_some(),
    ];
    if modes.iter().filter(|&&b| b).count() != 1 {
        return Err(ApiError::new(
            ErrorCode::ValidationFailed,
            "Exactly one of cron_expr, every_seconds, or at must be specified",
        ));
    }

    let job_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let now = chrono::Utc::now();

    // Compute next_run_at
    let next_run_at = if let Some(ref expr) = req.cron_expr {
        Some(
            crate::server_tools::cron_tool::compute_next_cron_pub(expr, &timezone)
                .map_err(|e| ApiError::new(ErrorCode::ValidationFailed, e))?,
        )
    } else if let Some(secs) = req.every_seconds {
        Some(now + chrono::Duration::seconds(secs as i64))
    } else if let Some(ref at) = req.at {
        Some(parse_at(at, &timezone).map_err(|e| ApiError::new(ErrorCode::ValidationFailed, e))?)
    } else {
        None
    };

    crate::db::cron::create_job(
        &state.db,
        &job_id,
        &c.sub,
        &name,
        req.cron_expr,
        req.every_seconds,
        &timezone,
        &req.message,
        &channel,
        &chat_id,
        delete_after_run,
        deliver,
        next_run_at,
    )
    .await
    .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({ "job_id": job_id })),
    ))
}

#[derive(Deserialize)]
struct UpdateJobRequest {
    enabled: Option<bool>,
    message: Option<String>,
    name: Option<String>,
}

async fn update_job(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
    Json(req): Json<UpdateJobRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    // Simple update — just enabled/message/name fields
    // Build dynamic UPDATE query
    let mut sets = Vec::new();
    let mut bind_idx = 1;

    if req.enabled.is_some() {
        sets.push(format!("enabled = ${bind_idx}"));
        bind_idx += 1;
    }
    if req.message.is_some() {
        sets.push(format!("message = ${bind_idx}"));
        bind_idx += 1;
    }
    if req.name.is_some() {
        sets.push(format!("name = ${bind_idx}"));
        bind_idx += 1;
    }

    if sets.is_empty() {
        return Ok(Json(serde_json::json!({ "message": "Nothing to update" })));
    }

    let sql = format!(
        "UPDATE cron_jobs SET {} WHERE job_id = ${bind_idx} AND user_id = ${}",
        sets.join(", "),
        bind_idx + 1
    );

    let mut query = sqlx::query(&sql);
    if let Some(enabled) = req.enabled {
        query = query.bind(enabled);
    }
    if let Some(ref message) = req.message {
        query = query.bind(message);
    }
    if let Some(ref name) = req.name {
        query = query.bind(name);
    }
    query = query.bind(&job_id).bind(&c.sub);

    query
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;

    Ok(Json(serde_json::json!({ "message": "Cron job updated" })))
}

async fn delete_job(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let deleted = crate::db::cron::delete_job(&state.db, &job_id, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    if !deleted {
        return Err(ApiError::new(ErrorCode::NotFound, "Cron job not found"));
    }
    Ok(Json(serde_json::json!({ "message": "Cron job deleted" })))
}

fn parse_at(at: &str, timezone: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    // Try RFC 3339 first
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(at) {
        return Ok(dt.with_timezone(&chrono::Utc));
    }
    // Try naive with timezone
    let naive = chrono::NaiveDateTime::parse_from_str(at, "%Y-%m-%dT%H:%M:%S")
        .map_err(|e| format!("Invalid datetime: {e}"))?;
    let tz: chrono_tz::Tz = timezone
        .parse()
        .map_err(|_| format!("Invalid timezone: {timezone}"))?;
    naive
        .and_local_timezone(tz)
        .single()
        .map(|d| d.with_timezone(&chrono::Utc))
        .ok_or_else(|| "Ambiguous datetime".into())
}

pub fn cron_api_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/cron-jobs", get(list_jobs).post(create_job))
        .route(
            "/api/cron-jobs/{job_id}",
            patch(update_job).delete(delete_job),
        )
}
