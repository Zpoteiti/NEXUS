/// 职责边界：
/// 1. 实现核心的 `run_agent_loop` 函数，控制 ReAct（思考-行动）的 while 循环。
/// 2. 【挂起/唤醒机制】当 Provider 返回 ToolCall 时：
///    a. 生成唯一的 request_id（UUID）。
///    b. 创建 oneshot::channel，将 oneshot::Sender 存入 AppState 挂起等待表。
///    c. 通过 route_tool 发送到目标设备。
///    d. .await oneshot::Receiver 挂起当前循环，让出线程。
///    e. ws.rs 收到 Client 返回的 ToolExecutionResult 后，唤醒继续执行。
/// 3. 【自我纠正机制】收到执行错误时，将错误信息包装为 tool_result 喂回 LLM。
///
/// 参考 nanobot：
/// - 完全复刻 nanobot/agent/loop.py 中的 _run_agent_loop 状态机逻辑。
/// - nanobot/agent/loop.py _run_agent_loop 中 finish_reason=="error" 分支。

use crate::context;
use crate::providers::{LlmResponse, ToolCallRequest};
use crate::state::AppState;
use crate::tools_registry::{route_tool, RouteError};
use serde_json::Value;
use std::sync::Arc;
use tracing::{error, info};

/// ReAct 超时配置
const AGENT_TIMEOUT_SECS: u64 = 120;

/// 运行一轮 ReAct 循环，处理单次用户输入。
///
/// 调用流程：
/// 1. 构建 system_prompt（设备列表 + 可用工具 schema）
/// 2. 构建 messages（含 history）
/// 3. 调用 LLM
/// 4. 若 LLM 返回 content → 返回
/// 5. 若 LLM 返回 tool_calls → 逐个路由执行，结果喂回 LLM，重试（最多 MAX_TOOL_RETRIES 次）
/// 6. 若 LLM 返回 error → 包装为错误 tool_result 重试
pub async fn run_single_turn(
    state: Arc<AppState>,
    user_id: &str,
    session_id: &str,
    user_input: &str,
    messages: &mut Vec<Value>,
) -> Result<String, String> {
    // 1. 构建 system prompt
    let system_prompt = context::build_system_prompt(&state, user_id, session_id, user_input).await;

    // 2. 获取工具 schema（带 device_name enum 注入）
    let tools = context::get_all_tools_schema(&state, user_id).await;

    // 3. 构建完整的 messages 列表
    let mut all_messages = vec![Value::String(system_prompt)];
    all_messages.extend(messages.clone());
    all_messages.push(json!({
        "role": "user",
        "content": user_input
    }));

    // 4. 调用 LLM（providers 抽象）
    let llm_response = call_llm_with_tools(&all_messages, &tools).await?;

    // 5. 处理 LLM 返回
    match llm_response.finish_reason.as_str() {
        "stop" => {
            // LLM 返回文本，直接追加到历史并返回
            if let Some(content) = llm_response.content {
                messages.push(json!({ "role": "assistant", "content": content }));
                return Ok(content);
            }
            Ok(String::new())
        }
        "tool_calls" => {
            // LLM 返回工具调用，进入工具执行循环
            execute_tool_calls_loop(state, user_id, &mut all_messages, llm_response.tool_calls, messages).await
        }
        "error" => {
            // LLM 错误，不写入历史，返回错误
            Err(llm_response.content.unwrap_or_else(|| "LLM error".to_string()))
        }
        _ => Err(format!("unknown finish_reason: {}", llm_response.finish_reason)),
    }
}

