/// Responsibility boundary:
/// Client startup entry point, completing initialization in three phases before entering the main loop.
///
/// [Phase 1: Connect]
/// 1. Load ClientConfig via config.rs (server address, device_id, auth credentials, etc.).
/// 2. Establish a WebSocket connection to the server via session.rs (path: /ws?device_id=...).
/// 3. After connection, the server initiates login (ServerToClient::RequireLogin);
///    the client submits credentials, and the server binds the device to the corresponding UserId.
///
/// [Phase 2: Discovery & Registration]
/// 4. session.rs calls discovery.rs to scan local built-in tools (e.g. shell).
/// 5. mcp_client.rs connects to and scans all external MCP servers for tool schemas.
/// 6. Built-in and MCP tool schemas are aggregated and sent via
///    ClientToServer::RegisterTools to the server, completing tool registration.
///    On reconnect, session.rs repeats this step.
///
/// [Phase 3: Main Loop]
/// 7. session.rs starts a heartbeat loop, periodically sending ClientToServer::Heartbeat
///    (with tools_hash for the server to detect tool changes).
/// 8. The main receive loop listens for ServerToClient messages,
///    dispatches ExecuteToolRequest to executor.rs,
///    and returns results as ClientToServer::ToolExecutionResult.

mod config;
mod discovery;
mod env;
mod executor;
mod guardrails;
mod mcp_client;
mod sandbox;
mod session;
pub mod tools;

use nexus_common::protocol::{ClientToServer, FileDownloadResponse, FileUploadRequest, FileUploadResponse, FsPolicy, McpServerEntry, ServerToClient};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config = config::load_config();

    // Ensure workspace directory exists
    let workspace = env::get_workspace_root();
    if let Err(e) = std::fs::create_dir_all(&workspace) {
        warn!("failed to create workspace directory '{}': {}", workspace.display(), e);
    }

    let fs_policy = Arc::new(RwLock::new(FsPolicy::default()));
    let mcp_config: Arc<RwLock<Vec<McpServerEntry>>> = Arc::new(RwLock::new(Vec::new()));
    let mut session = session::connect_and_loop(config, fs_policy.clone(), mcp_config).await;

    info!("nexus-client started, waiting for server messages...");

    while let Some(message) = session.recv().await {
        match &message {
            ServerToClient::ExecuteToolRequest(req) => {
                info!("received ExecuteToolRequest: tool={}", req.tool_name);
                let result = executor::execute_tool_request(req.clone(), &fs_policy).await;
                let response = ClientToServer::ToolExecutionResult(result);
                if let Err(e) = session.send(response).await {
                    warn!("failed to send ToolExecutionResult: {}", e);
                }
            }
            ServerToClient::FileUploadRequest(req) => {
                info!("received FileUploadRequest: path={}", req.file_path);
                let response = handle_file_upload_request(req, &fs_policy).await;
                let msg = ClientToServer::FileUploadResponse(response);
                if let Err(e) = session.send(msg).await {
                    warn!("failed to send FileUploadResponse: {}", e);
                }
            }
            ServerToClient::FileDownloadRequest { request_id, file_name, content_base64, destination_path } => {
                info!("received FileDownloadRequest: file={}, dest={}", file_name, destination_path);
                let response = handle_file_download_request(
                    &request_id, &file_name, &content_base64, &destination_path, &fs_policy,
                ).await;
                let msg = ClientToServer::FileDownloadResponse(response);
                if let Err(e) = session.send(msg).await {
                    warn!("failed to send FileDownloadResponse: {}", e);
                }
            }
            ServerToClient::RequireLogin {
                message } => {
                warn!("unexpected RequireLogin during main loop: {}", message);
            }
            ServerToClient::LoginSuccess { user_id, device_name, .. } => {
                info!("unexpected LoginSuccess during main loop: user_id={}, device_name={}", user_id, device_name);
            }
            ServerToClient::LoginFailed { reason } => {
                warn!("unexpected LoginFailed during main loop: {}", reason);
            }
            ServerToClient::HeartbeatAck { .. } => {
                // Handled in session.rs message loop; should not reach here
            }
        }
    }

    warn!("session inbound channel closed");
}

