//! Per-session ReAct agent loop: LLM call → tool dispatch → iterate.

use crate::bus::{InboundEvent, OutboundEvent};
use crate::context::{self, ChannelIdentity, SkillInfo};
use crate::providers::openai::{self, ChatMessage, LlmResponse};
use crate::server_tools::{self, ToolContext};
use crate::state::AppState;
use nexus_common::consts::{MAX_AGENT_ITERATIONS, USER_MESSAGE_MAX_CHARS};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

pub async fn run_session(
    state: Arc<AppState>,
    session_id: String,
    user_id: String,
    mut inbox: mpsc::Receiver<InboundEvent>,
) {
    info!("Agent loop started for session {session_id}");

    while let Some(event) = inbox.recv().await {
        // Acquire session lock (prevent concurrent DB writes)
        let lock = {
            let handle = state.sessions.get(&session_id).unwrap();
            Arc::clone(&handle.lock)
        };
        let _guard = lock.lock().await;

        if let Err(e) = handle_event(&state, &session_id, &user_id, event).await {
            error!("Session {session_id} error: {e}");
        }
    }

    info!("Agent loop ended for session {session_id}");
    state.sessions.remove(&session_id);
}

async fn handle_event(
    state: &Arc<AppState>,
    session_id: &str,
    user_id: &str,
    event: InboundEvent,
) -> Result<(), String> {
    // Load user
    let user = crate::db::users::find_by_id(&state.db, user_id)
        .await
        .map_err(|e| format!("Load user: {e}"))?
        .ok_or("User not found")?;

    // Large message conversion: >4K chars → save full to file, inline first 4K
    let content = if event.content.len() > USER_MESSAGE_MAX_CHARS {
        let file_id =
            crate::file_store::save_upload(user_id, "large_message.txt", event.content.as_bytes())
                .await
                .map_err(|e| format!("Save large message: {}", e.message))?;
        format!(
            "{}\n\n[Full message saved as file: /api/files/{file_id}]",
            &event.content[..USER_MESSAGE_MAX_CHARS]
        )
    } else {
        event.content.clone()
    };

    // Save user message to DB
    let msg_id = uuid::Uuid::new_v4().to_string();
    crate::db::messages::insert(
        &state.db,
        &msg_id,
        session_id,
        nexus_common::consts::ROLE_USER,
        &content,
        None,
        None,
        None,
    )
    .await
    .map_err(|e| format!("Save user message: {e}"))?;

    // Use channel-provided identity, or default to owner for gateway/cron
    let identity = event
        .identity
        .clone()
        .unwrap_or_else(|| ChannelIdentity::gateway_owner(&user));

    // Build tool context for server tools
    let tool_ctx = ToolContext {
        user_id: user_id.to_string(),
        session_id: session_id.to_string(),
        channel: event.channel.clone(),
        chat_id: event.chat_id.clone(),
        is_cron: event.cron_job_id.is_some(),
    };

    // Load skills
    let skills = load_skills(state, user_id).await;

    // Cache default soul for this session (avoids RwLock contention per iteration)
    let cached_default_soul = state.default_soul.read().await.clone();

    // Loop detection state
    let mut call_counts: HashMap<String, u32> = HashMap::new();

    // Load history once, append incrementally (avoids re-querying DB on every iteration)
    let mut history = crate::db::messages::list_uncompressed(&state.db, session_id)
        .await
        .map_err(|e| format!("Load history: {e}"))?;
    let mut needs_reload = false;

    // Agent iteration loop
    for iteration in 0..MAX_AGENT_ITERATIONS {
        if needs_reload {
            history = crate::db::messages::list_uncompressed(&state.db, session_id)
                .await
                .map_err(|e| format!("Reload history: {e}"))?;
            needs_reload = false;
        }
        let history_ref = &history;

        // Build tool schemas
        let tool_schemas = crate::tools_registry::build_tool_schemas(state, user_id);

        // Build context
        let messages = context::build_context(
            state,
            &user,
            &event,
            history_ref,
            &skills,
            &tool_schemas,
            &identity,
            &cached_default_soul,
        );

        // Check compression
        let llm_config = state.llm_config.read().await;
        let Some(config) = llm_config.as_ref() else {
            send_error(
                state,
                &event,
                session_id,
                user_id,
                "LLM not configured. Admin must set LLM config via API.",
            )
            .await;
            return Ok(());
        };
        let config = config.clone();
        drop(llm_config);

        let token_estimate = context::estimate_tokens(&messages);
        if (config.context_window as usize).saturating_sub(token_estimate)
            < nexus_common::consts::CONTEXT_COMPRESSION_THRESHOLD
        {
            // Compress history then reload
            crate::memory::compress(state, session_id, &history, &config).await;
            needs_reload = true;
            continue;
        }

        // Call LLM
        let tools = if tool_schemas.is_empty() {
            None
        } else {
            Some(tool_schemas.clone())
        };

        let response = openai::call_llm(&state.http_client, &config, messages, tools).await;

        match response {
            Ok(LlmResponse::Text(text)) => {
                // Save assistant message
                let msg_id = uuid::Uuid::new_v4().to_string();
                crate::db::messages::insert(
                    &state.db,
                    &msg_id,
                    session_id,
                    "assistant",
                    &text,
                    None,
                    None,
                    None,
                )
                .await
                .map_err(|e| format!("Save assistant message: {e}"))?;

                // Send final response
                let _ = state
                    .outbound_tx
                    .send(OutboundEvent {
                        channel: event.channel.clone(),
                        chat_id: event.chat_id.clone(),
                        session_id: session_id.to_string(),
                        user_id: user_id.to_string(),
                        content: text,
                        media: vec![],
                        is_progress: false,
                        metadata: Default::default(),
                    })
                    .await;

                return Ok(());
            }
            Ok(LlmResponse::ToolCalls(tool_calls)) => {
                // Save assistant message with tool_calls
                for tc in &tool_calls {
                    let msg_id = uuid::Uuid::new_v4().to_string();
                    crate::db::messages::insert(
                        &state.db,
                        &msg_id,
                        session_id,
                        "assistant",
                        "",
                        Some(&tc.id),
                        Some(&tc.function.name),
                        Some(&tc.function.arguments),
                    )
                    .await
                    .map_err(|e| format!("Save tool call: {e}"))?;
                }

                // Execute each tool call
                for tc in &tool_calls {
                    // Send human-friendly progress hint BEFORE execution
                    let progress_hint = build_tool_hint(&tc.function.name, &tc.function.arguments);
                    let _ = state
                        .outbound_tx
                        .send(OutboundEvent {
                            channel: event.channel.clone(),
                            chat_id: event.chat_id.clone(),
                            session_id: session_id.to_string(),
                            user_id: user_id.to_string(),
                            content: progress_hint,
                            media: vec![],
                            is_progress: true,
                            metadata: Default::default(),
                        })
                        .await;

                    // Loop detection
                    let call_key = format!("{}:{}", tc.function.name, tc.function.arguments);
                    let count = call_counts.entry(call_key).or_insert(0);
                    *count += 1;

                    let result_output = if *count >= 4 {
                        warn!(
                            "Loop detected: {} called 4+ times with same args, stopping",
                            tc.function.name
                        );
                        format!(
                            "Error: Tool '{}' called too many times with identical arguments. Stopping to prevent infinite loop.",
                            tc.function.name
                        )
                    } else if *count >= 3 {
                        format!(
                            "Warning: Tool '{}' has been called 3 times with identical arguments. Consider a different approach.",
                            tc.function.name
                        )
                    } else {
                        // Execute the tool
                        let result = execute_tool_call(
                            state,
                            user_id,
                            &tc.function.name,
                            &tc.function.arguments,
                            &tool_ctx,
                        )
                        .await;
                        result.output
                    };

                    // Save tool result to DB
                    let result_msg_id = uuid::Uuid::new_v4().to_string();
                    crate::db::messages::insert(
                        &state.db,
                        &result_msg_id,
                        session_id,
                        "tool",
                        &result_output,
                        Some(&tc.id),
                        None,
                        None,
                    )
                    .await
                    .map_err(|e| format!("Save tool result: {e}"))?;

                    // (progress hint already sent before execution)

                    // Hard stop on loop detection
                    if *call_counts
                        .get(&format!("{}:{}", tc.function.name, tc.function.arguments))
                        .unwrap_or(&0)
                        >= 4
                    {
                        send_error(
                            state,
                            &event,
                            session_id,
                            user_id,
                            "Agent stopped: infinite loop detected.",
                        )
                        .await;
                        return Ok(());
                    }
                }

                // Continue to next iteration (LLM call with updated history)
                info!(
                    "Session {session_id}: iteration {iteration}, {} tool calls",
                    tool_calls.len()
                );
            }
            Err(e) => {
                error!("LLM error in session {session_id}: {e}");
                send_error(
                    state,
                    &event,
                    session_id,
                    user_id,
                    &format!("LLM error: {e}"),
                )
                .await;
                return Ok(());
            }
        }
    }

    // Max iterations reached
    warn!("Session {session_id}: max iterations ({MAX_AGENT_ITERATIONS}) reached");
    send_error(
        state,
        &event,
        session_id,
        user_id,
        &format!("Reached maximum iterations ({MAX_AGENT_ITERATIONS}). Stopping."),
    )
    .await;

    Ok(())
}

