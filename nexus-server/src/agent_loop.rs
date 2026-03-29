//! Per-Session Agent Loop
//! 每个 session 有独立的实例，消费自己的 inbox queue，不与其他 session 共享

use crate::bus::{InboundEvent, OutboundEvent};
use crate::context;
use crate::providers::call_with_retry;
use crate::providers::{ChatCompletionRequest, LlmResponse, ToolCallRequest};
use crate::state::AppState;
use crate::tools_registry::{route_tool, RouteError};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

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
        let lock = state.session_manager.get_session_lock(&session_id).await
            .expect("session lock must exist after get_or_create_session");
        let _guard = lock.read().await;

        match run_single_turn(&state, &event).await {
            Ok(response) => {
                let outbound = OutboundEvent {
                    channel: event.channel.clone(),
                    chat_id: event.chat_id.clone(),
                    content: response,
                    media: vec![],
                    metadata: HashMap::new(),
                };
                state.bus.publish_outbound(outbound).await;
            }
            Err(e) => {
                error!("agent_session {} error: {}", session_id, e);
                let outbound = OutboundEvent {
                    channel: event.channel.clone(),
                    chat_id: event.chat_id.clone(),
                    content: format!("Error: {}", e),
                    media: vec![],
                    metadata: HashMap::new(),
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

    // 4. 调用 LLM（含自动重试 429 等瞬时错误）
    info!("agent_session {} calling LLM with {} tools", session_id, tools.len());
    let request = ChatCompletionRequest {
        messages: messages.clone(),
        tools,
        model: "mock".to_string(),
    };
    let response = call_with_retry(request);
    info!("agent_session {} LLM returned: finish_reason={}", session_id, response.choices[0].finish_reason);
    let llm_response = openai_to_llm_response(response);

    // 5. 处理 LLM 返回
    match llm_response.finish_reason.as_str() {
        "stop" => {
            info!("agent_session {} returning stop: {}", session_id, llm_response.content.as_ref().unwrap_or(&"<empty>".to_string()));
            Ok(llm_response.content.unwrap_or_default())
        }
        "tool_calls" => {
            info!("agent_session {} calling execute_tool_calls_loop with {} tool_calls", session_id, llm_response.tool_calls.len());
            execute_tool_calls_loop(state, user_id, messages, llm_response.tool_calls).await
        }
        _ => Err(format!("unknown finish_reason: {}", llm_response.finish_reason)),
    }
}

// ============================================================================
// 循环检测
// ============================================================================

/// 单个工具调用的身份标识（工具名 + 参数）
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct ToolCallKey {
    name: String,
    arguments: Value,
}

impl ToolCallKey {
    fn new(name: String, arguments: Value) -> Self {
        Self { name, arguments }
    }
}

/// 检测同一 (tool_name, arguments) 在一轮工具循环中被调用超过 MAX_REPEAT_THRESHOLD 次
/// 即判定为 LLM 陷入重复调用死循环。
const MAX_REPEAT_THRESHOLD: usize = 2;

/// 执行工具调用循环（无硬上限，通过循环检测打断真正的死循环）
async fn execute_tool_calls_loop(
    state: &Arc<AppState>,
    user_id: &str,
    messages: Vec<Value>,
    initial_tool_calls: Vec<ToolCallRequest>,
) -> Result<String, String> {
    let mut current_messages = messages.clone();
    let mut current_tool_calls = initial_tool_calls;
    // 记录本轮循环中每个 (tool_name, arguments) 被调用的次数
    let mut call_counts: HashMap<ToolCallKey, usize> = HashMap::new();
    /// 是否已经给过 LLM 一次"换个策略"的机会
    let mut gave_rethink_chance = false;

    loop {
        // 1. 将 assistant 消息（含 tool_calls）加入历史
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

        // 2. 循环检测：在执行前先更新计数
        let mut loop_detected: Option<(&ToolCallRequest, usize)> = None;
        for tc in &current_tool_calls {
            let key = ToolCallKey::new(tc.name.clone(), tc.arguments.clone());
            let count = call_counts.entry(key).or_insert(0);
            *count += 1;
            if *count > MAX_REPEAT_THRESHOLD {
                loop_detected = Some((tc, *count));
                break;
            }
        }

        // 3a. 检测到循环
        if let Some((tc, count)) = loop_detected {
            if gave_rethink_chance {
                // 第二次超过阈值 → hard error
                warn!(
                    "execute_tool_calls_loop: tool '{}' called {} times with identical arguments after rethink chance — hard error",
                    tc.name,
                    count
                );
                return Err(format!(
                    "Tool '{}' has been called repeatedly with the same arguments {} times. After being asked to try a different approach, the same tool was called again. Please try a fundamentally different strategy to complete this task.",
                    tc.name,
                    count
                ));
            }

            // 第一次超过阈值 → soft error，让 LLM 换个策略
            gave_rethink_chance = true;
            warn!(
                "execute_tool_calls_loop: tool '{}' called {} times with identical arguments — injecting soft error, asking LLM to try different approach",
                tc.name,
                count
            );
            let soft_error = format!(
                "[Loop Detected] The tool '{}' has been called {} times with identical arguments without progress. Please try a fundamentally different strategy or approach to complete this task.",
                tc.name,
                count
            );
            // 注入 soft error 作为 tool result，发给 LLM 重新思考
            let soft_tr = json!({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": soft_error
            });
            current_messages.push(soft_tr);

            // 调用 LLM，让它基于 soft error 重新思考
            let request = ChatCompletionRequest {
                messages: current_messages.clone(),
                tools: vec![],
                model: "mock".to_string(),
            };
            let response = call_with_retry(request);
            let llm_response = openai_to_llm_response(response);

            match llm_response.finish_reason.as_str() {
                "stop" => return Ok(llm_response.content.unwrap_or_default()),
                "tool_calls" => {
                    let new_count = llm_response.tool_calls.len();
                    info!("execute_tool_calls_loop: after soft error, LLM requested {} new tool calls", new_count);
                    current_tool_calls = llm_response.tool_calls;
                    // call_counts 不清除，继续累积
                    // 如果 LLM 继续调同一个工具，下一轮会触发 hard error
                    continue;
                }
                _ => {
                    return Err(format!("unknown finish_reason after soft error: {}", llm_response.finish_reason));
                }
            }
        }

        // 3b. 执行所有工具调用（无循环时）
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

        // 4. 调用 LLM（含自动重试 429 等瞬时错误），传入工具结果
        let request = ChatCompletionRequest {
            messages: current_messages.clone(),
            tools: vec![],
            model: "mock".to_string(),
        };
        let response = call_with_retry(request);
        let llm_response = openai_to_llm_response(response);

        match llm_response.finish_reason.as_str() {
            "stop" => return Ok(llm_response.content.unwrap_or_default()),
            "tool_calls" => {
                let new_count = llm_response.tool_calls.len();
                info!("execute_tool_calls_loop: LLM requested {} new tool calls", new_count);
                current_tool_calls = llm_response.tool_calls;
                // 注意：call_counts 不清除，继续累积计数
                // 这样同一工具被不同 turn 反复调用相同参数也会被检测到
            }
            _ => {
                return Err(format!("unknown finish_reason in tool loop: {}", llm_response.finish_reason));
            }
        }
    }
}

async fn execute_single_tool(
    state: &Arc<AppState>,
    user_id: &str,
    tc: &ToolCallRequest,
) -> Result<String, String> {
    info!("execute_single_tool: tool_name={}, arguments={}", tc.name, tc.arguments);
    let device_name = tc.arguments
        .get("device_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "device_name not found in tool call arguments".to_string())?
        .to_string();
    info!("execute_single_tool: resolved device_name={}", device_name);

    let params = tc.arguments.clone();
    let request_id = tc.id.clone();

    info!("execute_single_tool: calling route_tool for device={}", device_name);
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
