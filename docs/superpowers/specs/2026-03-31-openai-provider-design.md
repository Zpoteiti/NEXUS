# OpenAI-Compatible LLM Provider

**Date:** 2026-03-31
**Status:** Approved
**Goal:** Replace mock LLM with real OpenAI-compatible HTTP provider to unblock M3 acceptance.

## Architecture

```
nexus-server (agent_loop)
  → async call_with_retry()
    → openai::chat_completion()  [reqwest POST]
      → MiniMax API / LiteLLM / any OpenAI-compatible endpoint
```

## Design Decisions

1. **Delete mock.rs** — Mock provider served its purpose for M2 E2E testing. No longer needed.
2. **Delete LlmResponse abstraction** — agent_loop uses `ChatCompletionResponse` directly. Single provider format (OpenAI), no need for an intermediate type.
3. **Strip `<think>` tags** — MiniMax-M2.7 is a reasoning model that emits `<think>...</think>` in content. Strip these before returning to agent_loop so downstream code only sees the final reply.
4. **Async all the way** — `call_with_retry` becomes async with `tokio::time::sleep`. Prevents blocking tokio worker threads during HTTP calls and retry backoff.
5. **Fallback-free** — No mock fallback, no feature flags. If `LLM_API_KEY` is missing, server fails to start with a clear error.

## Changes

### 1. `Cargo.toml`

Add dependency:
```toml
reqwest = { version = "0.12", features = ["json"] }
```

### 2. `providers/mod.rs`

Move shared types here from mock.rs:
- `ChatCompletionRequest` — `messages: Vec<Value>`, `tools: Vec<Value>`, `model: String`
- `ChatCompletionResponse` — `id`, `model`, `choices: Vec<Choice>`, `usage`
- `Choice` — `index`, `message: AssistantMessage`, `finish_reason`
- `AssistantMessage` — `role`, `content: Option<String>`, `tool_calls: Option<Vec<ToolCall>>`
- `ToolCall` — `index`, `id`, `type`, `function: FunctionCall`
- `FunctionCall` — `name`, `arguments: String` (JSON string, matching OpenAI format)

Remove:
- `LlmResponse`, `ToolCallRequest` — no longer needed
- `mod mock` — deleted
- `is_transient_error` stays (used by retry logic)

`call_with_retry` becomes:
```rust
pub async fn call_with_retry(
    config: &LlmConfig,
    request: ChatCompletionRequest,
) -> Result<ChatCompletionResponse, ProviderError>
```

Retry logic: up to 3 attempts, exponential backoff (1s, 2s, 4s) on 429/5xx/timeout.

### 3. `providers/openai.rs`

Single async function:
```rust
pub async fn chat_completion(
    config: &LlmConfig,
    request: ChatCompletionRequest,
) -> Result<ChatCompletionResponse, ProviderError>
```

- POST to `{config.api_base}/chat/completions`
- Header: `Authorization: Bearer {config.api_key}`
- On success: deserialize response, strip `<think>...</think>` from content
- On HTTP error: return `ProviderError` with status code for retry classification

### 4. `config.rs`

New fields in `ServerConfig`:
```rust
pub struct LlmConfig {
    pub api_base: String,    // hardcoded for now
    pub api_key: String,     // hardcoded for now
    pub model: String,       // hardcoded for now
}
```

All three hardcoded initially. Will be made configurable via admin API later.

### 5. `agent_loop.rs`

- Remove `openai_to_llm_response` function
- Remove all references to `LlmResponse` and `ToolCallRequest`
- `call_with_retry` calls gain `.await`
- `model` read from `LlmConfig.model` instead of hardcoded `"mock"`
- Tool call extraction reads directly from `ChatCompletionResponse.choices[0].tool_calls`
- Tool call arguments parsed from JSON string to Value inline where needed

### 6. Delete `providers/mock.rs`

Entire file removed.

## Configuration

Hardcoded in `config.rs` for now:
```rust
api_base: "https://api.minimaxi.com/v1"
api_key: "sk-cp-..."  // committed as hardcoded, admin API will manage later
model: "MiniMax-M2.7"
```

## Error Type

```rust
pub enum ProviderError {
    HttpError { status: u16, body: String },
    NetworkError(String),
    ParseError(String),
}
```

`is_transient` method on ProviderError: true for status 429, 5xx, and network errors.

## Verification

After implementation, verify with the existing E2E flow:
1. Start MiniMax-backed nexus-server + nexus-gateway + nexus-client
2. Browser WS sends message
3. Agent loop calls MiniMax → receives tool_calls → routes to client → gets result → final reply
4. Confirm M3 acceptance criteria #1 (full session log sequence)
