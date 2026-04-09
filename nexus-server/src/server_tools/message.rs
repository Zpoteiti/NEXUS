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

    // Pull media files from device, save to server, collect URLs
    let mut media_urls = Vec::new();
    if !media_paths.is_empty() {
        if let Some(device_name) = from_device {
            for path in &media_paths {
                // Reuse shared file request logic from file_transfer
                match super::file_transfer::request_file_from_device(
                    state,
                    &ctx.user_id,
                    device_name,
                    path,
                )
                .await
                {
                    Ok((bytes, filename)) => {
                        match crate::file_store::save_upload(&ctx.user_id, &filename, &bytes).await
                        {
                            Ok(file_id) => media_urls.push(format!("/api/files/{file_id}")),
                            Err(e) => return (1, format!("Save media failed: {}", e.message)),
                        }
                    }
                    Err(e) => return (1, e),
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
