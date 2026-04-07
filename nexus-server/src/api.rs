/// Responsibility boundary:
/// 1. Provides standard HTTP REST API endpoints for the Vue WebUI.
/// 2. Handles non-conversation CRUD operations (session listing, memory, devices, etc.).
/// 3. Calls `db.rs` and `state.rs` directly -- never interacts with the message bus.

use axum::{
    extract::{Multipart, Path, Query, State},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;

use nexus_common::error::{ApiError, ErrorCode};

use crate::auth::Claims;
use crate::db;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct PaginationParams {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ============================================================================
// GET /api/sessions/:id/messages
// ============================================================================

pub async fn get_session_messages(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(session_id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Response {
    let limit = params.limit.unwrap_or(50).min(500);
    let offset = params.offset.unwrap_or(0);
    match db::get_session_messages(&state.db, &session_id, &claims.sub, limit, offset).await {
        Ok(messages) => Json(messages).into_response(),
        Err(e) => {
            tracing::error!("get_session_messages error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to get messages").into_response()
        }
    }
}

// ============================================================================
// GET /api/user/profile
// ============================================================================

pub async fn get_user_profile(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Response {
    match db::get_user_profile(&state.db, &claims.sub).await {
        Ok(Some(profile)) => Json(profile).into_response(),
        Ok(None) => {
            ApiError::new(ErrorCode::NotFound, "user not found").into_response()
        }
        Err(e) => {
            tracing::error!("get_user_profile error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to get user profile").into_response()
        }
    }
}

// ============================================================================
// GET /api/devices
// ============================================================================

pub async fn list_devices(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Response {
    // 1. Query all registered devices from DB
    let registered = match db::list_user_devices(&state.db, &claims.sub).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("list_devices db error: {e}");
            return ApiError::new(ErrorCode::InternalError, "failed to list devices").into_response();
        }
    };

    // 2. Read live devices from in-memory state
    let live_devices = state.devices.read().await;

    // Build a lookup: device_name -> &DeviceState for this user's online devices
    let mut live_lookup = std::collections::HashMap::new();
    for (_, dev) in live_devices.iter() {
        if dev.user_id == claims.sub {
            live_lookup.insert(dev.device_name.as_str(), dev);
        }
    }

    // 3. Merge: for each registered device, enrich with live status
    let merged: Vec<serde_json::Value> = registered
        .iter()
        .map(|reg| {
            if reg.revoked {
                serde_json::json!({
                    "device_name": reg.device_name,
                    "status": "revoked",
                    "tools_count": 0,
                    "fs_policy": serde_json::Value::Null,
                })
            } else if let Some(live) = live_lookup.get(reg.device_name.as_str()) {
                serde_json::json!({
                    "device_name": reg.device_name,
                    "status": "online",
                    "last_seen_secs_ago": live.last_seen.elapsed().as_secs(),
                    "tools_count": live.tools.len(),
                    "fs_policy": reg.fs_policy,
                })
            } else {
                serde_json::json!({
                    "device_name": reg.device_name,
                    "status": "offline",
                    "tools_count": 0,
                    "fs_policy": reg.fs_policy,
                })
            }
        })
        .collect();

    Json(merged).into_response()
}

// ============================================================================
// GET /api/user/soul
// ============================================================================

pub async fn get_soul(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Response {
    match db::get_user_soul(&state.db, &claims.sub).await {
        Ok(soul) => Json(serde_json::json!({ "soul": soul })).into_response(),
        Err(e) => {
            tracing::error!("get_soul error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to get soul").into_response()
        }
    }
}

// ============================================================================
// PATCH /api/user/soul
// ============================================================================

#[derive(Deserialize)]
pub struct UpdateSoulRequest {
    pub soul: String,
}

pub async fn update_soul(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<UpdateSoulRequest>,
) -> Response {
    match db::update_user_soul(&state.db, &claims.sub, &payload.soul).await {
        Ok(()) => Json(serde_json::json!({"message": "Soul updated"})).into_response(),
        Err(e) => {
            tracing::error!("update_soul error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to update soul").into_response()
        }
    }
}

// ============================================================================
// GET /api/user/preferences
// ============================================================================

pub async fn get_preferences(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Response {
    match db::get_user_preferences(&state.db, &claims.sub).await {
        Ok(prefs) => Json(serde_json::json!({ "preferences": prefs })).into_response(),
        Err(e) => {
            tracing::error!("get_preferences error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to get preferences").into_response()
        }
    }
}

// ============================================================================
// PATCH /api/user/preferences
// ============================================================================

#[derive(Deserialize)]
pub struct UpdatePreferencesRequest {
    pub preferences: serde_json::Value,
}

pub async fn update_preferences(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<UpdatePreferencesRequest>,
) -> Response {
    match db::update_user_preferences(&state.db, &claims.sub, &payload.preferences).await {
        Ok(()) => Json(serde_json::json!({"message": "Preferences updated"})).into_response(),
        Err(e) => {
            tracing::error!("update_preferences error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to update preferences").into_response()
        }
    }
}

// ============================================================================
// GET /api/admin/default-soul (admin only)
// ============================================================================

pub async fn get_default_soul(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Response {
    if !claims.is_admin {
        return ApiError::new(ErrorCode::Forbidden, "admin access required").into_response();
    }
    match db::get_system_config(&state.db, "default_soul").await {
        Ok(soul) => Json(serde_json::json!({ "default_soul": soul })).into_response(),
        Err(e) => {
            tracing::error!("get_default_soul error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to get default soul").into_response()
        }
    }
}

// ============================================================================
// PUT /api/admin/default-soul (admin only)
// ============================================================================

#[derive(Deserialize)]
pub struct SetDefaultSoulRequest {
    pub soul: String,
}

pub async fn set_default_soul(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<SetDefaultSoulRequest>,
) -> Response {
    if !claims.is_admin {
        return ApiError::new(ErrorCode::Forbidden, "admin access required").into_response();
    }
    match db::set_system_config(&state.db, "default_soul", &payload.soul).await {
        Ok(()) => Json(serde_json::json!({"message": "Default soul updated"})).into_response(),
        Err(e) => {
            tracing::error!("set_default_soul error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to set default soul").into_response()
        }
    }
}

// ============================================================================
// GET /api/user/memory
// ============================================================================

pub async fn get_memory(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Response {
    match db::get_user_memory(&state.db, &claims.sub).await {
        Ok(memory) => Json(serde_json::json!({ "memory": memory })).into_response(),
        Err(e) => {
            tracing::error!("get_memory error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to get memory").into_response()
        }
    }
}

// ============================================================================
// PATCH /api/user/memory
// ============================================================================

#[derive(Deserialize)]
pub struct UpdateMemoryRequest {
    pub memory: String,
}

/// Maximum memory size in characters (4K).
const MEMORY_MAX_CHARS: usize = 4096;

pub async fn update_memory(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<UpdateMemoryRequest>,
) -> Response {
    if payload.memory.len() > MEMORY_MAX_CHARS {
        return ApiError::new(
            ErrorCode::ValidationFailed,
            format!("Memory exceeds {} character limit ({} chars)", MEMORY_MAX_CHARS, payload.memory.len()),
        ).into_response();
    }
    match db::update_user_memory(&state.db, &claims.sub, &payload.memory).await {
        Ok(()) => Json(serde_json::json!({"message": "Memory updated"})).into_response(),
        Err(e) => {
            tracing::error!("update_memory error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to update memory").into_response()
        }
    }
}

// ============================================================================
// POST /api/files  (multipart upload)
// ============================================================================

pub async fn upload_file(
    Extension(claims): Extension<Claims>,
    mut multipart: Multipart,
) -> Response {
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        if name != "file" {
            continue;
        }

        let file_name = field
            .file_name()
            .unwrap_or("upload")
            .to_string();

        let bytes = match field.bytes().await {
            Ok(b) => b,
            Err(e) => {
                tracing::error!("upload_file: failed to read field bytes: {e}");
                return ApiError::new(ErrorCode::ValidationFailed, "failed to read upload data").into_response();
            }
        };

        match crate::file_store::save_upload(&claims.sub, &file_name, &bytes).await {
            Ok((file_id, _path)) => {
                return Json(serde_json::json!({
                    "file_id": file_id,
                    "file_name": file_name,
                }))
                .into_response();
            }
            Err(e) => {
                tracing::error!("upload_file: {e}");
                return ApiError::new(ErrorCode::InternalError, e).into_response();
            }
        }
    }

    ApiError::new(ErrorCode::ValidationFailed, "no 'file' field found in multipart data").into_response()
}

// ============================================================================
// GET /api/files/{file_id}  (download)
// ============================================================================

pub async fn download_file(
    Extension(claims): Extension<Claims>,
    Path(file_id): Path<String>,
) -> Response {
    // Validate file_id: reject path traversal and non-alphanumeric+hyphen characters
    if file_id.contains("..") || file_id.contains('/') || file_id.contains('\\')
        || !file_id.chars().all(|c| c.is_alphanumeric() || c == '-')
    {
        return ApiError::new(ErrorCode::ValidationFailed, "invalid file id").into_response();
    }

    // Search user uploads then shared media via file_store
    match crate::file_store::find_download(&claims.sub, &file_id).await {
        Some(path) => {
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => b,
                Err(_) => return ApiError::new(ErrorCode::InternalError, "failed to read file").into_response(),
            };
            let name = path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let content_type = mime_from_filename(&name);
            let headers = [
                (axum::http::header::CONTENT_TYPE, content_type),
                (axum::http::header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", name)),
                (axum::http::header::HeaderName::from_static("x-content-type-options"), "nosniff".to_string()),
            ];
            (headers, bytes).into_response()
        }
        None => ApiError::new(ErrorCode::NotFound, "file not found").into_response(),
    }
}

/// Guess MIME type from file extension.
fn mime_from_filename(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".pdf") {
        "application/pdf"
    } else if lower.ends_with(".json") {
        "application/json"
    } else if lower.ends_with(".txt") || lower.ends_with(".log") {
        "text/plain"
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        "text/html"
    } else if lower.ends_with(".mp4") {
        "video/mp4"
    } else if lower.ends_with(".mp3") {
        "audio/mpeg"
    } else {
        "application/octet-stream"
    }
    .to_string()
}
