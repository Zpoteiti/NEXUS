//! File upload/download with user-isolated paths. Hourly cleanup of old files.

use nexus_common::consts::{FILE_CLEANUP_AGE_HOURS, FILE_UPLOAD_MAX_BYTES};
use nexus_common::error::{ApiError, ErrorCode};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{info, warn};

const UPLOAD_BASE: &str = "/tmp/nexus-uploads";

pub fn user_upload_dir(user_id: &str) -> PathBuf {
    PathBuf::from(UPLOAD_BASE).join(user_id)
}

pub async fn save_upload(user_id: &str, filename: &str, data: &[u8]) -> Result<String, ApiError> {
    if data.len() > FILE_UPLOAD_MAX_BYTES {
        return Err(ApiError::new(
            ErrorCode::ValidationFailed,
            format!(
                "File exceeds {}MB limit",
                FILE_UPLOAD_MAX_BYTES / 1024 / 1024
            ),
        ));
    }
    let file_id = uuid::Uuid::new_v4().to_string();
    let dir = user_upload_dir(user_id);
    fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("mkdir: {e}")))?;

    let safe_name = sanitize_filename(filename);
    let path = dir.join(format!("{file_id}_{safe_name}"));
    fs::write(&path, data)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("write: {e}")))?;
    Ok(file_id)
}

pub async fn load_file(user_id: &str, file_id: &str) -> Result<(Vec<u8>, String), ApiError> {
    if file_id.contains("..") || file_id.contains('/') || file_id.contains('\\') {
        return Err(ApiError::new(
            ErrorCode::ValidationFailed,
            "Invalid file ID",
        ));
    }
    let dir = user_upload_dir(user_id);
    let mut entries = fs::read_dir(&dir)
        .await
        .map_err(|_| ApiError::new(ErrorCode::NotFound, "No files found"))?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(file_id) {
            let data = fs::read(entry.path())
                .await
                .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("read: {e}")))?;
            let original_name = name
                .strip_prefix(&format!("{file_id}_"))
                .unwrap_or(&name)
                .to_string();
            return Ok((data, original_name));
        }
    }
    Err(ApiError::new(ErrorCode::NotFound, "File not found"))
}

/// A resolved media item ready for channel delivery.
pub enum ResolvedMedia {
    /// File bytes + filename, fetched from the server file store.
    File { bytes: Vec<u8>, filename: String },
    /// Raw URL/path passed through as-is (e.g. direct URLs from future channels).
    Url(String),
}

/// Resolve media paths for channel delivery. Loads all `/api/files/{id}` entries
/// in parallel; passes through anything else as a raw URL.
pub async fn resolve_media(user_id: &str, media_paths: &[String]) -> Vec<ResolvedMedia> {
    let futures = media_paths.iter().map(|path| {
        let path = path.clone();
        let user_id = user_id.to_string();
        async move {
            if let Some(file_id) = path.strip_prefix("/api/files/") {
                match load_file(&user_id, file_id).await {
                    Ok((bytes, filename)) => ResolvedMedia::File { bytes, filename },
                    Err(e) => {
                        warn!("resolve_media: failed to load {file_id}: {}", e.message);
                        ResolvedMedia::Url(path)
                    }
                }
            } else {
                ResolvedMedia::Url(path)
            }
        }
    });
    futures_util::future::join_all(futures).await
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn spawn_cleanup_task() {
    tokio::spawn(async {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            if let Err(e) = cleanup_old_files().await {
                warn!("File cleanup error: {e}");
            }
        }
    });
}

async fn cleanup_old_files() -> Result<(), std::io::Error> {
    let base = Path::new(UPLOAD_BASE);
    if !base.exists() {
        return Ok(());
    }
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(FILE_CLEANUP_AGE_HOURS * 3600);
    let mut count = 0u32;
    let mut dirs = fs::read_dir(base).await?;
    while let Some(user_dir) = dirs.next_entry().await? {
        if !user_dir.file_type().await?.is_dir() {
            continue;
        }
        let mut files = fs::read_dir(user_dir.path()).await?;
        while let Some(file) = files.next_entry().await? {
            if let Ok(meta) = file.metadata().await {
                if let Ok(modified) = meta.modified() {
                    if modified < cutoff {
                        let _ = fs::remove_file(file.path()).await;
                        count += 1;
                    }
                }
            }
        }
    }
    if count > 0 {
        info!("Cleaned up {count} old files");
    }
    Ok(())
}