/// Build a human-friendly progress hint for a tool call.
fn build_tool_hint(tool_name: &str, arguments_json: &str) -> String {
    let args: serde_json::Value = serde_json::from_str(arguments_json)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    let device = args
        .get("device_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if server_tools::is_server_tool(tool_name) {
        // Server tools: no device
        format!("Executing {tool_name}...")
    } else if device.is_empty() {
        format!("Executing {tool_name}...")
    } else {
        format!("Executing {tool_name} on {device}...")
    }
}

async fn execute_tool_call(
    state: &Arc<AppState>,
    user_id: &str,
    tool_name: &str,
    arguments_json: &str,
    tool_ctx: &ToolContext,
) -> nexus_common::protocol::ToolExecutionResult {
    let args: serde_json::Value = serde_json::from_str(arguments_json)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    // Server tools first (no device_name)
    if server_tools::is_server_tool(tool_name) {
        return server_tools::execute(state, tool_ctx, tool_name, args).await;
    }

    // Client/MCP tools: extract device_name
    let device_name = match args.get("device_name").and_then(|v| v.as_str()) {
        Some(d) => d.to_string(),
        None => {
            return nexus_common::protocol::ToolExecutionResult {
                request_id: String::new(),
                exit_code: 1,
                output: format!("Missing device_name for tool '{tool_name}'"),
            };
        }
    };

    // TODO: server MCP routing (device_name == "server" for MCP)

    // Route to client device
    crate::tools_registry::route_to_device(state, user_id, &device_name, tool_name, args).await
}

