use async_trait::async_trait;
use nexus_common::error::{ErrorCode, NexusError};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

use super::{ServerTool, ServerToolResult};
use crate::state::AppState;

pub struct DownloadToDeviceTool;

#[async_trait]
impl ServerTool for DownloadToDeviceTool {
    fn name(&self) -> &str { "download_to_device" }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "download_to_device",
                "description": "Transfer a file from the server to a client device. Can download user uploads or skill files.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "source": {
                            "type": "string",
                            "enum": ["upload", "skill"],
                            "description": "File source: 'upload' for user-uploaded files, 'skill' for skill-bundled files."
                        },
                        "file_name": {
                            "type": "string",
                            "description": "For uploads: the filename. For skills: 'skill_name/path/to/file' (e.g., 'weather/scripts/fetch.py')."
                        },
                        "device_name": {
                            "type": "string",
                            "description": "The target device to download the file to."
                        },
                        "destination_path": {
                            "type": "string",
                            "description": "Where to save on the device. Defaults to workspace root."
                        }
                    },
                    "required": ["source", "file_name", "device_name"]
                }
            }
        })
    }

    async fn execute(
        &self,
        state: &Arc<AppState>,
        user_id: &str,
        _session_id: &str,
        arguments: Value,
        _event_channel: &str,
        _event_chat_id: &str,
    ) -> Result<ServerToolResult, NexusError> {
        let source = arguments.get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NexusError::new(ErrorCode::ToolInvalidParams, "download_to_device: missing source"))?
            .to_string();
        let file_name = arguments.get("file_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NexusError::new(ErrorCode::ToolInvalidParams, "download_to_device: missing file_name"))?
            .to_string();
        let device_name = arguments.get("device_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NexusError::new(ErrorCode::ToolInvalidParams, "download_to_device: missing device_name"))?
            .to_string();
        let destination_path = arguments.get("destination_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Resolve file bytes and actual file name based on source
        let (bytes, actual_file_name) = match source.as_str() {
            "upload" => {
                // Search user upload dir for the file (user isolation)
                let user_dir = crate::file_store::user_upload_dir(user_id).await;
                let upload_path = find_uploaded_file(&user_dir.to_string_lossy(), &file_name).await
                    .ok_or_else(|| NexusError::new(
                        ErrorCode::ExecutionFailed,
                        format!("File '{}' not found in uploads for this user", file_name),
                    ))?;

                let bytes = tokio::fs::read(&upload_path).await
                    .map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("failed to read file: {}", e)))?;

                let actual_name = std::path::Path::new(&upload_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| file_name.clone());

                (bytes, actual_name)
            }
            "skill" => {
                // Parse skill_name/relative_path
                let parts: Vec<&str> = file_name.splitn(2, '/').collect();
                if parts.len() != 2 {
                    return Err(NexusError::new(
                        ErrorCode::ToolInvalidParams,
                        "skill file_name must be 'skill_name/path' format (e.g., 'weather/scripts/fetch.py')",
                    ));
                }
                let (skill_name, relative_path) = (parts[0], parts[1]);

                // Security: no path traversal
                if relative_path.contains("..") || skill_name.contains("..") {
                    return Err(NexusError::new(ErrorCode::ToolBlocked, "path traversal not allowed"));
                }

                // Build path: {skills_dir}/{user_id}/{skill_name}/{relative_path}
                let skill_dir = PathBuf::from(&state.config.skills_dir)
                    .join(user_id)
                    .join(skill_name);
                let file_path = skill_dir.join(relative_path);

                // Security: verify resolved path stays within skill_dir
                let canonical = file_path.canonicalize()
                    .map_err(|_| NexusError::new(ErrorCode::ExecutionFailed, format!("file not found: {}", file_name)))?;
                let canonical_skill_dir = skill_dir.canonicalize()
                    .map_err(|_| NexusError::new(ErrorCode::ExecutionFailed, format!("skill '{}' not found", skill_name)))?;
                if !canonical.starts_with(&canonical_skill_dir) {
                    return Err(NexusError::new(ErrorCode::ToolBlocked, "access denied: path escapes skill directory"));
                }

                let bytes = tokio::fs::read(&canonical).await
                    .map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("failed to read skill file: {}", e)))?;

                let actual_name = canonical.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| file_name.clone());

                (bytes, actual_name)
            }
            other => {
                return Err(NexusError::new(
                    ErrorCode::ToolInvalidParams,
                    format!("download_to_device: invalid source '{}', must be 'upload' or 'skill'", other),
                ));
            }
        };

        // Check file size (max 25MB)
        if bytes.len() > 25 * 1024 * 1024 {
            return Err(NexusError::new(
                ErrorCode::ExecutionFailed,
                format!("File too large: {} bytes (max 25MB)", bytes.len()),
            ));
        }

        // Base64-encode
        use base64::Engine;
        let content_base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

        let size_kb = bytes.len() / 1024;

        // Find device
        let device_key = {
            let by_user = state.devices_by_user.read().await;
            by_user.get(user_id)
                .and_then(|devices| devices.get(&device_name).cloned())
                .ok_or_else(|| NexusError::new(ErrorCode::DeviceNotFound, format!("device '{}' not found", device_name)))?
        };

        let ws_tx = {
            let devices = state.devices.read().await;
            devices.get(&device_key)
                .map(|d| d.ws_tx.clone())
                .ok_or_else(|| NexusError::new(ErrorCode::DeviceOffline, format!("device '{}' not connected", device_name)))?
        };

        // 6. Send FileDownloadRequest via WebSocket
        let request_id = format!("{}:{}", device_key, uuid::Uuid::new_v4());
        let (tx, rx) = tokio::sync::oneshot::channel();
        state.file_download_pending.insert(request_id.clone(), tx);

        use nexus_common::protocol::ServerToClient;
        let msg = ServerToClient::FileDownloadRequest {
            request_id: request_id.clone(),
            file_name: actual_file_name.clone(),
            content_base64,
            destination_path,
        };
        let msg_text = serde_json::to_string(&msg)
            .map_err(|e| NexusError::new(ErrorCode::InternalError, format!("serialize error: {}", e)))?;
        ws_tx.send(axum::extract::ws::Message::Text(msg_text.into())).await
            .map_err(|e| NexusError::new(ErrorCode::ChannelError, format!("ws send error: {}", e)))?;

        // 7. Wait for response with 60s timeout
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            rx,
        ).await
            .map_err(|_| {
                state.file_download_pending.remove(&request_id);
                NexusError::new(ErrorCode::ToolTimeout, "file download timed out after 60s")
            })?
            .map_err(|_| NexusError::new(ErrorCode::ChannelError, "file download channel closed (device may have disconnected)"))?;

        if let Some(err) = response.error {
            return Err(NexusError::new(ErrorCode::ExecutionFailed, format!("file download failed on device: {}", err)));
        }

        Ok(ServerToolResult {
            output: format!("File '{}' ({} KB) successfully transferred to device '{}'.", actual_file_name, size_kb, device_name),
            media: Vec::new(),
        })
    }
}

/// Search the upload directory for a file matching the given name.
/// Files are stored as `{uuid_or_attachment_id}_{original_name}`, so we do a
/// partial match on the portion after the first underscore.
async fn find_uploaded_file(upload_dir: &str, file_name: &str) -> Option<String> {
    let mut entries = tokio::fs::read_dir(upload_dir).await.ok()?;
    let mut best_match: Option<String> = None;

    while let Ok(Some(entry)) = entries.next_entry().await {
        let entry_name = entry.file_name().to_string_lossy().to_string();
        // Exact match
        if entry_name == file_name {
            return Some(entry.path().to_string_lossy().to_string());
        }
        // Partial match: strip the prefix (everything before and including the first '_')
        if let Some(pos) = entry_name.find('_') {
            let suffix = &entry_name[pos + 1..];
            if suffix == file_name {
                best_match = Some(entry.path().to_string_lossy().to_string());
            }
        }
        // Also match if file_name is contained in entry_name
        if best_match.is_none() && entry_name.contains(file_name) {
            best_match = Some(entry.path().to_string_lossy().to_string());
        }
    }

    best_match
}
