/// Responsibility boundary:
/// 1. O(1) lookup of device_id by user_id + device_name.
/// 2. Build LLM tool schemas: inject `device_name` enum parameter into schemas reported by clients.
/// 3. Core routing function: route ExecuteToolRequest to the target device's client by device_name.

use serde_json::Value;
use nexus_common::error::{ErrorCode, NexusError};
use crate::state::AppState;

/// Find device_id by user_id and device_name.
///
/// Returns:
///   Some(device_id) -- found
///   None            -- device does not exist or does not belong to this user
pub async fn find_device_by_name(
    state: &AppState,
    user_id: &str,
    device_name: &str,
) -> Option<String> {
    let devices = state.devices_by_user.read().await;
    devices
        .get(user_id)?
        .get(device_name)
        .cloned()
}

fn inject_device_name_param(schema: Value, device_enum: &[String]) -> Value {
    // Ensure schema format is { "type": "function", "function": { ... } }
    let obj = match schema {
        Value::Object(mut m) => {
            // Get the function part
            if let Some(Value::Object(func)) = m.get_mut("function").map(|v| v.take()) {
                let name = func.get("name").cloned();
                let description = func.get("description").cloned();

                // Get parameters; if absent or not an object, create an empty object
                let mut params = match func.get("parameters") {
                    Some(Value::Object(p)) => p.clone(),
                    _ => serde_json::Map::new().into(),
                };

                // Inject device_name into properties
                let props = params
                    .entry("properties")
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));

                if let Value::Object(props_obj) = props {
                    props_obj.insert(
                        "device_name".to_string(),
                        json!({
                            "type": "string",
                            "enum": device_enum,
                            "description": "The target device to execute this tool on."
                        }),
                    );
                }

                // Merge required: existing required + device_name
                let existing_required: Vec<String> = params
                    .get("required")
                    .and_then(|r| r.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                let required: Vec<String> = if !existing_required.contains(&"device_name".to_string()) {
                    let mut r = existing_required;
                    r.push("device_name".to_string());
                    r
                } else {
                    existing_required
                };

                params.insert("required".to_string(), json!(required));

                // Rebuild the function object
                let mut new_func = serde_json::Map::new();
                if let Some(n) = name {
                    new_func.insert("name".to_string(), n);
                }
                if let Some(d) = description {
                    new_func.insert("description".to_string(), d);
                }
                new_func.insert("parameters".to_string(), Value::Object(params));

                m.insert("function".to_string(), Value::Object(new_func));
            }
            m
        }
        _ => return schema,
    };
    Value::Object(obj)
}

use serde_json::json;

/// Merge tool schemas across multiple devices, deduplicating by tool name.
///
/// When multiple devices register the same tool (e.g., both "xiaoshu" and "server2"
/// have "shell"), this function produces a single schema entry with a multi-value
/// `device_name` enum listing all devices that provide the tool.
///
/// The first schema seen for a given tool name wins as the "base" (parameters, description).
pub fn merge_device_tool_schemas(
    device_tools: &[(String, Vec<Value>)], // (device_name, schemas)
) -> Vec<Value> {
    use std::collections::HashMap;

    // tool_name -> index into `entries` vec
    let mut index_map: HashMap<String, usize> = HashMap::new();
    // (base_schema, collected_device_names) in insertion order
    let mut entries: Vec<(Value, Vec<String>)> = Vec::new();

    for (device_name, schemas) in device_tools {
        for schema in schemas {
            let tool_name = schema
                .pointer("/function/name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if tool_name.is_empty() {
                continue;
            }

            if let Some(&idx) = index_map.get(&tool_name) {
                let devices = &mut entries[idx].1;
                if !devices.contains(device_name) {
                    devices.push(device_name.clone());
                }
            } else {
                let idx = entries.len();
                entries.push((schema.clone(), vec![device_name.clone()]));
                index_map.insert(tool_name, idx);
            }
        }
    }

    entries
        .into_iter()
        .map(|(schema, devices)| inject_device_name_param(schema, &devices))
        .collect()
}