async fn handle_file_upload_request(
    req: &FileUploadRequest,
    fs_policy: &Arc<RwLock<FsPolicy>>,
) -> FileUploadResponse {
    use base64::Engine;

    // Validate and resolve path against FsPolicy before any file access
    let resolved_path = {
        let policy = fs_policy.read().await;
        match crate::env::sanitize_path_with_policy(&req.file_path, crate::env::FsOp::Read, &*policy) {
            Ok(p) => p,
            Err(e) => {
                return FileUploadResponse {
                    request_id: req.request_id.clone(),
                    file_name: String::new(),
                    content_base64: String::new(),
                    mime_type: None,
                    error: Some(format!("file access denied by device policy: {}", e)),
                };
            }
        }
    };

    let file_name = resolved_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let make_error = |error: String| FileUploadResponse {
        request_id: req.request_id.clone(),
        file_name: file_name.clone(),
        content_base64: String::new(),
        mime_type: None,
        error: Some(error),
    };

    // Check file size (max 25MB for Discord)
    match tokio::fs::metadata(&resolved_path).await {
        Ok(meta) if meta.len() > 25 * 1024 * 1024 => {
            return make_error(format!(
                "File too large: {} bytes (max 25MB)",
                meta.len()
            ));
        }
        Err(e) => {
            let msg = match e.kind() {
                std::io::ErrorKind::NotFound => format!("File not found: {}", resolved_path.display()),
                std::io::ErrorKind::PermissionDenied => format!("Permission denied: {}", resolved_path.display()),
                _ => format!("Failed to read file metadata: {}", e),
            };
            return make_error(msg);
        }
        _ => {}
    }

    // Read file using the resolved (validated) path
    let bytes = match tokio::fs::read(&resolved_path).await {
        Ok(b) => b,
        Err(e) => {
            return make_error(format!("Failed to read file: {}", e));
        }
    };

    let content_base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let mime_type = nexus_common::mime::detect_mime_from_extension(&file_name).map(|s| s.to_string());

    FileUploadResponse {
        request_id: req.request_id.clone(),
        file_name,
        content_base64,
        mime_type,
        error: None,
    }
}

async fn handle_file_download_request(
    request_id: &str,
    file_name: &str,
    content_base64: &str,
    destination_path: &str,
    fs_policy: &Arc<RwLock<FsPolicy>>,
) -> FileDownloadResponse {
    use base64::Engine;

    // Determine the target path
    let target = if destination_path.is_empty() {
        let workspace = crate::env::get_workspace_root();
        workspace.join(file_name).to_string_lossy().to_string()
    } else {
        destination_path.to_string()
    };

    // Validate path against FsPolicy (write operation)
    let resolved_path = {
        let policy = fs_policy.read().await;
        match crate::env::sanitize_path_with_policy(&target, crate::env::FsOp::Write, &*policy) {
            Ok(p) => p,
            Err(e) => {
                return FileDownloadResponse {
                    request_id: request_id.to_string(),
                    error: Some(format!("file write denied by device policy: {}", e)),
                };
            }
        }
    };

    // Base64-decode content
    let bytes = match base64::engine::general_purpose::STANDARD.decode(content_base64) {
        Ok(b) => b,
        Err(e) => {
            return FileDownloadResponse {
                request_id: request_id.to_string(),
                error: Some(format!("base64 decode error: {}", e)),
            };
        }
    };

    // Ensure parent directory exists
    if let Some(parent) = resolved_path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            return FileDownloadResponse {
                request_id: request_id.to_string(),
                error: Some(format!("failed to create directory: {}", e)),
            };
        }
    }

    // Write file to disk
    if let Err(e) = tokio::fs::write(&resolved_path, &bytes).await {
        return FileDownloadResponse {
            request_id: request_id.to_string(),
            error: Some(format!("failed to write file: {}", e)),
        };
    }

    info!("file downloaded to: {}", resolved_path.display());
    FileDownloadResponse {
        request_id: request_id.to_string(),
        error: None,
    }
}

