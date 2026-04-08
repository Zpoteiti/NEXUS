//! Per-Session Agent Loop
//! Each session has its own instance, consuming its own inbox queue, independent from other sessions.

use crate::bus::{InboundEvent, OutboundEvent};
use crate::context;
use crate::providers::{call_with_retry, ChatCompletionRequest};
use crate::state::AppState;
use crate::tools_registry::route_tool;
use base64::Engine;
use nexus_common::consts::SERVER_DEVICE_NAME;
use nexus_common::error::{ErrorCode, NexusError};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Result of a single turn, including the reply text and any media file paths.
struct TurnResult {
    reply: String,
    media: Vec<String>,
}

/// Post-process LLM content before returning to user.
/// Strips <think>...</think> blocks from reasoning models.
fn finalize_content(content: &str) -> String {
    static THINK_RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?s)<think>.*?</think>").unwrap());
    let cleaned = THINK_RE.replace_all(content, "");
    cleaned.trim().to_string()
}

use nexus_common::mime::{detect_mime_from_bytes, detect_mime_from_extension};

async fn emit_progress(state: &AppState, channel: &str, chat_id: &str, hint: &str) {
    let mut metadata = HashMap::new();
    metadata.insert("_progress".to_string(), serde_json::json!(true));
    let _ = state.bus.publish_outbound(OutboundEvent {
        channel: channel.to_string(),
        chat_id: chat_id.to_string(),
        content: hint.to_string(),
        media: Vec::new(),
        metadata,
    }).await;
}

fn make_outbound(event: &InboundEvent, content: String) -> OutboundEvent {
    let mut metadata = HashMap::new();
    metadata.insert("sender_id".into(), serde_json::json!(event.sender_id));
    OutboundEvent {
        channel: event.channel.clone(),
        chat_id: event.chat_id.clone(),
        content,
        media: Vec::new(),
        metadata,
    }
}

/// Per-Session AgentLoop: consumes the session inbox and runs the ReAct loop.
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
            if let Err(e) = crate::db::ensure_session(&state.db, &session_id, &event.sender_id).await {
                warn!("failed to create DB session {}: {}", session_id, e);
            }
            db_session_created = true;
        }
        debug!("agent_session {} received: content={}", session_id, event.content);

        // Hold the session lock (prevent concurrent DB writes from different channels)
        let lock = state.session_manager.get_session_lock(&session_id).await
            .expect("session lock must exist after get_or_create_session");
        let _guard = lock.lock().await;

        match run_single_turn(&state, &event).await {
            Ok(result) => {
                let outbound = OutboundEvent {
                    channel: event.channel.clone(),
                    chat_id: event.chat_id.clone(),
                    content: finalize_content(&result.reply),
                    media: result.media,
                    metadata: HashMap::new(),
                };
                state.bus.publish_outbound(outbound).await;
            }
            Err(e) => {
                error!("agent_session {} error: {}", session_id, e);
                // Show user-friendly message, log full error
                let user_msg = match e.code {
                    ErrorCode::ExecutionFailed => format!("Something went wrong: {}", e.message),
                    ErrorCode::ToolTimeout => "The operation timed out. Please try again.".to_string(),
                    ErrorCode::DeviceNotFound => "The device is not connected.".to_string(),
                    ErrorCode::DeviceOffline => "The device appears to be offline.".to_string(),
                    _ => format!("Error: {}", e.message),
                };
                state.bus.publish_outbound(make_outbound(&event, user_msg)).await;
            }
        }
    }

    state.bus.unregister_session(&session_id);
    state.session_manager.remove_session(&session_id).await;
    info!("agent_session ended and cleaned up: session_id={}", session_id);
}

// ============================================================================
// Loop detection
// ============================================================================

/// Identity key for a single tool call (tool name + arguments).
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

/// If the same (tool_name, arguments) pair is called more than MAX_REPEAT_THRESHOLD times
/// in a single tool loop, it is treated as the LLM being stuck in a repeat-call loop.
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

