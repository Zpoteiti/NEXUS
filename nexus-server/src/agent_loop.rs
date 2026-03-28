//! Per-Session Agent Loop
//! 每个 session 有独立的实例，消费自己的 inbox queue，不与其他 session 共享

use crate::bus::{InboundEvent, OutboundEvent};
use crate::context;
use crate::providers::mock::chat_completion;
use crate::providers::{ChatCompletionRequest, LlmResponse, ToolCallRequest};
use crate::state::AppState;
use crate::tools_registry::{route_tool, RouteError};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info};

/// Per-Session AgentLoop：消费 session inbox，处理 ReAct 循环
pub async fn run_session(
    session_id: String,
    mut inbox: mpsc::Receiver<InboundEvent>,
    state: Arc<AppState>,
) {
    info!("agent_session started: session_id={}", session_id);

    while let Some(event) = inbox.recv().await {
        info!("agent_session {} received: content={}", session_id, event.content);

        // 持有 session 锁（防止不同 channel 同时写数据库）
        let lock = state.session_manager.get_session_lock(&session_id).await;
        let _guard = lock.map(|l| l.read().await);

        match run_single_turn(&state, &event).await {
            Ok(response) => {
                let outbound = OutboundEvent {
                    channel: event.channel.clone(),
                    chat_id: event.chat_id.clone(),
                    content: response,
                };
                state.bus.publish_outbound(outbound).await;
            }
            Err(e) => {
                error!("agent_session {} error: {}", session_id, e);
                let outbound = OutboundEvent {
                    channel: event.channel.clone(),
                    chat_id: event.chat_id.clone(),
                    content: format!("Error: {}", e),
                };
                state.bus.publish_outbound(outbound).await;
            }
        }
    }

    info!("agent_session ended: session_id={}", session_id);
}

/// 运行一轮 ReAct 循环
async fn run_single_turn(
    state: &Arc<AppState>,
    event: &InboundEvent,
) -> Result<String, String> {
    let user_input = &event.content;
    let user_id = &event.sender_id;
    let session_id = &event.session_id;

    // 1. 构建 system prompt
    let system_prompt = context::build_system_prompt(state, user_id, session_id, user_input).await;

    // 2. 获取工具 schema
    let tools = context::get_all_tools_schema(state, user_id).await;

    // 3. 构建 messages
    let messages = vec![
        json!({ "role": "system", "content": system_prompt }),
        json!({ "role": "user", "content": user_input }),
    ];

    // 4. 调用 Mock LLM
    let request = ChatCompletionRequest {
        messages: messages.clone(),
        tools,
        model: "mock".to_string(),
    };
    let response = chat_completion(request);
    let llm_response = openai_to_llm_response(response);

    // 5. 处理 LLM 返回
    match llm_response.finish_reason.as_str() {
        "stop" => Ok(llm_response.content.unwrap_or_default()),
        "tool_calls" => {
            execute_tool_calls_loop(state, user_id, messages, llm_response.tool_calls).await
        }
        _ => Err(format!("unknown finish_reason: {}", llm_response.finish_reason)),
    }
}

/// 执行工具调用循环
async fn execute_tool_calls_loop(
    state: &Arc<AppState>,
    user_id: &str,
    mut messages: Vec<Value>,
    initial_tool_calls: Vec<ToolCallRequest>,
) -> Result<String, String> {
    let max_retries = 3;
    let mut current_messages = messages.clone();
    let mut current_tool_calls = initial_tool_calls;

    for attempt in 0..max_retries {
        for tc in &current_tool_calls {
            current_messages.push(json!({
                "role": "assistant",
                "tool_calls": [{
                    "id": tc.id,
                    "type": "function",
                    "function": { "name": tc.name, "arguments": tc.arguments }
                }]
            }));
        }

        let mut all_results: Vec<Value> = Vec::new();
        for tc in &current_tool_calls {
            let result = execute_single_tool(state, user_id, tc).await;
            let content = match result {
                Ok(output) => output,
                Err(e) => format!("{{\"error\": \"{}\"}}", e),
            };
            let tr = json!({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": content
            });
            current_messages.push(tr.clone());
            all_results.push(tr);
        }

        let request = ChatCompletionRequest {
            messages: current_messages.clone(),
            tools: vec![],
            model: "mock".to_string(),
        };
        let response = chat_completion(request);
        let llm_response = openai_to_llm_response(response);

        match llm_response.finish_reason.as_str() {
            "stop" => return Ok(llm_response.content.unwrap_or_default()),
            "tool_calls" => {
                current_tool_calls = llm_response.tool_calls;
                info!("tool retry {} with {} new calls", attempt + 1, current_tool_calls.len());
            }
            _ => {
                return Err(format!("unknown finish_reason in tool loop: {}", llm_response.finish_reason));
            }
        }
    }

    Err(format!("exceeded max tool retries ({})", max_retries))
}

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

    let params = tc.arguments.clone();
    let request_id = tc.id.clone();

    match route_tool(state, user_id, &tc.name, params, &request_id).await {
        Ok(result) => {
            let exit_code = result.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(1);
            let output = result.get("output").and_then(|v| v.as_str()).unwrap_or("");
            if exit_code == 0 { Ok(output.to_string()) } else { Err(output.to_string()) }
        }
        Err(RouteError::DeviceNotFound(name)) => Err(format!("device '{}' not found", name)),
        Err(RouteError::DeviceOffline(name)) => Err(format!("device '{}' is offline", name)),
        Err(RouteError::SendFailed(name)) => Err(format!("failed to send request to '{}'", name)),
    }
}

fn openai_to_llm_response(response: crate::providers::ChatCompletionResponse) -> LlmResponse {
    let choice = &response.choices[0];
    let content = choice.message.content.clone();
    let finish_reason = choice.finish_reason.clone();

    let tool_calls: Vec<ToolCallRequest> = choice
        .tool_calls
        .as_ref()
        .map(|calls| {
            calls.iter().map(|tc| {
                let arguments: serde_json::Value = serde_json::from_str(tc.function.arguments.as_str())
                    .unwrap_or_else(|_| json!({}));
                ToolCallRequest {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments,
                }
            }).collect()
        })
        .unwrap_or_default();

    LlmResponse { content, tool_calls, finish_reason }
}