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

/// Check if any message contains image_url content blocks
fn has_image_content(messages: &[Value]) -> bool {
    messages.iter().any(|m| {
        if let Some(content) = m.get("content") {
            if let Some(arr) = content.as_array() {
                return arr.iter().any(|part| {
                    part.get("type").and_then(|t| t.as_str()) == Some("image_url")
                });
            }
        }
        false
    })
}

/// Strip image content blocks, replacing with text placeholders
fn strip_image_content(mut request: ChatCompletionRequest) -> ChatCompletionRequest {
    for msg in &mut request.messages {
        let arr = match msg.get("content").and_then(|c| c.as_array()) {
            Some(arr) => arr.clone(),
            None => continue,
        };

        // Replace array content: keep text blocks, replace image blocks with placeholder
        let new_content: Vec<Value> = arr
            .iter()
            .map(|part| {
                if part.get("type").and_then(|t| t.as_str()) == Some("image_url") {
                    serde_json::json!({"type": "text", "text": "[image omitted]"})
                } else {
                    part.clone()
                }
            })
            .collect();

        // If all parts are text after stripping, simplify to string
        let texts: Vec<String> = new_content
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()).map(String::from))
            .collect();

        let replacement = if texts.len() == new_content.len() {
            serde_json::json!(texts.join("\n"))
        } else {
            serde_json::json!(new_content)
        };

        if let Some(c) = msg.get_mut("content") {
            *c = replacement;
        }
    }
    request
}

pub async fn call_with_retry(
    config: &LlmConfig,
    request: ChatCompletionRequest,
) -> Result<ChatCompletionResponse, ProviderError> {
    match call_with_retry_inner(config, request.clone()).await {
        Ok(resp) => Ok(resp),
        Err(e) if !e.is_transient() && has_image_content(&request.messages) => {
            warn!(
                "LLM error with image content, retrying without images: {}",
                e
            );
            let stripped = strip_image_content(request);
            call_with_retry_inner(config, stripped).await
        }
        Err(e) => Err(e),
    }
}

async fn call_with_retry_inner(
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
                        warn!("LLM call failed after {} retries: {}", attempt, e);
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