/// Run a single ReAct turn.
async fn run_single_turn(
    state: &Arc<AppState>,
    event: &InboundEvent,
) -> Result<TurnResult, NexusError> {
    let user_input = &event.content;
    let user_id = &event.sender_id;
    let session_id = &event.session_id;

    let llm_config = match state.config.llm.read().await.clone() {
        Some(config) => config,
        None => {
            state.bus.publish_outbound(make_outbound(event,
                "⚠️ LLM not configured. An admin must set up the LLM provider via the API first.".into()
            )).await;
            return Ok(TurnResult { reply: "LLM not configured".into(), media: Vec::new() });
        }
    };

    // Check if this is a checkpoint resume
    if let Some(resume_msgs) = event.metadata.get("resume_messages") {
        if let Some(resume_arr) = resume_msgs.as_array() {
            let resume_iteration = event.metadata.get("resume_iteration")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            let resumed_messages: Vec<Value> = resume_arr.clone();
            let tools = context::get_all_tools_schema(state, user_id).await;

            info!(
                "agent_session {} resuming from checkpoint: {} messages, iteration {}",
                session_id, resumed_messages.len(), resume_iteration
            );

            // Re-call LLM with the saved context to continue the tool loop
            let request = ChatCompletionRequest {
                messages: resumed_messages.clone(),
                tools: tools.clone(),
                model: llm_config.model.clone(),
                max_tokens: None,
            };
            let response = call_with_retry(&llm_config, request).await
                .map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("LLM provider error: {}", e)))?;
            let choice = response.choices.first()
                .ok_or_else(|| NexusError::new(ErrorCode::ExecutionFailed, "LLM returned empty choices array"))?;

            return match choice.finish_reason.as_str() {
                "stop" => {
                    let reply = choice.message.content.clone().unwrap_or_default();
                    let _ = crate::db::save_message(&state.db, session_id, "assistant", &reply, None, None, None).await;
                    let _ = crate::db::delete_checkpoint(&state.db, session_id).await;
                    Ok(TurnResult { reply, media: Vec::new() })
                }
                "tool_calls" => {
                    let tool_calls = parse_tool_calls(choice);
                    info!("agent_session {} resume: LLM requested {} tool calls", session_id, tool_calls.len());
                    execute_tool_calls_loop(state, user_id, session_id, &event.channel, &event.chat_id, resumed_messages, tool_calls, tools, &llm_config).await
                }
                _ => Err(NexusError::new(ErrorCode::ExecutionFailed, format!("unknown finish_reason: {}", choice.finish_reason))),
            };
        }
    }

    let system_prompt = context::build_system_prompt(state, user_id, session_id, user_input, &event.metadata).await;
    let tools = context::get_all_tools_schema(state, user_id).await;
    let history = context::build_message_history(state, session_id).await;

    let mut messages = vec![
        json!({ "role": "system", "content": system_prompt }),
    ];
    messages.extend(history);

    // Build user message — include images as vision content blocks if media is present
    let user_message = if event.media.is_empty() {
        json!({ "role": "user", "content": user_input })
    } else {
        let mut parts: Vec<Value> = Vec::new();
        for path in &event.media {
            if let Ok(bytes) = tokio::fs::read(path).await {
                let mime = detect_mime_from_extension(path)
                    .or_else(|| detect_mime_from_bytes(&bytes));
                if let Some(mime) = mime {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    parts.push(json!({
                        "type": "image_url",
                        "image_url": {"url": format!("data:{};base64,{}", mime, b64)}
                    }));
                } else {
                    let name = std::path::Path::new(path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let size_kb = tokio::fs::metadata(path).await
                        .map(|m| m.len() / 1024)
                        .unwrap_or(0);
                    parts.push(json!({"type": "text", "text": format!(
                        "[User uploaded: {} ({}KB) — use download_to_device to transfer to a device for processing]",
                        name, size_kb
                    )}));
                }
            }
        }
        parts.push(json!({"type": "text", "text": user_input}));
        json!({"role": "user", "content": parts})
    };
    messages.push(user_message);

    let _ = crate::db::save_message(&state.db, session_id, "user", user_input, None, None, None).await;

    // Context compression: compress history if context window is running low
    if let Some(consolidated) = crate::memory::maybe_consolidate(
        session_id,
        user_id,
        &state.db,
        &llm_config,
        &messages,
        llm_config.context_window,
    ).await {
        messages = consolidated;
    }

    emit_progress(state, &event.channel, &event.chat_id, "⏳ Thinking...").await;

    info!("agent_session {} calling LLM with {} tools", session_id, tools.len());
    let request = ChatCompletionRequest {
        messages: messages.clone(),
        tools: tools.clone(),
        model: llm_config.model.clone(),
        max_tokens: None,
    };
    let response = call_with_retry(&llm_config, request).await
        .map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("LLM provider error: {}", e)))?;
    let choice = response.choices.first()
        .ok_or_else(|| NexusError::new(ErrorCode::ExecutionFailed, "LLM returned empty choices array"))?;
    info!("agent_session {} LLM returned: finish_reason={}", session_id, choice.finish_reason);

    match choice.finish_reason.as_str() {
        "stop" => {
            let reply = choice.message.content.clone().unwrap_or_default();
            info!("agent_session {} returning stop: {}", session_id, reply);
            let _ = crate::db::save_message(&state.db, session_id, "assistant", &reply, None, None, None).await;
            Ok(TurnResult { reply, media: Vec::new() })
        }
        "tool_calls" => {
            let tool_calls = parse_tool_calls(choice);
            info!("agent_session {} calling execute_tool_calls_loop with {} tool_calls", session_id, tool_calls.len());
            execute_tool_calls_loop(state, user_id, session_id, &event.channel, &event.chat_id, messages, tool_calls, tools, &llm_config).await
        }
        _ => Err(NexusError::new(ErrorCode::ExecutionFailed, format!("unknown finish_reason: {}", choice.finish_reason))),
    }
}

