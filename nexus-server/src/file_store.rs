//! Centralized file storage for uploads, media, and temp files.

use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::info;

const UPLOAD_DIR: &str = "/tmp/nexus-uploads";
const MEDIA_DIR: &str = "/tmp/nexus-media";
const MAX_FILE_SIZE: usize = 25 * 1024 * 1024; // 25MB

/// Get the upload directory for a user, creating it if needed.
pub async fn user_upload_dir(user_id: &str) -> PathBuf {
    let dir = PathBuf::from(UPLOAD_DIR).join(sanitize(user_id));
    fs::create_dir_all(&dir).await.ok();
    dir
}

/// Get the shared media directory, creating it if needed.
pub async fn media_dir() -> PathBuf {
    let dir = PathBuf::from(MEDIA_DIR);
    fs::create_dir_all(&dir).await.ok();
    dir
}

/// Save an uploaded file. Returns (file_id, full_path).
pub async fn save_upload(user_id: &str, filename: &str, data: &[u8]) -> Result<(String, PathBuf), String> {
    if data.len() > MAX_FILE_SIZE {
        return Err(format!("file too large: {}MB (max {}MB)", data.len() / 1024 / 1024, MAX_FILE_SIZE / 1024 / 1024));
    }
    let file_id = uuid::Uuid::new_v4().to_string();
    let safe_name = format!("{}_{}", file_id, sanitize(filename));
    let dir = user_upload_dir(user_id).await;
    let path = dir.join(&safe_name);
    fs::write(&path, data).await.map_err(|e| format!("write failed: {}", e))?;
    Ok((file_id, path))
}

/// Save media (from send_file tool). Returns full path.
pub async fn save_media(filename: &str, data: &[u8]) -> Result<PathBuf, String> {
    let dir = media_dir().await;
    let safe_name = format!("{}_{}", uuid::Uuid::new_v4(), sanitize(filename));
    let path = dir.join(&safe_name);
    fs::write(&path, data).await.map_err(|e| format!("write failed: {}", e))?;
    Ok(path)
}

/// Find a file by file_id prefix in a directory.
pub async fn find_file_by_id(dir: &Path, file_id: &str) -> Option<PathBuf> {
    let mut entries = fs::read_dir(dir).await.ok()?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.file_name().to_string_lossy().starts_with(file_id) {
            return Some(entry.path());
        }
    }
    None
}

/// Find a file for download -- searches user uploads then shared media.
pub async fn find_download(user_id: &str, file_id: &str) -> Option<PathBuf> {
    let user_dir = user_upload_dir(user_id).await;
    if let Some(p) = find_file_by_id(&user_dir, file_id).await {
        return Some(p);
    }
    let media = media_dir().await;
    find_file_by_id(&media, file_id).await
}

/// Convert a media file path to a download URL for the browser.
pub fn path_to_download_url(path: &str) -> Option<String> {
    let filename = path.split('/').last()?;
    // Extract UUID from the filename (format: uuid_originalname)
    let uuid_part = filename.split('_').next()?;
    // Validate it looks like a UUID (36 chars with hyphens)
    if uuid_part.len() == 36 && uuid_part.chars().filter(|c| *c == '-').count() == 4 {
        Some(format!("/api/files/{}", uuid_part))
    } else {
        None
    }
}

/// Clean up files older than TTL.
pub async fn cleanup_old_files(max_age_secs: u64) {
    let now = std::time::SystemTime::now();
    for dir in &[UPLOAD_DIR, MEDIA_DIR] {
        if let Ok(mut entries) = fs::read_dir(dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.is_dir() {
                    // Recurse into user subdirectories
                    cleanup_dir(&path, now, max_age_secs).await;
                } else {
                    maybe_delete(&path, now, max_age_secs).await;
                }
            }
        }
    }
}

async fn cleanup_dir(dir: &Path, now: std::time::SystemTime, max_age_secs: u64) {
    if let Ok(mut entries) = fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            maybe_delete(&entry.path(), now, max_age_secs).await;
        }
    }
}

async fn maybe_delete(path: &Path, now: std::time::SystemTime, max_age_secs: u64) {
    if let Ok(meta) = fs::metadata(path).await {
        if let Ok(modified) = meta.modified() {
            if let Ok(age) = now.duration_since(modified) {
                if age.as_secs() > max_age_secs {
                    let _ = fs::remove_file(path).await;
                    info!("file_store: cleaned up old file: {}", path.display());
                }
            }
        }
    }
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| match c {
        '/' | '\\' | '\0' => '_',
        '.' if s.contains("..") => '_',
        _ => c,
    }).collect()
}
