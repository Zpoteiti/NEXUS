//! Per-Session Agent Loop
//! 每个 session 有独立的实例，消费自己的 inbox queue，不与其他 session 共享

use crate::bus::{InboundEvent, OutboundEvent};
use crate::context;
use crate::providers::{call_with_retry, ChatCompletionRequest};
use crate::state::AppState;
use crate::tools_registry::{route_tool, RouteError};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

/// Result of a single turn, including the reply text and any media file paths.
struct TurnResult {
    reply: String,
    media: Vec<String>,
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
        info!("agent_session {} received: content={}", session_id, event.content);

        // 持有 session 锁（防止不同 channel 同时写数据库）
        let lock = state.session_manager.get_session_lock(&session_id).await
            .expect("session lock must exist after get_or_create_session");
        let _guard = lock.read().await;

        match run_single_turn(&state, &event).await {
            Ok(result) => {
                let outbound = OutboundEvent {
                    channel: event.channel.clone(),
                    chat_id: event.chat_id.clone(),
                    content: result.reply,
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
) -> Result<TurnResult, String> {
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

    info!("agent_session {} calling LLM with {} tools", session_id, tools.len());
    let request = ChatCompletionRequest {
        messages: messages.clone(),
        tools: tools.clone(),
        model: llm_config.model.clone(),
        max_tokens: None,
    };
    let response = call_with_retry(&llm_config, request).await
        .map_err(|e| format!("LLM provider error: {}", e))?;
    let choice = response.choices.first()
        .ok_or_else(|| "LLM returned empty choices array".to_string())?;
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
        _ => Err(format!("unknown finish_reason: {}", choice.finish_reason)),
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
) -> Result<TurnResult, String> {
    let mut current_messages = messages.clone();
    let mut current_tool_calls = initial_tool_calls;
    let mut call_counts: HashMap<ToolCallKey, usize> = HashMap::new();
    let mut gave_rethink_chance = false;
    let mut pending_media: Vec<String> = Vec::new();

    loop {
        for tc in &current_tool_calls {
            // Save assistant tool_call to DB
            let _ = crate::db::save_message(
                &state.db, session_id, "assistant", "",
                Some(&tc.id), Some(&tc.name), Some(&tc.arguments.to_string()),
            ).await;
            current_messages.push(json!({
                "role": "assistant",
                "tool_calls": [{
                    "id": tc.id,
                    "type": "function",
                    "function": { "name": tc.name, "arguments": tc.arguments.to_string() }
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
                tools: tools.clone(),
                model: llm_config.model.clone(),
                max_tokens: None,
            };
            let response = call_with_retry(llm_config, request).await
                .map_err(|e| format!("LLM provider error: {}", e))?;
            let choice = response.choices.first()
                .ok_or_else(|| "LLM returned empty choices array".to_string())?;

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
                    return Err(format!("unknown finish_reason after soft error: {}", choice.finish_reason));
                }
            }
        }

        for tc in &current_tool_calls {
            // Emit progress: tool call intent
            {
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

            let result = execute_single_tool(state, user_id, session_id, tc).await;
            let mut content = match result {
                Ok(output) => output,
                Err(e) => {
                    // Emit error progress
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

        let request = ChatCompletionRequest {
            messages: current_messages.clone(),
            tools: tools.clone(),
            model: llm_config.model.clone(),
        };
        let response = call_with_retry(llm_config, request).await
            .map_err(|e| format!("LLM provider error: {}", e))?;
        let choice = response.choices.first()
            .ok_or_else(|| "LLM returned empty choices array".to_string())?;

        match choice.finish_reason.as_str() {
            "stop" => {
                let reply = choice.message.content.clone().unwrap_or_default();
                let _ = crate::db::save_message(&state.db, session_id, "assistant", &reply, None, None, None).await;
                return Ok(TurnResult { reply, media: pending_media });
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
    session_id: &str,
    tc: &ToolCallParsed,
) -> Result<String, String> {
    info!("execute_single_tool: tool_name={}, arguments={}", tc.name, tc.arguments);

    // ── Built-in server-side tool: send_file ──
    if tc.name == "send_file" {
        let device_name = tc.arguments.get("device_name")
            .and_then(|v| v.as_str())
            .ok_or("send_file: missing device_name")?
            .to_string();
        let file_path = tc.arguments.get("file_path")
            .and_then(|v| v.as_str())
            .ok_or("send_file: missing file_path")?
            .to_string();

        // Find the device
        let device_key = {
            let by_user = state.devices_by_user.read().await;
            by_user.get(user_id)
                .and_then(|devices| devices.get(&device_name).cloned())
                .ok_or_else(|| format!("device '{}' not found", device_name))?
        };

        let ws_tx = {
            let devices = state.devices.read().await;
            devices.get(&device_key)
                .map(|d| d.ws_tx.clone())
                .ok_or_else(|| format!("device '{}' not connected", device_name))?
        };

        // Create oneshot channel for file upload response
        let request_id = format!("{}:{}", device_key, uuid::Uuid::new_v4());
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut pending = state.file_upload_pending.write().await;
            pending.insert(request_id.clone(), tx);
        }

        // Send FileUploadRequest to client
        use nexus_common::protocol::{ServerToClient, FileUploadRequest};
        let msg = ServerToClient::FileUploadRequest(FileUploadRequest {
            request_id: request_id.clone(),
            file_path: file_path.clone(),
        });
        let msg_text = serde_json::to_string(&msg)
            .map_err(|e| format!("serialize error: {}", e))?;
        ws_tx.send(axum::extract::ws::Message::Text(msg_text.into())).await
            .map_err(|e| format!("ws send error: {}", e))?;

        // Wait for response with timeout
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            rx,
        ).await
            .map_err(|_| "file upload timed out after 60s".to_string())?
            .map_err(|_| "file upload channel closed (device may have disconnected)".to_string())?;

        // Check for error
        if let Some(err) = response.error {
            return Err(format!("file upload failed: {}", err));
        }

        // Decode base64 and save to temp
        let bytes = base64::engine::general_purpose::STANDARD.decode(&response.content_base64)
            .map_err(|e| format!("base64 decode error: {}", e))?;

        let dir = std::path::Path::new("/tmp/nexus-media");
        tokio::fs::create_dir_all(dir).await
            .map_err(|e| format!("mkdir error: {}", e))?;
        let save_path = dir.join(format!("{}_{}", uuid::Uuid::new_v4(), response.file_name));
        tokio::fs::write(&save_path, &bytes).await
            .map_err(|e| format!("write error: {}", e))?;

        // Return special marker so the tool loop can attach the file to the outbound event
        return Ok(format!("__FILE__:{}", save_path.to_string_lossy()));
    }

    // ── Built-in server-side tool: save_memory ──
    if tc.name == "save_memory" {
        let content = tc.arguments.get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if content.is_empty() {
            return Err("save_memory: content is empty".to_string());
        }
        // Generate embedding if configured
        let embedding = {
            let emb_config = state.config.embedding.read().await.clone();
            if let Some(ref cfg) = emb_config {
                let emb = crate::context::embed_text_throttled(cfg, &content, &state.embedding_semaphore).await;
                if emb.is_empty() { None } else { Some(emb) }
            } else {
                None
            }
        };
        // Dedup: skip if a very similar memory already exists
        if let Some(ref emb) = embedding {
            match crate::db::find_similar_memory(&state.db, user_id, emb, 0.92).await {
                Ok(true) => {
                    tracing::info!("save_memory: skipping duplicate (cosine > 0.92)");
                    return Ok("Memory already exists (similar content found).".to_string());
                }
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!("save_memory: dedup check failed: {}, proceeding with save", e);
                }
            }
        }
        match crate::db::save_memory_chunk(
            &state.db, session_id, user_id,
            &format!("[{}] Agent saved: {}", chrono::Utc::now().format("%Y-%m-%d %H:%M"), &content[..content.len().min(80)]),
            &content,
            embedding.as_deref(),
        ).await {
            Ok(()) => return Ok("Memory saved successfully.".to_string()),
            Err(e) => return Err(format!("Failed to save memory: {}", e)),
        }
    }

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
        Err(RouteError::SendFailed(name)) => Err(format!("failed to send request to '{}'", name)),
    }
}
