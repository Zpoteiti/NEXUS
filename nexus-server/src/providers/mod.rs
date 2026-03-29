use serde_json::Value;

pub mod mock;
pub use mock::{chat_completion, set_mock_transient_errors, clear_mock_transient_errors, ChatCompletionRequest, ChatCompletionResponse};

use tracing::warn;

/// LLM 标准返回结构（跨 provider 统一）
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCallRequest>,
    pub finish_reason: String,
}

/// 单次工具调用请求（由 LLM 生成）
#[derive(Debug, Clone)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

// ============================================================================
// 瞬时错误检测与重试
// ============================================================================

/// 瞬时错误关键字（与 nanobot providers/base.py 保持一致）
const _TRANSIENT_ERROR_MARKERS: &[&str] = &[
    "429",
    "rate limit",
    "500",
    "502",
    "503",
    "504",
    "overloaded",
    "timeout",
    "timed out",
];

/// 重试退避延迟（秒）：指数退避 1s, 2s, 4s
const RETRY_DELAYS: &[u64] = &[1, 2, 4];

/// 检测响应内容是否包含瞬时错误
fn is_transient_error(content: &str) -> bool {
    let lower = content.to_lowercase();
    _TRANSIENT_ERROR_MARKERS.iter().any(|m| lower.contains(m))
}

/// 调用 LLM 并自动重试瞬时错误（429、5xx、timeout 等）。
///
/// 当 `chat_completion` 返回的 `finish_reason == "error"` 且内容包含瞬时错误标记时，
/// 按照指数退避重试最多 3 次。
///
/// Real LLM providers (HTTP-based) should use this wrapper when calling the API,
/// so that 429 / rate-limit errors are handled transparently.
pub fn call_with_retry(request: ChatCompletionRequest) -> ChatCompletionResponse {
    let mut attempt = 0;
    let max_attempts = 1 + RETRY_DELAYS.len(); // 1 次 + 3 次重试 = 4 次

    loop {
        let response = chat_completion(request.clone());

        // 非错误响应，直接返回
        if response.choices.first().map(|c| &c.finish_reason) != Some(&"error".to_string()) {
            return response;
        }

        // 检查是否是瞬时错误
        let content = response
            .choices
            .first()
            .and_then(|c| c.message.content.as_ref())
            .map(|s| s.as_str())
            .unwrap_or("");

        if !is_transient_error(content) {
            // 非瞬时错误（如 invalid request），不重试，直接返回
            return response;
        }

        if attempt >= RETRY_DELAYS.len() {
            // 重试次数用尽，返回最后一次响应
            warn!(
                "LLM transient error: all {} retries exhausted, last error: {}",
                RETRY_DELAYS.len(),
                &content[..content.len().min(100)]
            );
            return response;
        }

        let delay = RETRY_DELAYS[attempt];
        warn!(
            "LLM transient error (attempt {}/{}), retrying in {}s: {}",
            attempt + 1,
            max_attempts,
            delay,
            &content[..content.len().min(100)]
        );
        std::thread::sleep(std::time::Duration::from_secs(delay));
        attempt += 1;
    }
}
