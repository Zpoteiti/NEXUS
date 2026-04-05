//! Per-Session Agent Loop
//! 每个 session 有独立的实例，消费自己的 inbox queue，不与其他 session 共享

use crate::bus::{InboundEvent, OutboundEvent};
use crate::context;
use crate::providers::{call_with_retry, ChatCompletionRequest};
use crate::state::AppState;
use crate::tools_registry::route_tool;
use base64::Engine;
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

/// Detect image MIME type from file path (extension first, then magic bytes).
fn detect_image_mime(path: &str) -> Option<&'static str> {
    let lower = path.to_lowercase();
    if lower.ends_with(".png") {
        Some("image/png")
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        Some("image/jpeg")
    } else if lower.ends_with(".gif") {
        Some("image/gif")
    } else if lower.ends_with(".webp") {
        Some("image/webp")
    } else if lower.ends_with(".bmp") {
        Some("image/bmp")
    } else {
        // Try magic bytes
        if let Ok(bytes) = std::fs::read(path) {
            if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
                Some("image/png")
            } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
                Some("image/jpeg")
            } else if bytes.starts_with(b"GIF8") {
                Some("image/gif")
            } else if bytes.len() >= 12 && &bytes[8..12] == b"WEBP" {
                Some("image/webp")
            } else {
                None
            }
        } else {
            None
        }
    }
}

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
            if let Err(e) = crate::db::ensure_session(&state.db, &session_id, &event.sender_id).await {
                warn!("failed to create DB session {}: {}", session_id, e);
            }
            db_session_created = true;
        }
        debug!("agent_session {} received: content={}", session_id, event.content);

        // 持有 session 锁（防止不同 channel 同时写数据库）
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
                state.bus.publish_outbound(make_outbound(&event, format!("Error: {}", e))).await;
            }
        }
    }

    state.bus.unregister_session(&session_id);
    state.session_manager.remove_session(&session_id).await;
    info!("agent_session ended and cleaned up: session_id={}", session_id);
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
) -> Result<TurnResult, NexusError> {
    let user_input = &event.content;
    let user_id = &event.sender_id;
    let session_id = &event.session_id;

    let base_llm_config = match state.config.llm.read().await.clone() {
        Some(config) => config,
        None => {
            state.bus.publish_outbound(make_outbound(event,
                "⚠️ LLM not configured. An admin must set up the LLM provider via the API first.".into()
            )).await;
            return Ok(TurnResult { reply: "LLM not configured".into(), media: Vec::new() });
        }
    };
    // Route through LiteLLM proxy
    let llm_config = state.litellm_llm_config(&base_llm_config);

    // Context compression check
    crate::memory::maybe_consolidate(
        session_id,
        &event.sender_id,
        &state.db,
        &llm_config,
        &state.config.embedding,
        &state.embedding_semaphore,
    )
    .await;

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
                let mime = detect_image_mime(path);
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
                    parts.push(json!({"type": "text", "text": format!("[file: {}]", name)}));
                }
            }
        }
        parts.push(json!({"type": "text", "text": user_input}));
        json!({"role": "user", "content": parts})
    };
    messages.push(user_message);

    let _ = crate::db::save_message(&state.db, session_id, "user", user_input, None, None, None).await;

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

/// 执行工具调用循环（无硬上限，通过循环检测打断真正的死循环）
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

        // Emit progress hints for all tool calls
        for tc in &current_tool_calls {
            let mut metadata = HashMap::new();
            metadata.insert("_progress".to_string(), serde_json::json!(true));
            metadata.insert("_tool_hint".to_string(), serde_json::json!(true));
            let hint = format!("🔧 `{}`", tc.name);
            let _ = state.bus.publish_outbound(OutboundEvent {
                channel: event_channel.to_string(),
                chat_id: event_chat_id.to_string(),
                content: hint,
                media: Vec::new(),
                metadata,
            }).await;
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
            let mut content = match result {
                Ok(output) => output,
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
                    format!("{{\"error\": \"{}\"}}", e)
                }
            };
            // Check for file transfer marker and extract media path
            if content.starts_with("__FILE__:") {
                let file_path = content.strip_prefix("__FILE__:").unwrap().to_string();
                let file_name = file_path.split('/').last().unwrap_or(&file_path);
                content = format!("File '{}' has been sent to the user.", file_name);
                pending_media.push(file_path);
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
) -> Result<String, NexusError> {
    debug!("execute_single_tool: tool_name={}, arguments={}", tc.name, tc.arguments);

    // ── Check server-native tools first ──
    if let Some(tool) = state.server_tools.get(&tc.name) {
        let result = tool.execute(state, user_id, session_id, tc.arguments.clone(), event_channel, event_chat_id).await?;
        // If tool produced media, return with __FILE__ markers
        if !result.media.is_empty() {
            return Ok(result.media.join("\n"));
        }
        return Ok(result.output);
    }

    let device_name = tc.arguments
        .get("device_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| NexusError::new(ErrorCode::ToolInvalidParams, "device_name not found in tool call arguments"))?
        .to_string();
    info!("execute_single_tool: resolved device_name={}", device_name);

    // ── Check server MCP tools (device_name="server") ──
    if device_name == "server" && tc.name.starts_with("mcp_") {
        let manager = state.server_mcp.read().await;
        return manager.call_tool(&tc.name, tc.arguments.clone()).await;
    }

    let params = tc.arguments.clone();
    let request_id = tc.id.clone();

    info!("execute_single_tool: calling route_tool for device={}", device_name);
    let result = route_tool(state, user_id, &tc.name, params, &request_id).await?;
    let exit_code = result.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(1);
    let output = result.get("output").and_then(|v| v.as_str()).unwrap_or("");
    if exit_code == 0 {
        Ok(output.to_string())
    } else {
        Err(NexusError::new(ErrorCode::ExecutionFailed, output.to_string()))
    }
}
