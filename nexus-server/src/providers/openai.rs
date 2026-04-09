//! OpenAI-compatible chat completions provider.
//! Single POST {api_base}/chat/completions endpoint.
//! Retry with exponential backoff for 429/5xx. Strips <think> tags.

use crate::config::LlmConfig;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::LazyLock;
use tracing::{debug, warn};

// -- Request/Response types --

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant_text(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: "assistant".into(),
            content: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            name: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            name: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

pub enum LlmResponse {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}

// -- Internal request/response structs --

#[derive(Serialize)]
struct CompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Deserialize)]
struct CompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    content: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,
}

// -- Think tag stripping --

static THINK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<think>.*?</think>").unwrap());

fn strip_think_tags(content: &str) -> String {
    THINK_RE.replace_all(content, "").trim().to_string()
}

// -- Main entry point --

pub async fn call_llm(
    client: &reqwest::Client,
    config: &LlmConfig,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<Value>>,
) -> Result<LlmResponse, String> {
    let url = format!("{}/chat/completions", config.api_base.trim_end_matches('/'));

    let tool_choice = if tools.as_ref().is_some_and(|t| !t.is_empty()) {
        Some("auto".to_string())
    } else {
        None
    };

    let body = CompletionRequest {
        model: config.model.clone(),
        messages,
        tools,
        tool_choice,
    };

    let mut last_error = String::new();

    for attempt in 0..3 {
        if attempt > 0 {
            let delay = 1u64 << attempt; // 1s, 2s, 4s
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            debug!("LLM retry attempt {attempt}");
        }

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                last_error = format!("HTTP error: {e}");
                warn!("LLM request failed: {last_error}");
                continue;
            }
        };

        let status = resp.status().as_u16();

        if status == 429 || status >= 500 {
            last_error = format!("HTTP {status}");
            warn!("LLM transient error: {last_error}");
            continue;
        }

        if status != 200 {
            let body_text = resp.text().await.unwrap_or_default();
            last_error = format!("HTTP {status}: {body_text}");
            warn!("LLM non-transient error: {last_error}");
            // Don't retry non-transient errors
            break;
        }

        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("Read response body: {e}"))?;

        let parsed: CompletionResponse = serde_json::from_str(&body_text).map_err(|e| {
            format!(
                "Parse LLM response: {e}\nBody: {}",
                &body_text[..body_text.len().min(500)]
            )
        })?;

        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or("No choices in LLM response")?;

        // Tool calls take priority over content
        if let Some(tool_calls) = choice.message.tool_calls {
            if !tool_calls.is_empty() {
                return Ok(LlmResponse::ToolCalls(tool_calls));
            }
        }

        if let Some(content) = choice.message.content {
            let cleaned = strip_think_tags(&content);
            return Ok(LlmResponse::Text(cleaned));
        }

        return Err("LLM returned empty response (no content, no tool_calls)".into());
    }

    Err(format!("LLM failed after 3 attempts: {last_error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_think_tags() {
        assert_eq!(strip_think_tags("hello"), "hello");
        assert_eq!(
            strip_think_tags("<think>reasoning here</think>answer"),
            "answer"
        );
        assert_eq!(
            strip_think_tags("<think>line1\nline2</think>\nresult"),
            "result"
        );
        assert_eq!(strip_think_tags("<think>all thinking</think>"), "");
    }

    #[test]
    fn test_chat_message_system() {
        let m = ChatMessage::system("hello");
        assert_eq!(m.role, "system");
        assert_eq!(m.content.unwrap(), "hello");
    }

    #[test]
    fn test_chat_message_serialization() {
        let m = ChatMessage::user("test");
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"role\":\"user\""));
        // Optional None fields should be skipped
        assert!(!json.contains("tool_calls"));
        assert!(!json.contains("tool_call_id"));
    }
}