/// 工具调用循环：执行工具调用，若结果触发 LLM 自我纠正则重试。
async fn execute_tool_calls_loop(
    state: Arc<AppState>,
    user_id: &str,
    messages: &mut Vec<Value>,
    initial_tool_calls: Vec<ToolCallRequest>,
    history: &mut Vec<Value>,
) -> Result<String, String> {
    let max_retries = 3;
    let mut current_messages = messages.clone();
    let mut current_tool_calls = initial_tool_calls;

    for attempt in 0..max_retries {
        // 追加当前轮次的 tool_calls 消息
        for tc in &current_tool_calls {
            current_messages.push(json!({
                "role": "assistant",
                "tool_calls": [{
                    "id": tc.id,
                    "type": "function",
                    "function": {
                        "name": tc.name,
                        "arguments": tc.arguments
                    }
                }]
            }));
        }

        // 执行所有 tool_calls，追加 tool_results
        let mut all_results: Vec<Value> = Vec::new();
        for tc in &current_tool_calls {
            let result = execute_single_tool(&state, user_id, tc).await;
            let tool_result_content = match result {
                Ok(output) => output,
                Err(e) => format!("{{\"error\": \"{}\"}}", e),
            };
            let tr = json!({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": tool_result_content
            });
            current_messages.push(tr.clone());
            all_results.push(tr);
        }

        // 若所有工具执行成功，追加 tool_results 到历史
        history.extend(all_results);

        // 调用 LLM 继续推理（带上工具执行结果）
        let tools = context::get_all_tools_schema(&state, user_id).await;
        let llm_response = call_llm_with_tools(&current_messages, &tools).await?;

        match llm_response.finish_reason.as_str() {
            "stop" => {
                if let Some(content) = llm_response.content {
                    history.push(json!({ "role": "assistant", "content": content }));
                    return Ok(content);
                }
                return Ok(String::new());
            }
            "tool_calls" => {
                // LLM 决定继续调用工具，更新 tool_calls 并继续循环
                current_tool_calls = llm_response.tool_calls;
                // 注意：不追加到 history（这轮还没结束）
                info!("tool retry {} with {} new calls", attempt + 1, current_tool_calls.len());
            }
            "error" => {
                // LLM 报错，将错误包装为 tool_result 重试
                let error_msg = llm_response.content.unwrap_or_else(|| "LLM error".to_string());
                history.push(json!({
                    "role": "tool",
                    "tool_call_id": current_tool_calls.first().map(|tc| tc.id.as_str()).unwrap_or("error"),
                    "content": format!("{{\"error\": \"{}\"}}", error_msg)
                }));
                error!("LLM error during tool execution: {}", error_msg);
                if attempt == max_retries - 1 {
                    return Err(format!("LLM error after {} retries: {}", max_retries, error_msg));
                }
            }
            _ => {
                return Err(format!("unknown finish_reason in tool loop: {}", llm_response.finish_reason));
            }
        }
    }

    Err(format!("exceeded max tool retries ({})", max_retries))
}

/// 执行单个工具调用，通过 route_tool 路由到目标设备。
async fn execute_single_tool(
    state: &Arc<AppState>,
    user_id: &str,
    tc: &ToolCallRequest,
) -> Result<String, String> {
    let device_name = tc.arguments
        .get("device_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "device_name not found in tool call arguments".to_string())?
        .to_string();

    let tool_name = &tc.name;
    let params = tc.arguments.clone();
    let request_id = tc.id.clone();

    match route_tool(state, user_id, tool_name, params, &request_id).await {
        Ok(result) => {
            // ToolExecutionResult { exit_code, output }
            let exit_code = result.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(1);
            let output = result.get("output").and_then(|v| v.as_str()).unwrap_or("");
            if exit_code == 0 {
                Ok(output.to_string())
            } else {
                Err(output.to_string())
            }
        }
        Err(RouteError::DeviceNotFound(name)) => {
            Err(format!("device '{}' not found", name))
        }
        Err(RouteError::DeviceOffline(name)) => {
            Err(format!("device '{}' is offline", name))
        }
        Err(RouteError::SendFailed(name)) => {
            Err(format!("failed to send request to '{}'", name))
        }
    }
}

/// 调用 LLM（通过 providers trait）。
///
/// 目前 providers 为 stub，调用时会返回 mock 响应。
async fn call_llm_with_tools(
    messages: &[Value],
    _tools: &[Value],
) -> Result<LlmResponse, String> {
    // TODO: 接入 providers::call_llm
    // providers::openai::call_llm(messages, tools).await
    Ok(LlmResponse {
        content: Some("Tool execution completed.".to_string()),
        tool_calls: Vec::new(),
        finish_reason: "stop".to_string(),
    })
}
