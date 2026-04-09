//! Skills CRUD API endpoints.

use crate::auth::extract_claims;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use nexus_common::error::{ApiError, ErrorCode};
use serde::Deserialize;
use std::sync::Arc;

fn claims(headers: &HeaderMap, state: &AppState) -> Result<crate::auth::Claims, ApiError> {
    extract_claims(headers, &state.config.jwt_secret)
}

async fn list_skills(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let skills = crate::db::skills::list_by_user(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(serde_json::json!({ "skills": skills })))
}

#[derive(Deserialize)]
struct CreateSkillRequest {
    name: String,
    content: String,
}

async fn create_skill(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateSkillRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;

    // Parse frontmatter from content
    let (name, description, always_on) =
        crate::server_tools::skills::parse_skill_frontmatter_pub(&req.content)
            .map_err(|e| ApiError::new(ErrorCode::ValidationFailed, e))?;

    // Use provided name or frontmatter name
    let skill_name = if req.name.is_empty() { name } else { req.name };

    // Write to disk
    let skill_dir = format!("{}/{}/{skill_name}", state.config.skills_dir, c.sub);
    tokio::fs::create_dir_all(&skill_dir)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("mkdir: {e}")))?;
    tokio::fs::write(format!("{skill_dir}/SKILL.md"), &req.content)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("write: {e}")))?;

    // Upsert in DB
    let skill_id = uuid::Uuid::new_v4().to_string();
    crate::db::skills::upsert_skill(
        &state.db,
        &skill_id,
        &c.sub,
        &skill_name,
        &description,
        always_on,
        &skill_dir,
    )
    .await
    .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;

    Ok(Json(serde_json::json!({
        "skill_id": skill_id,
        "name": skill_name,
        "description": description,
        "always_on": always_on,
    })))
}

#[derive(Deserialize)]
struct InstallSkillRequest {
    repo: String,
    branch: Option<String>,
}

async fn install_skill(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<InstallSkillRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let args = serde_json::json!({
        "repo": req.repo,
        "branch": req.branch.unwrap_or_else(|| "main".into()),
    });
    let (code, output) = crate::server_tools::skills::install_skill(&state, &c.sub, &args).await;
    if code != 0 {
        return Err(ApiError::new(ErrorCode::InternalError, output));
    }
    Ok(Json(serde_json::json!({ "message": output })))
}

async fn delete_skill_handler(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let deleted = crate::db::skills::delete_skill(&state.db, &c.sub, &name)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    if !deleted {
        return Err(ApiError::new(ErrorCode::NotFound, "Skill not found"));
    }
    // Remove from disk
    let skill_dir = format!("{}/{}/{name}", state.config.skills_dir, c.sub);
    let _ = tokio::fs::remove_dir_all(&skill_dir).await;
    Ok(Json(serde_json::json!({ "message": "Skill deleted" })))
}

pub fn skills_api_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/skills", get(list_skills).post(create_skill))
        .route("/api/skills/install", post(install_skill))
        .route("/api/skills/{name}", delete(delete_skill_handler))
}