/// Execute the tool call loop (no hard cap; real infinite loops are broken by loop detection).
async fn execute_tool_calls_loop(
    state: &Arc<AppState>,
    user_id: &str,
    session_id: &str,
    event_channel: &str,
    event_chat_id: &str,
    messages: Vec<Value>,
    initial_tool_calls: Vec<ToolCallParsed>,
    tools: Vec<Value>,
    llm_config: &crate::config::LlmConfig,
) -> Result<TurnResult, NexusError> {
    let mut current_messages = messages.clone();
    let mut current_tool_calls = initial_tool_calls;
    let mut call_counts: HashMap<ToolCallKey, usize> = HashMap::new();
    let mut gave_rethink_chance = false;
    let mut pending_media: Vec<String> = Vec::new();
    let mut iteration = 0u32;

    loop {
        iteration += 1;
        if iteration > nexus_common::consts::MAX_AGENT_ITERATIONS {
            let msg = format!("Agent reached maximum iteration limit ({}). Stopping.", nexus_common::consts::MAX_AGENT_ITERATIONS);
            warn!("{}", msg);
            let _ = crate::db::save_message(&state.db, session_id, "assistant", &msg, None, None, None).await;
            return Ok(TurnResult { reply: msg, media: pending_media });
        }
        // Build a single assistant message with all tool calls
        let tool_calls_json: Vec<Value> = current_tool_calls.iter().map(|tc| {
            json!({
                "id": tc.id,
                "type": "function",
                "function": { "name": tc.name, "arguments": tc.arguments.to_string() }
            })
        }).collect();
        current_messages.push(json!({
            "role": "assistant",
            "tool_calls": tool_calls_json
        }));
        // Save each tool call to DB
        for tc in &current_tool_calls {
            let _ = crate::db::save_message(
                &state.db, session_id, "assistant", "",
                Some(&tc.id), Some(&tc.name), Some(&tc.arguments.to_string()),
            ).await;
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
                return Err(NexusError::new(ErrorCode::ExecutionFailed, format!(
                    "Tool '{}' has been called repeatedly with the same arguments {} times. After being asked to try a different approach, the same tool was called again. Please try a fundamentally different strategy to complete this task.",
                    tc.name, count
                )));
            }

            gave_rethink_chance = true;
            warn!(
                "execute_tool_calls_loop: tool '{}' called {} times with identical arguments — injecting soft error",
                tc.name, count
            );
            // Must provide a tool result for EVERY tool_call_id in the assistant message
            for call in &current_tool_calls {
                let content = if call.id == tc.id {
                    format!(
                        "[Loop Detected] The tool '{}' has been called {} times with identical arguments without progress. Please try a fundamentally different strategy or approach to complete this task.",
                        tc.name, count
                    )
                } else {
                    "[Skipped] Tool execution skipped due to loop detection on a parallel call.".to_string()
                };
                current_messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call.id,
                    "content": content
                }));
            }

            let request = ChatCompletionRequest {
                messages: current_messages.clone(),
                tools: tools.clone(),
                model: llm_config.model.clone(),
                max_tokens: None,
            };
            let response = call_with_retry(llm_config, request).await
                .map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("LLM provider error: {}", e)))?;
            let choice = response.choices.first()
                .ok_or_else(|| NexusError::new(ErrorCode::ExecutionFailed, "LLM returned empty choices array"))?;

            match choice.finish_reason.as_str() {
                "stop" => {
                    let reply = choice.message.content.clone().unwrap_or_default();
                    let _ = crate::db::save_message(&state.db, session_id, "assistant", &reply, None, None, None).await;
                    return Ok(TurnResult { reply, media: pending_media });
                }
                "tool_calls" => {
                    current_tool_calls = parse_tool_calls(choice);
                    info!("execute_tool_calls_loop: after soft error, LLM requested {} new tool calls", current_tool_calls.len());
                    continue;
                }
                _ => {
                    return Err(NexusError::new(ErrorCode::ExecutionFailed, format!("unknown finish_reason after soft error: {}", choice.finish_reason)));
                }
            }
        }

        // Execute all tool calls concurrently
        let mut futures = Vec::new();
        for tc in current_tool_calls.clone() {
            let state = state.clone();
            let user_id = user_id.to_string();
            let session_id = session_id.to_string();
            let channel = event_channel.to_string();
            let chat_id = event_chat_id.to_string();
            futures.push(tokio::spawn(async move {
                let result = execute_single_tool(&state, &user_id, &session_id, &channel, &chat_id, &tc).await;
                (tc, result)
            }));
        }

        let results = futures::future::join_all(futures).await;
        for join_result in results {
            let (tc, result) = join_result.map_err(|e| NexusError::new(ErrorCode::InternalError, format!("task join error: {}", e)))?;
            let (mut content, media) = match result {
                Ok((output, media)) => (output, media),
                Err(e) => {
                    let mut metadata = HashMap::new();
                    metadata.insert("_progress".to_string(), serde_json::json!(true));
                    metadata.insert("_error".to_string(), serde_json::json!(true));
                    let _ = state.bus.publish_outbound(OutboundEvent {
                        channel: event_channel.to_string(),
                        chat_id: event_chat_id.to_string(),
                        content: format!("⚠️ Tool `{}` error: {}", tc.name, e),
                        media: Vec::new(),
                        metadata,
                    }).await;
                    (serde_json::json!({"error": e.to_string()}).to_string(), vec![])
                }
            };
            // Add media from server tools to pending_media
            for m in media {
                let file_name = m.split('/').last().unwrap_or(&m);
                content = format!("File '{}' has been sent to the user.", file_name);
                pending_media.push(m);
            }
            // Save tool result to DB
            let _ = crate::db::save_message(
                &state.db, session_id, "tool", &content,
                Some(&tc.id), None, None,
            ).await;
            current_messages.push(json!({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": content
            }));
        }

        // Save checkpoint after tool batch (for crash recovery)
        let _ = crate::db::save_checkpoint(
            &state.db, session_id, user_id,
            &current_messages, iteration, event_channel, event_chat_id,
        ).await;

        emit_progress(state, event_channel, event_chat_id, "⏳ Analyzing results...").await;

        let request = ChatCompletionRequest {
            messages: current_messages.clone(),
            tools: tools.clone(),
            model: llm_config.model.clone(),
            max_tokens: None,
        };
        let response = call_with_retry(llm_config, request).await
            .map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("LLM provider error: {}", e)))?;
        let choice = response.choices.first()
            .ok_or_else(|| NexusError::new(ErrorCode::ExecutionFailed, "LLM returned empty choices array"))?;

        match choice.finish_reason.as_str() {
            "stop" => {
                emit_progress(state, event_channel, event_chat_id, "💬 Composing response...").await;
                let reply = choice.message.content.clone().unwrap_or_default();
                let _ = crate::db::save_message(&state.db, session_id, "assistant", &reply, None, None, None).await;
                // Clear checkpoint on successful completion
                let _ = crate::db::delete_checkpoint(&state.db, session_id).await;
                return Ok(TurnResult { reply, media: pending_media });
            }
            "tool_calls" => {
                current_tool_calls = parse_tool_calls(choice);
                info!("execute_tool_calls_loop: LLM requested {} new tool calls", current_tool_calls.len());
            }
            _ => {
                return Err(NexusError::new(ErrorCode::ExecutionFailed, format!("unknown finish_reason in tool loop: {}", choice.finish_reason)));
            }
        }
    }
}

