//! file_transfer server tool: relay files between devices.

use crate::state::AppState;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

pub async fn file_transfer(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let from_device = match args.get("from_device").and_then(Value::as_str) {
        Some(d) => d,
        None => return (1, "Missing required parameter: from_device".into()),
    };
    let to_device = match args.get("to_device").and_then(Value::as_str) {
        Some(d) => d,
        None => return (1, "Missing required parameter: to_device".into()),
    };
    let file_path = match args.get("file_path").and_then(Value::as_str) {
        Some(p) => p,
        None => return (1, "Missing required parameter: file_path".into()),
    };

    // Step 1: Get file content from source
    let (content_bytes, filename) = if from_device == "server" {
        // Read from server filesystem (per-user isolated paths only)
        match read_server_file(state, user_id, file_path).await {
            Ok(data) => data,
            Err(e) => return (1, e),
        }
    } else {
        // Request from client device
        match request_file_from_device(state, user_id, from_device, file_path).await {
            Ok(data) => data,
            Err(e) => return (1, e),
        }
    };

    // Step 2: Send file to destination
    if to_device == "server" {
        // Save to server upload dir
        let fname = Path::new(file_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        match crate::file_store::save_upload(user_id, &fname, &content_bytes).await {
            Ok(file_id) => (0, format!("File saved to server: /api/files/{file_id}")),
            Err(e) => (1, format!("Save to server failed: {}", e.message)),
        }
    } else {
        // Send to client device
        match send_file_to_device(state, user_id, to_device, &filename, &content_bytes).await {
            Ok(()) => (0, format!("File transferred: {from_device} -> {to_device}")),
            Err(e) => (1, e),
        }
    }
}

/// Read a file from server filesystem. Restricted to user's upload + skills dirs.
async fn read_server_file(
    state: &AppState,
    user_id: &str,
    file_path: &str,
) -> Result<(Vec<u8>, String), String> {
    // Validate path (per-user isolation)
    let canonical = tokio::fs::canonicalize(file_path)
        .await
        .map_err(|e| format!("Path not found: {e}"))?;
    let canonical_str = canonical.to_string_lossy();

    let upload_dir = crate::file_store::user_upload_dir(user_id);
    let upload_prefix = upload_dir.to_string_lossy().to_string();
    let skills_prefix = format!("{}/{user_id}/", state.config.skills_dir);

    if !canonical_str.starts_with(&upload_prefix) && !canonical_str.starts_with(&skills_prefix) {
        return Err(format!(
            "Access denied: server file path must be within your uploads or skills directory"
        ));
    }

    let data = tokio::fs::read(&canonical)
        .await
        .map_err(|e| format!("Read file: {e}"))?;
    let filename = canonical
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    Ok((data, filename))
}

/// Request a file from a client device via FileRequest protocol.
pub async fn request_file_from_device(
    state: &AppState,
    user_id: &str,
    device_name: &str,
    file_path: &str,
) -> Result<(Vec<u8>, String), String> {
    let device_key = AppState::device_key(user_id, device_name);
    let conn = state
        .devices
        .get(&device_key)
        .ok_or_else(|| format!("Device '{device_name}' is offline"))?;

    let request_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = tokio::sync::oneshot::channel();
    state
        .pending
        .entry(device_key.clone())
        .or_default()
        .insert(request_id.clone(), tx);

    let msg = nexus_common::protocol::ServerToClient::FileRequest {
        request_id: request_id.clone(),
        path: file_path.to_string(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    {
        let mut sink = conn.sink.lock().await;
        futures_util::SinkExt::send(&mut *sink, axum::extract::ws::Message::Text(json.into()))
            .await
            .map_err(|e| format!("Send FileRequest: {e}"))?;
    }
    drop(conn);

    let result = tokio::time::timeout(std::time::Duration::from_secs(60), rx)
        .await
        .map_err(|_| "File request timed out".to_string())?
        .map_err(|_| format!("Device '{device_name}' disconnected"))?;

    if result.exit_code != 0 {
        return Err(format!("File request failed: {}", result.output));
    }

    let file_data: Value =
        serde_json::from_str(&result.output).map_err(|e| format!("Parse response: {e}"))?;
    let b64 = file_data
        .get("content_base64")
        .and_then(Value::as_str)
        .ok_or("Missing content_base64 in response")?;

    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("Decode base64: {e}"))?;

    let filename = Path::new(file_path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    Ok((bytes, filename))
}

/// Send a file to a client device via FileSend protocol.
async fn send_file_to_device(
    state: &AppState,
    user_id: &str,
    device_name: &str,
    filename: &str,
    content: &[u8],
) -> Result<(), String> {
    let device_key = AppState::device_key(user_id, device_name);
    let conn = state
        .devices
        .get(&device_key)
        .ok_or_else(|| format!("Device '{device_name}' is offline"))?;

    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(content);
    let request_id = uuid::Uuid::new_v4().to_string();

    let (tx, rx) = tokio::sync::oneshot::channel();
    state
        .pending
        .entry(device_key.clone())
        .or_default()
        .insert(request_id.clone(), tx);

    let msg = nexus_common::protocol::ServerToClient::FileSend {
        request_id: request_id.clone(),
        filename: filename.to_string(),
        content_base64: b64,
        destination: filename.to_string(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    {
        let mut sink = conn.sink.lock().await;
        futures_util::SinkExt::send(&mut *sink, axum::extract::ws::Message::Text(json.into()))
            .await
            .map_err(|e| format!("Send FileSend: {e}"))?;
    }
    drop(conn);

    let result = tokio::time::timeout(std::time::Duration::from_secs(60), rx)
        .await
        .map_err(|_| "File send timed out".to_string())?
        .map_err(|_| format!("Device '{device_name}' disconnected"))?;

    if result.exit_code != 0 {
        return Err(format!("File send failed: {}", result.output));
    }

    Ok(())
}
