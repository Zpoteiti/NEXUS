# OpenAI-Compatible LLM Provider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace mock LLM with a real OpenAI-compatible HTTP provider targeting MiniMax API, enabling M3 acceptance with real LLM responses and tool calling.

**Architecture:** `agent_loop` calls `async call_with_retry()` which sends HTTP POST to `{api_base}/chat/completions` via reqwest. Response `<think>` tags are stripped. All types live in `providers/mod.rs`; `mock.rs` is deleted.

**Tech Stack:** reqwest 0.12 (HTTP client), serde/serde_json (serialization), tokio (async runtime)

---

## File Structure

| Action | File | Responsibility |
|--------|------|----------------|
| Modify | `nexus-server/Cargo.toml` | Add reqwest dependency |
| Rewrite | `nexus-server/src/providers/mod.rs` | Shared types, ProviderError, async call_with_retry |
| Rewrite | `nexus-server/src/providers/openai.rs` | Async HTTP client for OpenAI-compatible API |
| Delete | `nexus-server/src/providers/mock.rs` | Remove mock provider |
| Modify | `nexus-server/src/config.rs` | Add LlmConfig with hardcoded values |
| Modify | `nexus-server/src/state.rs` | No change needed (config already in AppState via ServerConfig) |
| Modify | `nexus-server/src/agent_loop.rs` | Use async provider, remove LlmResponse abstraction |

---

### Task 1: Add reqwest dependency

**Files:**
- Modify: `nexus-server/Cargo.toml`

- [ ] **Step 1: Add reqwest to dependencies**

Add after the `jsonwebtoken = "9"` line in `nexus-server/Cargo.toml`:

```toml
reqwest = { version = "0.12", features = ["json"] }
```

- [ ] **Step 2: Verify it compiles**

Run:
```bash
cd D:/GitHub/NEXUS && cargo check --package nexus-server
```
Expected: compiles successfully (warnings OK)

- [ ] **Step 3: Commit**

```bash
cd D:/GitHub/NEXUS && git add nexus-server/Cargo.toml && git commit -m "deps: add reqwest for OpenAI HTTP provider"
```

---

### Task 2: Add LlmConfig to config.rs

**Files:**
- Modify: `nexus-server/src/config.rs`

- [ ] **Step 1: Add LlmConfig struct and hardcoded values**

Add at the top of `config.rs`, after the existing `ServerConfig` struct definition (after line 11):

```rust
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub api_base: String,
    pub api_key: String,
    pub model: String,
}
```

Add a new field to `ServerConfig`:

```rust
pub llm: LlmConfig,
```

In `load_config()`, before the final `ServerConfig { ... }` return, add:

```rust
    let llm = LlmConfig {
        api_base: "https://api.minimaxi.com/v1".to_string(),
        api_key: "sk-cp-1BBnG-2Gwn17dP38KWGk9l4nz1ZlB7ozhT-1ol6rhVjjH2bRng7zFTTMg8Mqky51W5KxX9NyKF5vXaklYVDdFmFGDBa9nTmHTEhWZr39K-3g7huPKbvJGoU".to_string(),
        model: "MiniMax-M2.7".to_string(),
    };
```

And add `llm,` to the `ServerConfig` return struct.

- [ ] **Step 2: Verify it compiles**

Run:
```bash
cd D:/GitHub/NEXUS && cargo check --package nexus-server
```
Expected: compiles (warnings OK)

- [ ] **Step 3: Commit**

```bash
cd D:/GitHub/NEXUS && git add nexus-server/src/config.rs && git commit -m "config: add hardcoded LlmConfig for MiniMax API"
```

---

### Task 3: Rewrite providers/mod.rs with shared types and async call_with_retry

**Files:**
- Rewrite: `nexus-server/src/providers/mod.rs`
- Delete: `nexus-server/src/providers/mock.rs`

- [ ] **Step 1: Delete mock.rs**

```bash
rm D:/GitHub/NEXUS/nexus-server/src/providers/mock.rs
```

- [ ] **Step 2: Rewrite mod.rs**

Replace the entire contents of `nexus-server/src/providers/mod.rs` with:

```rust
pub mod openai;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

use crate::config::LlmConfig;

// ============================================================================
// OpenAI Chat Completions types (shared across providers)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub messages: Vec<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Value>,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    pub index: usize,
    pub message: AssistantMessage,
    pub finish_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    pub id: String,
    #[serde(rename = "type")]
    pub typ: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

// ============================================================================
// Provider error
// ============================================================================

#[derive(Debug)]
pub enum ProviderError {
    HttpError { status: u16, body: String },
    NetworkError(String),
    ParseError(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::HttpError { status, body } => write!(f, "HTTP {}: {}", status, body),
            ProviderError::NetworkError(msg) => write!(f, "Network error: {}", msg),
            ProviderError::ParseError(msg) => write!(f, "Parse error: {}", msg),
        }
    }
}

impl ProviderError {
    pub fn is_transient(&self) -> bool {
        match self {
            ProviderError::HttpError { status, .. } => {
                *status == 429 || *status >= 500
            }
            ProviderError::NetworkError(_) => true,
            ProviderError::ParseError(_) => false,
        }
    }
}

// ============================================================================
// Retry wrapper
// ============================================================================

const RETRY_DELAYS: &[u64] = &[1, 2, 4];

pub async fn call_with_retry(
    config: &LlmConfig,
    request: ChatCompletionRequest,
) -> Result<ChatCompletionResponse, ProviderError> {
    let mut last_error: Option<ProviderError> = None;

    for attempt in 0..=RETRY_DELAYS.len() {
        match openai::chat_completion(config, request.clone()).await {
            Ok(response) => return Ok(response),
            Err(e) => {
                if !e.is_transient() || attempt >= RETRY_DELAYS.len() {
                    if attempt > 0 {
                        warn!(
                            "LLM call failed after {} retries: {}",
                            attempt, e
                        );
                    }
                    return Err(e);
                }
                let delay = RETRY_DELAYS[attempt];
                warn!(
                    "LLM transient error (attempt {}/{}), retrying in {}s: {}",
                    attempt + 1,
                    RETRY_DELAYS.len() + 1,
                    delay,
                    e
                );
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap())
}
```

