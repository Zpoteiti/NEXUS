use reqwest::Client;
use std::sync::LazyLock;
use tracing::debug;

use crate::config::LlmConfig;
use super::{ChatCompletionRequest, ChatCompletionResponse, ProviderError};

static HTTP_CLIENT: LazyLock<Client> = LazyLock::new(|| {
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