/// Inject device_name parameter into a list of schemas with a single device name.
/// Used for server MCP tools where device_name is always "server".
pub fn inject_device_name_into_schemas(schemas: &[Value], device_name: &str) -> Vec<Value> {
    let device_enum = vec![device_name.to_string()];
    schemas.iter()
        .map(|s| inject_device_name_param(s.clone(), &device_enum))
        .collect()
}

/// Route a tool call to the target device by device_name.
///
/// Flow:
/// 1. Extract device_name from LLM arguments (LLM selects from schema enum)
/// 2. Find the target device_id via find_device_by_name
/// 3. Verify the device is online
/// 4. Strip device_name from arguments (client does not know this field)
/// 5. Send ExecuteToolRequest to the target device via ws_tx
/// 6. Store oneshot::Sender in pending table and await the result
///
/// If the device does not exist or is offline, returns an error for the caller (agent_loop) to feed back to the LLM.
pub async fn route_tool(
    state: &AppState,
    user_id: &str,
    tool_name: &str,
    mut arguments: Value,
    request_id: &str,
) -> Result<Value, NexusError> {
    // 1. Extract device_name
    let device_name = arguments
        .get("device_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| NexusError::new(ErrorCode::DeviceNotFound, "device_name not found in tool arguments"))?
        .to_string();

    // 2. Find the target device
    let device_id = find_device_by_name(state, user_id, &device_name)
        .await
        .ok_or_else(|| NexusError::new(ErrorCode::DeviceNotFound, format!("device '{}' not found or does not belong to this user", device_name)))?;

    // 3. Verify device is online
    let ws_tx = {
        let devices = state.devices.read().await;
        let device_state = devices
            .get(&device_id)
            .ok_or_else(|| NexusError::new(ErrorCode::DeviceNotFound, format!("device '{}' not found", device_name)))?;

        if device_state.device_name != device_name {
            // device_id found but device_name does not match (stale data in devices_by_user)
            return Err(NexusError::new(ErrorCode::DeviceNotFound, format!("device '{}' name mismatch", device_name)));
        }

        device_state.ws_tx.clone()
    };

    // 4. Strip device_name from arguments (client does not need it)
    if let Some(obj) = arguments.as_object_mut() {
        obj.remove("device_name");
    }

    // 5. Create oneshot channel and store in pending table
    let (tx, rx) = tokio::sync::oneshot::channel();
    state.pending.insert(request_id.to_string(), tx);

    // 6. Send ExecuteToolRequest to target device
    let execute_req = nexus_common::protocol::ServerToClient::ExecuteToolRequest(
        nexus_common::protocol::ExecuteToolRequest {
            request_id: request_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments,
        },
    );
    let msg_text = serde_json::to_string(&execute_req)
        .map_err(|_| NexusError::new(ErrorCode::ChannelError, format!("failed to serialize request for device '{}'", device_name)))?;
    let msg = axum::extract::ws::Message::Text(msg_text.into());

    if let Err(_) = ws_tx.send(msg).await {
        // Clean up stale pending entry before returning error
        state.pending.remove(&request_id.to_string());
        return Err(NexusError::new(ErrorCode::ChannelError, format!("failed to send request to device '{}'", device_name)));
    }

    // 7. Await the result (120s timeout to prevent indefinite hang)
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        rx,
    ).await
        .map_err(|_| {
            // Clean up stale pending entry on timeout
            state.pending.remove(&request_id.to_string());
            NexusError::new(ErrorCode::ExecutionTimeout, format!("device '{}' timed out after 120s", device_name))
        })?
        .map_err(|_| NexusError::new(ErrorCode::ChannelError, format!("channel closed for device '{}'", device_name)))?;

    serde_json::to_value(result)
        .map_err(|_| NexusError::new(ErrorCode::InternalError, format!("failed to serialize result from device '{}'", device_name)))
}
