/// 职责边界：
/// 1. 根据 user_id + device_name，O(1) 查找对应的 device_id。
/// 2. 构建 LLM 工具 Schema：向 Client 上报的原始 Schema 中注入 `device_name` enum 参数。
/// 3. 核心路由函数：根据 device_name 将 ExecuteToolRequest 路由到目标设备的 Client。
///
/// 参考 nanobot：
/// - 对应 `nanobot/agent/tools/registry.py` 的工具管理逻辑，但 Nexus 增加了多设备路由层。

use serde_json::Value;
use crate::state::AppState;

#[derive(Debug)]
pub enum RouteError {
    DeviceNotFound(String),
    DeviceOffline(String),
    SendFailed(String),
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RouteError::DeviceNotFound(name) => {
                write!(f, "device '{}' not found or does not belong to this user", name)
            }
            RouteError::DeviceOffline(name) => {
                write!(f, "device '{}' is currently offline", name)
            }
            RouteError::SendFailed(name) => {
                write!(f, "failed to send request to device '{}'", name)
            }
        }
    }
}

/// 根据 user_id 和 device_name 查找 device_id。
///
/// 返回值：
///   Some(device_id) — 找到
///   None            — 设备不存在或不属于该用户
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

/// 修饰工具 schema，注入 device_name 参数。
///
/// 原始 schema（Client 注册时，MCP 工具等原生 schema）：
/// {
///   "type": "function",
///   "function": {
///     "name": "run_shell_command",
///     "parameters": {
///       "type": "object",
///       "properties": { "command": { "type": "string" } }
///     }
///   }
/// }
///
/// 修饰后 schema（Server 注入了 device_name enum，LLM 看到的是完整跨设备视图）：
/// {
///   "type": "function",
///   "function": {
///     "name": "run_shell_command",
///     "parameters": {
///       "type": "object",
///       "properties": {
///         "device_name": { "enum": ["mac-mini", "ubuntu-server"] },
///         "command": { "type": "string" }
///       },
///       "required": ["device_name", "command"]
///     }
///   }
/// }
///
/// 关键点：
/// - Server **修饰**（clone + mutation）Client 上报的 schema，**不修改原始数据**
/// - device_name 被添加到**每个工具**的 schema 中
/// - 原有工具参数（command 等）被保留
pub async fn build_tools_schema(
    state: &AppState,
    user_id: &str,
    original_schemas: Vec<Value>,
) -> Vec<Value> {
    // 1. 获取当前用户的设备名称列表
    let device_enum: Vec<String> = {
        let devices = state.devices_by_user.read().await;
        devices
            .get(user_id)
            .map(|d| d.keys().cloned().collect())
            .unwrap_or_default()
    };

    // 2. 遍历每个工具 schema，注入 device_name 参数
    original_schemas
        .into_iter()
        .map(|schema| inject_device_name_param(schema, &device_enum))
        .collect()
}

fn inject_device_name_param(schema: Value, device_enum: &[String]) -> Value {
    // 确保 schema 格式为 { "type": "function", "function": { ... } }
    let obj = match schema {
        Value::Object(mut m) => {
            // 获取 function 部分
            if let Some(Value::Object(func)) = m.get_mut("function").map(|v| v.take()) {
                let name = func.get("name").cloned();
                let description = func.get("description").cloned();

                // 获取 parameters，若不存在或不是 object 则创建空 object
                let mut params = match func.get("parameters") {
                    Some(Value::Object(p)) => p.clone(),
                    _ => serde_json::Map::new().into(),
                };

                // 注入 device_name 到 properties
                let props = params
                    .entry("properties")
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));

                if let Value::Object(ref mut props_obj) = props {
                    props_obj.insert(
                        "device_name".to_string(),
                        json!({
                            "type": "string",
                            "enum": device_enum,
                            "description": "The target device to execute this tool on."
                        }),
                    );
                }

                // 合并 required：原有 required + device_name
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

                // 重建 function 对象
                let mut new_func = serde_json::Map::new();
                if let Some(n) = name {
                    new_func.insert("name".to_string(), n);
                }
                if let Some(d) = description {
                    new_func.insert("description".to_string(), d);
                }
                new_func.insert("parameters".to_string(), params);

                m.insert("function".to_string(), Value::Object(new_func));
            }
            m
        }
        _ => return schema,
    };
    Value::Object(obj)
}

use serde_json::json;

/// 根据 device_name 路由工具调用到目标设备。
///
/// 流程：
/// 1. 从 LLM arguments 中提取 device_name（LLM 从 schema enum 中选择）
/// 2. 通过 find_device_by_name 查找目标 device_id
/// 3. 验证设备在线
/// 4. 从 arguments 中剥离 device_name（Client 不知道此字段）
/// 5. 通过 ws_tx 发送 ExecuteToolRequest 到目标设备
/// 6. 将 oneshot::Sender 存入 pending 表，挂起等待结果
///
/// 若设备不存在/离线，返回错误字符串，由调用方（agent_loop）包装为 Tool Result 喂回 LLM。
pub async fn route_tool(
    state: &AppState,
    user_id: &str,
    tool_name: &str,
    mut arguments: Value,
    request_id: &str,
) -> Result<Value, RouteError> {
    // 1. 提取 device_name
    let device_name = arguments
        .get("device_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| RouteError::DeviceNotFound("device_name not found in tool arguments".to_string()))?
        .to_string();

    // 2. 查找目标设备
    let device_id = find_device_by_name(state, user_id, &device_name)
        .await
        .ok_or_else(|| RouteError::DeviceNotFound(device_name.clone()))?;

    // 3. 验证设备在线
    let ws_tx = {
        let devices = state.devices.read().await;
        let device_state = devices
            .get(&device_id)
            .ok_or_else(|| RouteError::DeviceNotFound(device_name.clone()))?;

        if device_state.device_name != device_name {
            // device_id 找到了但 device_name 不匹配（说明 devices_by_user 有脏数据）
            return Err(RouteError::DeviceNotFound(device_name));
        }

        device_state.ws_tx.clone()
    };

    // 4. 剥离 device_name 从 arguments（Client 不需要知道设备名）
    if let Some(obj) = arguments.as_object_mut() {
        obj.remove("device_name");
    }

    // 5. 创建 oneshot 通道，存入 pending 表
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut pending = state.pending.write().await;
        pending.insert(request_id.to_string(), tx);
    }

    // 6. 发送 ExecuteToolRequest 到目标设备
    let execute_req = nexus_common::protocol::ServerToClient::ExecuteToolRequest(
        nexus_common::protocol::ExecuteToolRequest {
            request_id: request_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments,
        },
    );
    let msg_text = serde_json::to_string(&execute_req)
        .map_err(|_| RouteError::SendFailed(device_name.clone()))?;
    let msg = axum::extract::ws::Message::Text(msg_text.into());

    ws_tx.send(msg)
        .map_err(|_| RouteError::SendFailed(device_name.clone()))?;

    // 7. 挂起等待结果
    rx.await
        .map_err(|_| RouteError::SendFailed(device_name))?
        .map(|result| {
            // 将 ToolExecutionResult 转为 JSON Value 返回
            serde_json::to_value(result)
                .unwrap_or_else(|_| json!({ "error": "failed to serialize result" }))
        })
}
