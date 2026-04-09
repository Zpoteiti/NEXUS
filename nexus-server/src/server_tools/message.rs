//! message server tool: send content to a channel with optional media from a device.

use crate::server_tools::ToolContext;
use crate::state::AppState;
use serde_json::Value;
use std::sync::Arc;

pub async fn message_tool(state: &Arc<AppState>, ctx: &ToolContext, args: &Value) -> (i32, String) {
    let content = match args.get("content").and_then(Value::as_str) {
        Some(c) => c.to_string(),
        None => return (1, "Missing required parameter: content".into()),
    };
    let channel = match args.get("channel").and_then(Value::as_str) {
        Some(c) => c.to_string(),
        None => return (1, "Missing required parameter: channel".into()),
    };
    let chat_id = args
        .get("chat_id")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .or_else(|| ctx.chat_id.clone());

    let media_paths: Vec<String> = args
        .get("media")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let from_device = args.get("from_device").and_then(Value::as_str);

    // If media present and from_device specified, pull files from device
    let mut media_urls = Vec::new();
    if !media_paths.is_empty() {
        if let Some(device_name) = from_device {
            for path in &media_paths {
                // Send FileRequest to device, await FileResponse
                let device_key = AppState::device_key(&ctx.user_id, device_name);
                if let Some(conn) = state.devices.get(&device_key) {
                    let request_id = uuid::Uuid::new_v4().to_string();
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    state
                        .pending
                        .entry(device_key.clone())
                        .or_default()
                        .insert(request_id.clone(), tx);

                    let msg = nexus_common::protocol::ServerToClient::FileRequest {
                        request_id: request_id.clone(),
                        path: path.clone(),
                    };
                    let json = serde_json::to_string(&msg).unwrap();
                    {
                        let mut sink = conn.sink.lock().await;
                        if let Err(e) = futures_util::SinkExt::send(
                            &mut *sink,
                            axum::extract::ws::Message::Text(json.into()),
                        )
                        .await
                        {
                            return (1, format!("Failed to request file from {device_name}: {e}"));
                        }
                    }
                    drop(conn);

                    match tokio::time::timeout(std::time::Duration::from_secs(60), rx).await {
                        Ok(Ok(result)) => {
                            if result.exit_code == 0 {
                                // Parse base64 content, save to server
                                if let Ok(file_data) = serde_json::from_str::<Value>(&result.output)
                                {
                                    if let Some(b64) =
                                        file_data.get("content_base64").and_then(Value::as_str)
                                    {
                                        use base64::Engine;
                                        if let Ok(bytes) =
                                            base64::engine::general_purpose::STANDARD.decode(b64)
                                        {
                                            let filename = std::path::Path::new(path)
                                                .file_name()
                                                .unwrap_or_default()
                                                .to_string_lossy()
                                                .to_string();
                                            if let Ok(file_id) = crate::file_store::save_upload(
                                                &ctx.user_id,
                                                &filename,
                                                &bytes,
                                            )
                                            .await
                                            {
                                                media_urls.push(format!("/api/files/{file_id}"));
                                            }
                                        }
                                    }
                                }
                            } else {
                                return (1, format!("File request failed: {}", result.output));
                            }
                        }
                        Ok(Err(_)) => {
                            return (1, format!("Device {device_name} disconnected"));
                        }
                        Err(_) => return (1, "File request timed out".into()),
                    }
                } else {
                    return (1, format!("Device '{device_name}' is offline"));
                }
            }
        }
    }

    // Publish OutboundEvent
    let _ = state
        .outbound_tx
        .send(crate::bus::OutboundEvent {
            channel,
            chat_id,
            session_id: ctx.session_id.clone(),
            user_id: ctx.user_id.clone(),
            content,
            media: media_urls,
            is_progress: false,
            metadata: Default::default(),
        })
        .await;

    (0, "Message sent.".into())
}
