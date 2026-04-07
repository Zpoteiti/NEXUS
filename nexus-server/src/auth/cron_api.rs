/// Cron REST API handlers (list, create, delete, update).

use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;

use nexus_common::error::{ApiError, ErrorCode};

use super::Claims;
use crate::db;
use crate::state::AppState;

/// GET /api/cron-jobs -- list current user's cron jobs
pub async fn list_cron_jobs_api(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    match db::list_cron_jobs(&state.db, &claims.sub).await {
        Ok(jobs) => Json(json!({ "cron_jobs": jobs })).into_response(),
        Err(e) => {
            tracing::error!("list_cron_jobs_api error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to list cron jobs").into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateCronJobRequest {
    pub message: String,
    pub cron_expr: Option<String>,
    pub every_seconds: Option<i32>,
    pub at: Option<String>,
    pub timezone: Option<String>,
    pub channel: String,
    #[serde(default)]
    pub chat_id: Option<String>,
    #[serde(default)]
    pub delete_after_run: bool,
    pub name: Option<String>,
}

/// POST /api/cron-jobs -- create a cron job
pub async fn create_cron_job_api(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Json(payload): Json<CreateCronJobRequest>,
) -> Response {
    let name = payload.name.as_deref().unwrap_or("api-job");
    let timezone = payload.timezone.as_deref().unwrap_or("UTC");

    match db::create_cron_job(
        &state.db,
        &claims.sub,
        name,
        payload.cron_expr.as_deref(),
        payload.every_seconds,
        payload.at.as_deref(),
        timezone,
        &payload.message,
        &payload.channel,
        payload.chat_id.as_deref().unwrap_or(""),
        payload.delete_after_run,
    )
    .await
    {
        Ok(job_id) => (StatusCode::CREATED, Json(json!({ "job_id": job_id }))).into_response(),
        Err(e) => {
            tracing::error!("create_cron_job_api error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to create cron job").into_response()
        }
    }
}

/// DELETE /api/cron-jobs/{job_id} -- delete a cron job
pub async fn delete_cron_job_api(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(job_id): Path<String>,
) -> Response {
    match db::delete_cron_job(&state.db, &claims.sub, &job_id).await {
        Ok(true) => Json(json!({"message": "Cron job deleted"})).into_response(),
        Ok(false) => ApiError::new(ErrorCode::NotFound, "cron job not found").into_response(),
        Err(e) => {
            tracing::error!("delete_cron_job_api error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to delete cron job").into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateCronJobRequest {
    pub enabled: Option<bool>,
    pub message: Option<String>,
}

/// PATCH /api/cron-jobs/{job_id} -- update a cron job (enable/disable, change message)
pub async fn update_cron_job_api(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(job_id): Path<String>,
    Json(payload): Json<UpdateCronJobRequest>,
) -> Response {
    match db::update_cron_job(
        &state.db,
        &claims.sub,
        &job_id,
        payload.enabled,
        payload.message.as_deref(),
    )
    .await
    {
        Ok(true) => Json(json!({"message": "Cron job updated"})).into_response(),
        Ok(false) => Json(json!({"message": "No changes applied"})).into_response(),
        Err(e) => {
            tracing::error!("update_cron_job_api error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to update cron job").into_response()
        }
    }
}
