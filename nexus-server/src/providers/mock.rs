//! Mock LLM Provider — 两轮状态机，输出标准 OpenAI Chat Completions 格式。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};

static CALL_COUNT: AtomicU64 = AtomicU64::new(0);

/// OpenAI Chat Completions 请求（agents 调用时构造）
#[derive(Debug, Deserialize)]
pub struct ChatCompletionRequest {
    pub messages: Vec<Value>,
    pub tools: Vec<Value>,
    #[serde(default)]
    pub model: String,
}

/// OpenAI Chat Completions 响应（标准格式）
#[derive(Debug, Serialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub model: String,
    pub choices: Vec<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct Choice {
    pub index: usize,
    pub message: AssistantMessage,
    #[serde(rename = "finish_reason")]
    pub finish_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Serialize)]
pub struct AssistantMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Serialize)]
pub struct ToolCall {
    pub index: Option<usize>,
    pub id: String,
    #[serde(rename = "type")]
    pub typ: String,
    pub function: FunctionCall,
}

#[derive(Debug, Serialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// 检查最后一条消息是否是 tool_result
fn is_tool_result_round(messages: &[Value]) -> bool {
    messages
        .last()
        .and_then(|m| m.get("role"))
        .and_then(|r| r.as_str())
        == Some("tool")
}

/// 生成 call id
fn next_call_id() -> String {
    format!("call_{}", uuid::Uuid::new_v4().to_string().replace("-", ""))
}

/// 处理一轮 Chat Completions 请求
pub fn chat_completion(request: ChatCompletionRequest) -> ChatCompletionResponse {
    let call_num = CALL_COUNT.fetch_add(1, Ordering::SeqCst);
    let response_id = format!("chatcmpl-mock-{}", call_num + 1);

    // 第2轮：带 tool_result → 返回 stop
    if is_tool_result_round(&request.messages) {
        let tool_content = request
            .messages
            .last()
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("(empty)");

        let display = format!("已执行工具，结果如下：\n{}", tool_content);

        return ChatCompletionResponse {
            id: response_id,
            model: request.model,
            choices: vec![Choice {
                index: 0,
                message: AssistantMessage {
                    role: "assistant".to_string(),
                    content: Some(display),
                    tool_calls: None,
                },
                finish_reason: "stop".to_string(),
                tool_calls: None,
            }],
            usage: Some(json!({
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "total_tokens": 150
            })),
        };
    }

    // 第1轮：返回 list_dir tool_call
    let tool_call_id = next_call_id();
    ChatCompletionResponse {
        id: response_id,
        model: request.model,
        choices: vec![Choice {
            index: 0,
            message: AssistantMessage {
                role: "assistant".to_string(),
                content: None,
                tool_calls: Some(vec![ToolCall {
                    index: Some(0),
                    id: tool_call_id.clone(),
                    typ: "function".to_string(),
                    function: FunctionCall {
                        name: "list_dir".to_string(),
                        arguments: json!({ "path": "." }).to_string(),
                    },
                }]),
            },
            finish_reason: "tool_calls".to_string(),
            tool_calls: Some(vec![ToolCall {
                index: Some(0),
                id: tool_call_id,
                typ: "function".to_string(),
                function: FunctionCall {
                    name: "list_dir".to_string(),
                    arguments: json!({ "path": "." }).to_string(),
                },
            }]),
        }],
        usage: Some(json!({
            "prompt_tokens": 80,
            "completion_tokens": 30,
            "total_tokens": 110
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_first_round_returns_tool_call() {
        let request = ChatCompletionRequest {
            messages: vec![
                json!({"role": "system", "content": "You are a helpful assistant."}),
                json!({"role": "user", "content": "list files"}),
            ],
            tools: vec![],
            model: "mock".to_string(),
        };
        let response = chat_completion(request);
        assert_eq!(response.choices[0].finish_reason, "tool_calls");
        let tc = &response.choices[0].tool_calls.as_ref().unwrap()[0];
        assert_eq!(tc.function.name, "list_dir");
    }

    #[test]
    fn test_second_round_returns_stop() {
        let request = ChatCompletionRequest {
            messages: vec![
                json!({"role": "system", "content": "You are a helpful assistant."}),
                json!({"role": "user", "content": "list files"}),
                json!({
                    "role": "tool",
                    "tool_call_id": "call_abc",
                    "content": "file1.txt\nfile2.rs"
                }),
            ],
            tools: vec![],
            model: "mock".to_string(),
        };
        let response = chat_completion(request);
        assert_eq!(response.choices[0].finish_reason, "stop");
        assert!(response.choices[0].message.content.as_ref().unwrap().starts_with("已执行工具"));
    }
}