//! User, session, and file endpoints. All require JWT.

use crate::auth::{Claims, extract_claims};
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{HeaderMap, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use nexus_common::consts::MEMORY_TEXT_MAX_CHARS;
use nexus_common::error::{ApiError, ErrorCode};
use serde::Deserialize;
use std::sync::Arc;

fn claims(headers: &HeaderMap, state: &AppState) -> Result<Claims, ApiError> {
    extract_claims(headers, &state.config.jwt_secret)
}

// -- User Profile --

async fn get_profile(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let user = crate::db::users::find_by_id(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "User not found"))?;
    Ok(Json(serde_json::json!({
        "user_id": user.user_id,
        "email": user.email,
        "is_admin": user.is_admin,
        "created_at": user.created_at.to_rfc3339(),
    })))
}

// -- Soul --

async fn get_soul(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let user = crate::db::users::find_by_id(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "User not found"))?;
    Ok(Json(serde_json::json!({ "soul": user.soul })))
}

#[derive(Deserialize)]
struct SoulUpdate {
    soul: String,
}

async fn patch_soul(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<SoulUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    crate::db::users::update_soul(&state.db, &c.sub, Some(&req.soul))
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(serde_json::json!({ "message": "Soul updated" })))
}

// -- Memory --

async fn get_memory(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let user = crate::db::users::find_by_id(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "User not found"))?;
    Ok(Json(serde_json::json!({ "memory": user.memory_text })))
}

#[derive(Deserialize)]
struct MemoryUpdate {
    memory: String,
}

async fn patch_memory(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<MemoryUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    if req.memory.len() > MEMORY_TEXT_MAX_CHARS {
        return Err(ApiError::new(
            ErrorCode::ValidationFailed,
            format!("Memory exceeds {MEMORY_TEXT_MAX_CHARS} characters"),
        ));
    }
    crate::db::users::update_memory(&state.db, &c.sub, &req.memory)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(serde_json::json!({ "message": "Memory updated" })))
}

// -- Sessions --

async fn list_sessions(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<crate::db::sessions::Session>>, ApiError> {
    let c = claims(&headers, &state)?;
    let sessions = crate::db::sessions::list_by_user(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(sessions))
}

async fn delete_session(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let session = crate::db::sessions::find_by_id(&state.db, &session_id)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Session not found"))?;
    if session.user_id != c.sub {
        return Err(ApiError::new(ErrorCode::Forbidden, "Not your session"));
    }
    crate::db::sessions::delete_session(&state.db, &session_id)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    state.sessions.remove(&session_id);
    Ok(Json(serde_json::json!({ "message": "Session deleted" })))
}

#[derive(Deserialize)]
struct MessageQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn get_messages(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(q): Query<MessageQuery>,
) -> Result<Json<Vec<crate::db::messages::Message>>, ApiError> {
    let c = claims(&headers, &state)?;
    let session = crate::db::sessions::find_by_id(&state.db, &session_id)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Session not found"))?;
    if session.user_id != c.sub {
        return Err(ApiError::new(ErrorCode::Forbidden, "Not your session"));
    }
    let limit = q.limit.unwrap_or(50).min(500);
    let offset = q.offset.unwrap_or(0);
    let msgs = crate::db::messages::list_paginated(&state.db, &session_id, limit, offset)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(msgs))
}

// -- Files --

async fn upload_file(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let field = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::new(ErrorCode::ValidationFailed, format!("multipart: {e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::ValidationFailed, "No file provided"))?;
    let filename = field.file_name().unwrap_or("upload").to_string();
    let data = field
        .bytes()
        .await
        .map_err(|e| ApiError::new(ErrorCode::ValidationFailed, format!("read: {e}")))?;
    let file_id = crate::file_store::save_upload(&c.sub, &filename, &data).await?;
    Ok(Json(serde_json::json!({
        "file_id": file_id,
        "file_name": filename,
    })))
}

async fn download_file(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(file_id): Path<String>,
) -> Result<Response<Body>, ApiError> {
    let c = claims(&headers, &state)?;
    let (data, filename) = crate::file_store::load_file(&c.sub, &file_id).await?;
    let mime = nexus_common::mime::detect_mime_from_extension(&filename)
        .unwrap_or("application/octet-stream");
    Ok(Response::builder()
        .header("Content-Type", mime)
        .header(
            "Content-Disposition",
            format!("attachment; filename=\"{filename}\""),
        )
        .header("X-Content-Type-Options", "nosniff")
        .body(Body::from(data))
        .unwrap())
}

pub fn api_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/user/profile", get(get_profile))
        .route("/api/user/soul", get(get_soul).patch(patch_soul))
        .route("/api/user/memory", get(get_memory).patch(patch_memory))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{session_id}", delete(delete_session))
        .route("/api/sessions/{session_id}/messages", get(get_messages))
        .route("/api/files", post(upload_file))
        .route("/api/files/{file_id}", get(download_file))
}