async fn execute_single_tool(
    state: &Arc<AppState>,
    user_id: &str,
    session_id: &str,
    event_channel: &str,
    event_chat_id: &str,
    tc: &ToolCallParsed,
) -> Result<(String, Vec<String>), NexusError> {
    debug!("execute_single_tool: tool_name={}, arguments={}", tc.name, tc.arguments);

    // 1. Determine tool location for progress hint
    let location = if state.server_tools.get(&tc.name).is_some() {
        "server".to_string()
    } else if tc.arguments.get("device_name").and_then(|v| v.as_str()) == Some(SERVER_DEVICE_NAME) && tc.name.starts_with("mcp_") {
        SERVER_DEVICE_NAME.to_string()
    } else {
        tc.arguments.get("device_name").and_then(|v| v.as_str()).unwrap_or("unknown").to_string()
    };

    // 2. Emit progress hint (ALWAYS, for all tool types)
    emit_progress(state, event_channel, event_chat_id,
        &format!("🔧 {} on {}", tc.name, location)).await;

    // Server-native tools: return output + media paths
    if let Some(tool) = state.server_tools.get(&tc.name) {
        let result = tool.execute(state, user_id, session_id, tc.arguments.clone(), event_channel, event_chat_id).await?;
        return Ok((result.output, result.media));
    }

    let device_name = tc.arguments
        .get("device_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| NexusError::new(ErrorCode::ToolInvalidParams, "device_name not found in tool call arguments"))?
        .to_string();
    info!("execute_single_tool: resolved device_name={}", device_name);

    // Server MCP tools (device_name=SERVER_DEVICE_NAME)
    if device_name == SERVER_DEVICE_NAME && tc.name.starts_with("mcp_") {
        let manager = state.server_mcp.read().await;
        let output = manager.call_tool(&tc.name, tc.arguments.clone()).await?;
        return Ok((output, vec![]));
    }

    let params = tc.arguments.clone();
    let request_id = tc.id.clone();

    info!("execute_single_tool: calling route_tool for device={}", device_name);
    let result = route_tool(state, user_id, &tc.name, params, &request_id).await?;
    let exit_code = result.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(1);
    let output = result.get("output").and_then(|v| v.as_str()).unwrap_or("");
    if exit_code == 0 {
        Ok((output.to_string(), vec![]))
    } else {
        Err(NexusError::new(ErrorCode::ExecutionFailed, output.to_string()))
    }
}
