use async_trait::async_trait;
use nexus_common::error::{ErrorCode, NexusError};
use serde_json::{json, Value};
use std::sync::Arc;

use super::{ServerTool, ServerToolResult};
use crate::state::AppState;

pub struct SendFileTool;

#[async_trait]
impl ServerTool for SendFileTool {
    fn name(&self) -> &str { "send_file" }

    fn schema(&self) -> Value {
        // send_file includes its own device_name parameter because it needs
        // to know which device to pull the file from.
        json!({
            "type": "function",
            "function": {
                "name": "send_file",
                "description": "Retrieve a file from a device and send it to the user. The file will be attached to the reply.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "device_name": {
                            "type": "string",
                            "description": "The device to retrieve the file from."
                        },
                        "file_path": {
                            "type": "string",
                            "description": "Path to the file on the device."
                        }
                    },
                    "required": ["device_name", "file_path"]
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
        let device_name = arguments.get("device_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NexusError::new(ErrorCode::ToolInvalidParams, "send_file: missing device_name"))?
            .to_string();
        let file_path = arguments.get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NexusError::new(ErrorCode::ToolInvalidParams, "send_file: missing file_path"))?
            .to_string();

        // Find the device
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

        // Create oneshot channel for file upload response
        let request_id = format!("{}:{}", device_key, uuid::Uuid::new_v4());
        let (tx, rx) = tokio::sync::oneshot::channel();
        state.file_upload_pending.insert(request_id.clone(), tx);

        // Send FileUploadRequest to client
        use nexus_common::protocol::{ServerToClient, FileUploadRequest};
        let msg = ServerToClient::FileUploadRequest(FileUploadRequest {
            request_id: request_id.clone(),
            file_path: file_path.clone(),
        });
        let msg_text = serde_json::to_string(&msg)
            .map_err(|e| NexusError::new(ErrorCode::InternalError, format!("serialize error: {}", e)))?;
        ws_tx.send(axum::extract::ws::Message::Text(msg_text.into())).await
            .map_err(|e| NexusError::new(ErrorCode::ChannelError, format!("ws send error: {}", e)))?;

        // Wait for response with timeout
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            rx,
        ).await
            .map_err(|_| NexusError::new(ErrorCode::ToolTimeout, "file upload timed out after 60s"))?
            .map_err(|_| NexusError::new(ErrorCode::ChannelError, "file upload channel closed (device may have disconnected)"))?;

        if let Some(err) = response.error {
            return Err(NexusError::new(ErrorCode::ExecutionFailed, format!("file upload failed: {}", err)));
        }

        // Decode base64 and save to temp
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(&response.content_base64)
            .map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("base64 decode error: {}", e)))?;

        let save_path = crate::file_store::save_media(&response.file_name, &bytes).await
            .map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, e))?;

        Ok(ServerToolResult {
            output: format!("File saved: {}", save_path.display()),
            media: vec![save_path.to_string_lossy().to_string()],
        })
    }
}
