/// 职责边界：
/// 1. 专门为 Vue WebUI 提供标准的 HTTP REST API。
/// 2. 负责非对话类的 CRUD 操作。例如：拉取历史会话列表、重命名会话、拉取所有向量记忆文档、查询在线设备和可用工具等。
/// 3. 直接调用 `db.rs` 和 `state.rs`，【绝对不与消息总线 bus 交互】。
///
/// 参考 nanobot：
/// - 替代 `nanobot/session/manager.py` 中的 `list_sessions` 等文件查询方法，将其转化为 JSON API 接口。

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

/// Max upload size: 25 MB
const MAX_UPLOAD_SIZE: usize = 25 * 1024 * 1024;

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
    let devices = state.devices.read().await;
    let user_devices: Vec<serde_json::Value> = devices
        .iter()
        .filter(|(_, dev)| dev.user_id == claims.sub)
        .map(|(key, dev)| {
            let masked_key = if key.len() > 12 {
                format!("{}...{}", &key[..8], &key[key.len() - 4..])
            } else {
                "****".to_string()
            };
            serde_json::json!({
                "device_key": masked_key,
                "device_name": dev.device_name,
                "tools_count": dev.tools.len(),
                "last_seen_secs_ago": dev.last_seen.elapsed().as_secs(),
            })
        })
        .collect();
    Json(user_devices).into_response()
}

// ============================================================================
// GET /api/memories
// ============================================================================

pub async fn list_memories(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<PaginationParams>,
) -> Response {
    let limit = params.limit.unwrap_or(20).min(100);
    let offset = params.offset.unwrap_or(0);
    match db::list_memory_chunks(&state.db, &claims.sub, limit, offset).await {
        Ok(chunks) => Json(chunks).into_response(),
        Err(e) => {
            tracing::error!("list_memories error: {e}");
            ApiError::new(ErrorCode::InternalError, "failed to list memories").into_response()
        }
    }
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

        if bytes.len() > MAX_UPLOAD_SIZE {
            return ApiError::new(ErrorCode::ValidationFailed, "file exceeds 25MB limit").into_response();
        }

        let file_id = uuid::Uuid::new_v4().to_string();
        let dir = format!("/tmp/nexus-uploads/{}", claims.sub);
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            tracing::error!("upload_file: failed to create dir {dir}: {e}");
            return ApiError::new(ErrorCode::InternalError, "failed to create upload directory").into_response();
        }

        let stored_name = format!("{}_{}", file_id, file_name);
        let file_path = format!("{}/{}", dir, stored_name);

        if let Err(e) = tokio::fs::write(&file_path, &bytes).await {
            tracing::error!("upload_file: failed to write file: {e}");
            return ApiError::new(ErrorCode::InternalError, "failed to save file").into_response();
        }

        return Json(serde_json::json!({
            "file_id": file_id,
            "file_name": file_name,
        }))
        .into_response();
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
    // Search the requesting user's upload directory first
    let user_dir = format!("/tmp/nexus-uploads/{}", claims.sub);
    if let Some(resp) = try_serve_file_from_dir(&user_dir, &file_id).await {
        return resp;
    }

    // Also search /tmp/nexus-media/ for agent-generated files (e.g. send_file results)
    if let Some(resp) = try_serve_file_from_dir("/tmp/nexus-media", &file_id).await {
        return resp;
    }

    ApiError::new(ErrorCode::NotFound, "file not found").into_response()
}

/// Search a directory for a file whose name starts with `file_id` and serve it.
async fn try_serve_file_from_dir(dir: &str, file_id: &str) -> Option<Response> {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return None,
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(file_id) {
            let path = entry.path();
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => b,
                Err(_) => return None,
            };
            let content_type = mime_from_filename(&name);
            let headers = [
                (axum::http::header::CONTENT_TYPE, content_type),
                (axum::http::header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", name)),
                (axum::http::header::HeaderName::from_static("x-content-type-options"), "nosniff".to_string()),
            ];
            return Some((headers, bytes).into_response());
        }
    }
    None
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
