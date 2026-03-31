//! Per-Session Agent Loop
//! 每个 session 有独立的实例，消费自己的 inbox queue，不与其他 session 共享

use crate::bus::{InboundEvent, OutboundEvent};
use crate::context;
use crate::providers::{call_with_retry, ChatCompletionRequest};
use crate::state::AppState;
use crate::tools_registry::{route_tool, RouteError};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

fn make_outbound(event: &InboundEvent, content: String) -> OutboundEvent {
    OutboundEvent {
        channel: event.channel.clone(),
        chat_id: event.chat_id.clone(),
        content,
        media: Vec::new(),
        metadata: HashMap::new(),
    }
}

/// Per-Session AgentLoop：消费 session inbox，处理 ReAct 循环
pub async fn run_session(
    session_id: String,
    mut inbox: mpsc::Receiver<InboundEvent>,
    state: Arc<AppState>,
) {
    info!("agent_session started: session_id={}", session_id);

    // Ensure session exists in DB (for message foreign key)
    // Uses sender_id from the first event; silently ignores errors.
    let mut db_session_created = false;

    while let Some(event) = inbox.recv().await {
        if !db_session_created {
            // Create DB session record on first message (need user_id from event)
            if let Err(e) = sqlx::query(
                "INSERT INTO sessions (session_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"
            )
            .bind(&session_id)
            .bind(&event.sender_id)
            .execute(&state.db)
            .await {
                warn!("failed to create DB session {}: {}", session_id, e);
            }
            db_session_created = true;
        }
        info!("agent_session {} received: content={}", session_id, event.content);

        // 持有 session 锁（防止不同 channel 同时写数据库）
        let lock = state.session_manager.get_session_lock(&session_id).await
            .expect("session lock must exist after get_or_create_session");
        let _guard = lock.read().await;

        match run_single_turn(&state, &event).await {
            Ok(response) => {
                state.bus.publish_outbound(make_outbound(&event, response)).await;
            }
            Err(e) => {
                error!("agent_session {} error: {}", session_id, e);
                state.bus.publish_outbound(make_outbound(&event, format!("Error: {}", e))).await;
            }
        }
    }

    info!("agent_session ended: session_id={}", session_id);
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

/// Parsed tool call with arguments as Value (parsed from JSON string)
#[derive(Debug, Clone)]
struct ToolCallParsed {
    id: String,
    name: String,
    arguments: Value,
}

/// Parse tool calls from a Choice, deserializing arguments from JSON string to Value
fn parse_tool_calls(choice: &crate::providers::Choice) -> Vec<ToolCallParsed> {
    choice.message.tool_calls.as_ref()
        .map(|calls| {
            calls.iter().map(|tc| {
                let arguments: Value = serde_json::from_str(&tc.function.arguments)
                    .unwrap_or_else(|_| json!({}));
                ToolCallParsed {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments,
                }
            }).collect()
        })
        .unwrap_or_default()
}

/// 运行一轮 ReAct 循环
async fn run_single_turn(
    state: &Arc<AppState>,
    event: &InboundEvent,
) -> Result<String, String> {
    let user_input = &event.content;
    let user_id = &event.sender_id;
    let session_id = &event.session_id;

    let system_prompt = context::build_system_prompt(state, user_id, session_id, user_input).await;
    let tools = context::get_all_tools_schema(state, user_id).await;
    let history = context::build_message_history(state, session_id).await;

    let mut messages = vec![
        json!({ "role": "system", "content": system_prompt }),
    ];
    messages.extend(history);
    messages.push(json!({ "role": "user", "content": user_input }));

    let _ = crate::db::save_message(&state.db, session_id, "user", user_input, None).await;

    info!("agent_session {} calling LLM with {} tools", session_id, tools.len());
    let request = ChatCompletionRequest {
        messages: messages.clone(),
        tools,
        model: state.config.llm.model.clone(),
    };
    let response = call_with_retry(&state.config.llm, request).await
        .map_err(|e| format!("LLM provider error: {}", e))?;
    let choice = &response.choices[0];
    info!("agent_session {} LLM returned: finish_reason={}", session_id, choice.finish_reason);

    match choice.finish_reason.as_str() {
        "stop" => {
            let reply = choice.message.content.clone().unwrap_or_default();
            info!("agent_session {} returning stop: {}", session_id, reply);
            let _ = crate::db::save_message(&state.db, session_id, "assistant", &reply, None).await;
            Ok(reply)
        }
        "tool_calls" => {
            let tool_calls = parse_tool_calls(choice);
            info!("agent_session {} calling execute_tool_calls_loop with {} tool_calls", session_id, tool_calls.len());
            execute_tool_calls_loop(state, user_id, session_id, messages, tool_calls).await
        }
        _ => Err(format!("unknown finish_reason: {}", choice.finish_reason)),
    }
}

/// 执行工具调用循环（无硬上限，通过循环检测打断真正的死循环）
async fn execute_tool_calls_loop(
    state: &Arc<AppState>,
    user_id: &str,
    session_id: &str,
    messages: Vec<Value>,
    initial_tool_calls: Vec<ToolCallParsed>,
) -> Result<String, String> {
    let mut current_messages = messages.clone();
    let mut current_tool_calls = initial_tool_calls;
    let mut call_counts: HashMap<ToolCallKey, usize> = HashMap::new();
    let mut gave_rethink_chance = false;

    loop {
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

        let mut loop_detected: Option<(&ToolCallParsed, usize)> = None;
        for tc in &current_tool_calls {
            let key = ToolCallKey::new(tc.name.clone(), tc.arguments.clone());
            let count = call_counts.entry(key).or_insert(0);
            *count += 1;
            if *count > MAX_REPEAT_THRESHOLD {
                loop_detected = Some((tc, *count));
                break;
            }
        }

        if let Some((tc, count)) = loop_detected {
            if gave_rethink_chance {
                warn!(
                    "execute_tool_calls_loop: tool '{}' called {} times with identical arguments after rethink chance — hard error",
                    tc.name, count
                );
                return Err(format!(
                    "Tool '{}' has been called repeatedly with the same arguments {} times. After being asked to try a different approach, the same tool was called again. Please try a fundamentally different strategy to complete this task.",
                    tc.name, count
                ));
            }

            gave_rethink_chance = true;
            warn!(
                "execute_tool_calls_loop: tool '{}' called {} times with identical arguments — injecting soft error",
                tc.name, count
            );
            let soft_error = format!(
                "[Loop Detected] The tool '{}' has been called {} times with identical arguments without progress. Please try a fundamentally different strategy or approach to complete this task.",
                tc.name, count
            );
            current_messages.push(json!({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": soft_error
            }));

            let request = ChatCompletionRequest {
                messages: current_messages.clone(),
                tools: vec![],
                model: state.config.llm.model.clone(),
            };
            let response = call_with_retry(&state.config.llm, request).await
                .map_err(|e| format!("LLM provider error: {}", e))?;
            let choice = &response.choices[0];

            match choice.finish_reason.as_str() {
                "stop" => {
                    let reply = choice.message.content.clone().unwrap_or_default();
                    let _ = crate::db::save_message(&state.db, session_id, "assistant", &reply, None).await;
                    return Ok(reply);
                }
                "tool_calls" => {
                    current_tool_calls = parse_tool_calls(choice);
                    info!("execute_tool_calls_loop: after soft error, LLM requested {} new tool calls", current_tool_calls.len());
                    continue;
                }
                _ => {
                    return Err(format!("unknown finish_reason after soft error: {}", choice.finish_reason));
                }
            }
        }

        for tc in &current_tool_calls {
            let result = execute_single_tool(state, user_id, tc).await;
            let content = match result {
                Ok(output) => output,
                Err(e) => format!("{{\"error\": \"{}\"}}", e),
            };
            current_messages.push(json!({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": content
            }));
        }

        let request = ChatCompletionRequest {
            messages: current_messages.clone(),
            tools: vec![],
            model: state.config.llm.model.clone(),
        };
        let response = call_with_retry(&state.config.llm, request).await
            .map_err(|e| format!("LLM provider error: {}", e))?;
        let choice = &response.choices[0];

        match choice.finish_reason.as_str() {
            "stop" => {
                let reply = choice.message.content.clone().unwrap_or_default();
                let _ = crate::db::save_message(&state.db, session_id, "assistant", &reply, None).await;
                return Ok(reply);
            }
            "tool_calls" => {
                current_tool_calls = parse_tool_calls(choice);
                info!("execute_tool_calls_loop: LLM requested {} new tool calls", current_tool_calls.len());
            }
            _ => {
                return Err(format!("unknown finish_reason in tool loop: {}", choice.finish_reason));
            }
        }
    }
}

async fn execute_single_tool(
    state: &Arc<AppState>,
    user_id: &str,
    tc: &ToolCallParsed,
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
