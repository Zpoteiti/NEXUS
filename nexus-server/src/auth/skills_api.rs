/// Skills REST API handlers (list, create, delete, admin_list).

use axum::{
    extract::{Json, Path, State},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;

use nexus_common::error::{ApiError, ErrorCode};

use super::Claims;
use crate::db;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateSkillRequest {
    pub name: String,
    pub content: String,
}

/// GET /api/skills -- list current user's skills (metadata only)
pub async fn list_skills(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    match db::list_skills(&state.db, &claims.sub).await {
        Ok(skills) => Json(json!({ "skills": skills })).into_response(),
        Err(e) => {
            tracing::error!("list_skills error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to list skills").into_response()
        }
    }
}

/// POST /api/skills -- create or update a skill
pub async fn create_skill(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Json(payload): Json<CreateSkillRequest>,
) -> Response {
    let user_id = &claims.sub;

    // Validate name: alphanumeric, hyphens, underscores only
    if payload.name.is_empty() || !payload.name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return ApiError::new(ErrorCode::ValidationFailed, "invalid skill name: must be non-empty and contain only alphanumeric, hyphens, or underscores").into_response();
    }

    // Parse frontmatter
    let (fm_name, description, always_on) = crate::server_tools::skills::parse_frontmatter(&payload.content);
    let skill_name = fm_name.unwrap_or_else(|| payload.name.clone());

    // Create skill directory: skills_dir / user_id / name /
    let skill_dir = std::path::PathBuf::from(&state.config.skills_dir)
        .join(user_id)
        .join(&payload.name);

    if let Err(e) = tokio::fs::create_dir_all(&skill_dir).await {
        tracing::error!("create_skill: failed to create directory {:?}: {}", skill_dir, e);
        return ApiError::new(ErrorCode::InternalError, "failed to create skill directory").into_response();
    }

    // Write SKILL.md
    let skill_md_path = skill_dir.join("SKILL.md");
    if let Err(e) = tokio::fs::write(&skill_md_path, &payload.content).await {
        tracing::error!("create_skill: failed to write SKILL.md: {}", e);
        return ApiError::new(ErrorCode::InternalError, "failed to write skill file").into_response();
    }

    let skill_path = skill_dir.to_string_lossy().to_string();

    // Insert/update in DB
    match db::create_skill(&state.db, user_id, &skill_name, &description, always_on, &skill_path).await {
        Ok(skill_id) => Json(json!({
            "skill_id": skill_id,
            "name": skill_name,
            "description": description,
            "always_on": always_on,
        })).into_response(),
        Err(e) => {
            tracing::error!("create_skill db error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to save skill metadata").into_response()
        }
    }
}

/// DELETE /api/skills/{name} -- remove a skill
pub async fn delete_skill(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(name): Path<String>,
) -> Response {
    let user_id = &claims.sub;

    // Look up skill to get filesystem path before deleting from DB
    let skill = match db::get_skill(&state.db, user_id, &name).await {
        Ok(Some(s)) => Some(s),
        Ok(None) => None,
        Err(e) => {
            tracing::error!("delete_skill lookup error: {e}");
            return ApiError::new(ErrorCode::InternalError, "failed to look up skill").into_response();
        }
    };

    match db::delete_skill(&state.db, user_id, &name).await {
        Ok(true) => {
            // Clean up filesystem
            if let Some(skill) = skill {
                if let Err(e) = tokio::fs::remove_dir_all(&skill.skill_path).await {
                    tracing::warn!("delete_skill: failed to remove directory {}: {}", skill.skill_path, e);
                }
            }
            Json(json!({"message": "Skill deleted"})).into_response()
        }
        Ok(false) => ApiError::new(ErrorCode::NotFound, "skill not found").into_response(),
        Err(e) => {
            tracing::error!("delete_skill error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to delete skill").into_response()
        }
    }
}

/// GET /api/admin/skills -- list all skills across all users (admin only)
pub async fn admin_list_skills(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
) -> Response {
    if !claims.is_admin {
        return ApiError::new(ErrorCode::Forbidden, "admin access required").into_response();
    }
    match db::list_all_skills(&state.db).await {
        Ok(skills) => Json(json!({ "skills": skills })).into_response(),
        Err(e) => {
            tracing::error!("admin_list_skills error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to list skills").into_response()
        }
    }
}