- [ ] **Step 3: Verify it compiles (expect errors from agent_loop.rs — that's OK for now)**

Run:
```bash
cd D:/GitHub/NEXUS && cargo check --package nexus-server 2>&1 | head -20
```
Expected: errors in `agent_loop.rs` referencing removed types — will fix in Task 5.

- [ ] **Step 4: Commit**

```bash
cd D:/GitHub/NEXUS && git add nexus-server/src/providers/mod.rs && git add -u nexus-server/src/providers/mock.rs && git commit -m "providers: rewrite mod.rs with shared types, delete mock.rs"
```

---

### Task 4: Implement openai.rs HTTP client

**Files:**
- Rewrite: `nexus-server/src/providers/openai.rs`

- [ ] **Step 1: Implement openai.rs**

Replace the entire contents of `nexus-server/src/providers/openai.rs` with:

```rust
use reqwest::Client;
use once_cell::sync::Lazy;
use tracing::debug;

use crate::config::LlmConfig;
use super::{ChatCompletionRequest, ChatCompletionResponse, ProviderError};

// Reuse a single reqwest Client across calls (connection pooling)
static HTTP_CLIENT: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .expect("failed to create HTTP client")
});

/// Strip `<think>...</think>` blocks from content (MiniMax reasoning model output).
fn strip_think_tags(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("<think>") {
        result.push_str(&rest[..start]);
        match rest[start..].find("</think>") {
            Some(end) => {
                rest = &rest[start + end + "</think>".len()..];
            }
            None => {
                // Unclosed <think> tag — discard everything after it
                return result.trim().to_string();
            }
        }
    }
    result.push_str(rest);
    result.trim().to_string()
}

pub async fn chat_completion(
    config: &LlmConfig,
    request: ChatCompletionRequest,
) -> Result<ChatCompletionResponse, ProviderError> {
    let url = format!("{}/chat/completions", config.api_base);

    debug!("POST {} model={}", url, request.model);

    let http_response = HTTP_CLIENT
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .json(&request)
        .send()
        .await
        .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

    let status = http_response.status().as_u16();

    if status != 200 {
        let body = http_response
            .text()
            .await
            .unwrap_or_else(|_| "failed to read response body".to_string());
        return Err(ProviderError::HttpError { status, body });
    }

    let body = http_response
        .text()
        .await
        .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

    let mut response: ChatCompletionResponse = serde_json::from_str(&body)
        .map_err(|e| ProviderError::ParseError(format!("{}: {}", e, &body[..body.len().min(200)])))?;

    // Strip <think> tags from assistant content
    for choice in &mut response.choices {
        if let Some(ref content) = choice.message.content {
            let stripped = strip_think_tags(content);
            choice.message.content = if stripped.is_empty() { None } else { Some(stripped) };
        }
    }

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_think_tags_basic() {
        let input = "<think>\nsome reasoning\n</think>\n\nHello!";
        assert_eq!(strip_think_tags(input), "Hello!");
    }

    #[test]
    fn test_strip_think_tags_no_tags() {
        let input = "Hello world";
        assert_eq!(strip_think_tags(input), "Hello world");
    }

    #[test]
    fn test_strip_think_tags_empty_after_strip() {
        let input = "<think>only thinking</think>";
        assert_eq!(strip_think_tags(input), "");
    }

    #[test]
    fn test_strip_think_tags_multiple() {
        let input = "<think>a</think>Hello<think>b</think> world";
        assert_eq!(strip_think_tags(input), "Hello world");
    }

    #[test]
    fn test_strip_think_tags_unclosed() {
        let input = "Before<think>unclosed reasoning";
        assert_eq!(strip_think_tags(input), "Before");
    }
}
```

- [ ] **Step 2: Add once_cell dependency**

`once_cell` is needed for `Lazy`. Add to `nexus-server/Cargo.toml`:

```toml
once_cell = "1"
```

- [ ] **Step 3: Run the unit tests**

```bash
cd D:/GitHub/NEXUS && cargo test --package nexus-server -- providers::openai::tests --nocapture
```
Expected: all 5 tests pass

- [ ] **Step 4: Commit**

```bash
cd D:/GitHub/NEXUS && git add nexus-server/src/providers/openai.rs nexus-server/Cargo.toml && git commit -m "feat: implement OpenAI-compatible HTTP provider with think-tag stripping"
```

---

### Task 5: Refactor agent_loop.rs to use async provider directly

**Files:**
- Modify: `nexus-server/src/agent_loop.rs`

- [ ] **Step 1: Update imports**

Replace lines 1-15 of `agent_loop.rs` with:

```rust
//! Per-Session Agent Loop
//! 每个 session 有独立的实例，消费自己的 inbox queue，不与其他 session 共享

use crate::bus::{InboundEvent, OutboundEvent};
use crate::context;
use crate::providers::{call_with_retry, ChatCompletionRequest, ChatCompletionResponse, ToolCall, FunctionCall};
use crate::state::AppState;
use crate::tools_registry::{route_tool, RouteError};
use serde_json::{json, Value};
use sqlx::Executor;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
```

- [ ] **Step 2: Update run_single_turn to use async provider and ChatCompletionResponse directly**

Replace the `run_single_turn` function (lines 75-127) with:

```rust
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
            let tool_calls = parse_tool_calls(&choice);
            info!("agent_session {} calling execute_tool_calls_loop with {} tool_calls", session_id, tool_calls.len());
            execute_tool_calls_loop(state, user_id, session_id, messages, tool_calls).await
        }
        _ => Err(format!("unknown finish_reason: {}", choice.finish_reason)),
    }
}
```

- [ ] **Step 3: Add parse_tool_calls helper and ToolCallParsed struct**

Add after the `ToolCallKey` definition (after line 144 of original):

```rust
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
```

- [ ] **Step 4: Update execute_tool_calls_loop signature and internals**

Replace the `execute_tool_calls_loop` function. Change `Vec<ToolCallRequest>` to `Vec<ToolCallParsed>`, update all `call_with_retry` calls to be async with config, and remove `openai_to_llm_response`:

```rust
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
        // 1. Append assistant message with tool_calls to history
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

        // 2. Loop detection
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

        // 3a. Loop detected
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

        // 3b. Execute tool calls
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

        // 4. Call LLM with tool results
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
```

- [ ] **Step 5: Update execute_single_tool to use ToolCallParsed**

Replace the `execute_single_tool` function:

```rust
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
```

- [ ] **Step 6: Delete openai_to_llm_response function**

Remove the `openai_to_llm_response` function entirely (was at the end of the file). It is no longer used.

- [ ] **Step 7: Verify full compilation**

```bash
cd D:/GitHub/NEXUS && cargo check --package nexus-server
```
Expected: compiles with no errors

- [ ] **Step 8: Run all tests**

```bash
cd D:/GitHub/NEXUS && cargo test --package nexus-server
```
Expected: `providers::openai::tests` pass. Old mock tests are gone.

- [ ] **Step 9: Commit**

```bash
cd D:/GitHub/NEXUS && git add nexus-server/src/agent_loop.rs && git commit -m "feat: wire agent_loop to async OpenAI provider, remove mock LLM"
```

---

### Task 6: End-to-end smoke test

**Files:** None (manual verification)

- [ ] **Step 1: Start PostgreSQL (if not running)**

Verify PostgreSQL is running with the NEXUS database.

- [ ] **Step 2: Start nexus-server**

```bash
cd D:/GitHub/NEXUS && cargo run --package nexus-server
```
Expected: "Server listening on 0.0.0.0:8080" (or configured port). No panic on startup.

- [ ] **Step 3: Start nexus-gateway**

In a separate terminal:
```bash
cd D:/GitHub/NEXUS && cargo run --package nexus-gateway
```

- [ ] **Step 4: Start nexus-client**

In a separate terminal:
```bash
cd D:/GitHub/NEXUS && cargo run --package nexus-client
```
Expected: connects to server, registers tools, heartbeat starts.

- [ ] **Step 5: Send a test message via browser WebSocket**

Use the existing E2E test script or connect via browser to `ws://localhost:9090/ws/browser` with a valid JWT. Send a message like "list files in the current directory".

Expected log sequence on server:
1. `calling LLM with N tools` — HTTP request sent to MiniMax
2. `LLM returned: finish_reason=tool_calls` — MiniMax returned a tool call
3. `execute_single_tool: tool_name=shell` — routing to client
4. `LLM returned: finish_reason=stop` — final reply received

Expected: browser receives the final reply with actual command output.

- [ ] **Step 6: Commit updated spec**

```bash
cd D:/GitHub/NEXUS && git add docs/superpowers/ && git commit -m "docs: update spec with hardcoded config, add implementation plan"
```
