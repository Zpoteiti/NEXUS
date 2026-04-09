//! Tool schema merging, device_name injection, and tool call routing.
//! Server tools have no device_name. Client + MCP tools get device_name enum.

use crate::state::AppState;
use futures_util::SinkExt;
use nexus_common::consts::TOOL_EXECUTION_TIMEOUT_SEC;
use nexus_common::protocol::{ExecuteToolRequest, ServerToClient, ToolExecutionResult};
use serde_json::Value;
use std::sync::Arc;
use tracing::warn;

/// Build merged tool schemas for a user. Uses cache if available.
pub fn build_tool_schemas(state: &AppState, user_id: &str) -> Vec<Value> {
    // Check cache first
    if let Some(cached) = state.tool_schema_cache.get(user_id) {
        return cached.clone();
    }

    let mut schemas = Vec::new();

    // 1. Server native tool schemas (no device_name)
    schemas.extend(crate::server_tools::tool_schemas());

    // 2. Collect device names and their tools
    let mut device_tools: Vec<(String, Vec<Value>)> = Vec::new();
    if let Some(keys) = state.devices_by_user.get(user_id) {
        for key in keys.value() {
            if let Some(conn) = state.devices.get(key) {
                device_tools.push((conn.device_name.clone(), conn.tools.clone()));
            }
        }
    }

    // 3. Build online device name list
    let device_names: Vec<String> = device_tools.iter().map(|(n, _)| n.clone()).collect();

    // 4. Merge client tools with device_name enum
    // Collect unique tool names across all devices
    let mut seen_tools: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for (device_name, tools) in &device_tools {
        for tool in tools {
            let name = tool
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                seen_tools
                    .entry(name)
                    .or_default()
                    .push(device_name.clone());
            }
        }
    }

    // For each unique tool, create schema with device_name enum
    for (device_name, tools) in &device_tools {
        for tool in tools {
            let name = tool
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            if name.is_empty() {
                continue;
            }

            // Skip if we already added this tool (from another device)
            if let Some(devices) = seen_tools.get(name) {
                if devices.first().map(|d| d.as_str()) != Some(device_name) {
                    continue; // Already added from first device
                }
            }

            let available_devices = seen_tools.get(name).cloned().unwrap_or_default();

            // TODO: Also include "server" if server MCP has this tool name
            let mut schema = tool.clone();
            inject_device_name_enum(&mut schema, &available_devices);
            schemas.push(schema);
        }
    }

    // Cache result
    state
        .tool_schema_cache
        .insert(user_id.to_string(), schemas.clone());

    schemas
}

/// Inject device_name enum into a tool schema's parameters.
fn inject_device_name_enum(schema: &mut Value, devices: &[String]) {
    if let Some(function) = schema.get_mut("function") {
        if let Some(parameters) = function.get_mut("parameters") {
            if let Some(properties) = parameters.get_mut("properties") {
                if let Some(obj) = properties.as_object_mut() {
                    obj.insert(
                        "device_name".to_string(),
                        serde_json::json!({
                            "type": "string",
                            "enum": devices,
                            "description": "Target device to execute this tool on"
                        }),
                    );
                }
            }
            // Add device_name to required
            if let Some(required) = parameters.get_mut("required") {
                if let Some(arr) = required.as_array_mut() {
                    arr.push(Value::String("device_name".into()));
                }
            }
        }
    }
}

/// Route a tool call to the correct device. Returns the tool execution result.
pub async fn route_to_device(
    state: &Arc<AppState>,
    user_id: &str,
    device_name: &str,
    tool_name: &str,
    arguments: Value,
) -> ToolExecutionResult {
    let device_key = AppState::device_key(user_id, device_name);

    // Get device connection
    let conn = match state.devices.get(&device_key) {
        Some(c) => c,
        None => {
            return ToolExecutionResult {
                request_id: String::new(),
                exit_code: 1,
                output: format!("Device '{device_name}' is offline"),
            };
        }
    };

    let request_id = uuid::Uuid::new_v4().to_string();

    // Create oneshot channel for response
    let (tx, rx) = tokio::sync::oneshot::channel();
    state
        .pending
        .entry(device_key.clone())
        .or_default()
        .insert(request_id.clone(), tx);

    // Send ExecuteToolRequest to device
    let req = ServerToClient::ExecuteToolRequest(ExecuteToolRequest {
        request_id: request_id.clone(),
        tool_name: tool_name.to_string(),
        arguments,
    });
    let json = serde_json::to_string(&req).unwrap();
    {
        let mut sink = conn.sink.lock().await;
        if let Err(e) = sink
            .send(axum::extract::ws::Message::Text(json.into()))
            .await
        {
            // Clean up pending
            if let Some(device_pending) = state.pending.get(&device_key) {
                device_pending.remove(&request_id);
            }
            return ToolExecutionResult {
                request_id,
                exit_code: 1,
                output: format!("Failed to send to device: {e}"),
            };
        }
    }
    drop(conn); // Release DashMap ref before awaiting

    // Await response with timeout
    match tokio::time::timeout(
        std::time::Duration::from_secs(TOOL_EXECUTION_TIMEOUT_SEC),
        rx,
    )
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => ToolExecutionResult {
            request_id,
            exit_code: -2,
            output: format!("Device '{device_name}' disconnected during tool execution"),
        },
        Err(_) => {
            // Timeout — clean up pending
            if let Some(device_pending) = state.pending.get(&device_key) {
                device_pending.remove(&request_id);
            }
            warn!("Tool {tool_name} timed out on {device_name}");
            ToolExecutionResult {
                request_id,
                exit_code: -1,
                output: format!(
                    "Tool '{tool_name}' timed out after {TOOL_EXECUTION_TIMEOUT_SEC}s on '{device_name}'"
                ),
            }
        }
    }
}