async fn load_skills(state: &AppState, user_id: &str) -> Vec<SkillInfo> {
    let db_skills = crate::db::skills::list_by_user(&state.db, user_id)
        .await
        .unwrap_or_default();

    let mut skills = Vec::new();
    for skill in db_skills {
        let content = tokio::fs::read_to_string(format!("{}/SKILL.md", skill.skill_path))
            .await
            .unwrap_or_default();
        skills.push(SkillInfo {
            name: skill.name,
            description: skill.description,
            always_on: skill.always_on,
            content,
        });
    }
    skills
}

async fn send_error(
    state: &AppState,
    event: &InboundEvent,
    session_id: &str,
    user_id: &str,
    message: &str,
) {
    // Save error as assistant message
    let msg_id = uuid::Uuid::new_v4().to_string();
    let _ = crate::db::messages::insert(
        &state.db,
        &msg_id,
        session_id,
        "assistant",
        message,
        None,
        None,
        None,
    )
    .await;

    // Send to channel
    let _ = state
        .outbound_tx
        .send(OutboundEvent {
            channel: event.channel.clone(),
            chat_id: event.chat_id.clone(),
            session_id: session_id.to_string(),
            user_id: user_id.to_string(),
            content: message.to_string(),
            media: vec![],
            is_progress: false,
            metadata: Default::default(),
        })
        .await;
}
