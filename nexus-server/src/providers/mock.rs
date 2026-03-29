//! Mock LLM Provider — 两轮状态机，输出标准 OpenAI Chat Completions 格式。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

static CALL_COUNT: AtomicU64 = AtomicU64::new(0);

/// Module-level counter for simulating 429 / rate-limit errors in tests.
/// When > 0, that many calls to `chat_completion` will return a 429 error response.
/// Use `set_mock_transient_errors(n)` to set, `clear_mock_transient_errors()` to reset.
static TRANSIENT_ERRORS_REMAINING: AtomicUsize = AtomicUsize::new(0);

/// 设置接下来 N 次调用返回 429 错误（用于测试重试逻辑）
pub fn set_mock_transient_errors(n: usize) {
    TRANSIENT_ERRORS_REMAINING.store(n, Ordering::SeqCst);
}

/// 清除 429 模拟，恢复正常调用
pub fn clear_mock_transient_errors() {
    TRANSIENT_ERRORS_REMAINING.store(0, Ordering::SeqCst);
}

/// 尝试消耗一次 429 模拟机会，返回是否应返回 429 错误
fn consume_transient_error() -> bool {
    loop {
        let current = TRANSIENT_ERRORS_REMAINING.load(Ordering::SeqCst);
        if current == 0 {
            return false;
        }
        match TRANSIENT_ERRORS_REMAINING.compare_exchange(
            current, current - 1, Ordering::SeqCst, Ordering::SeqCst,
        ) {
            Ok(_) => return current > 0,
            Err(_) => continue,
        }
    }
}

/// OpenAI Chat Completions 请求（agents 调用时构造）
#[derive(Debug, Clone, Deserialize)]
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

/// 从 tools schema 中提取第一个 device_name enum 值。
/// tools schema 中每个工具都有 device_name 参数（由 build_tools_schema 注入）。
fn extract_first_device_name(tools: &[Value]) -> Option<String> {
    for tool in tools {
        if let Some(Value::Object(func)) = tool.get("function") {
            if let Some(Value::Object(params)) = func.get("parameters") {
                if let Some(Value::Object(props)) = params.get("properties") {
                    if let Some(Value::Object(device_name_param)) = props.get("device_name") {
                        if let Some(arr) = device_name_param.get("enum").and_then(|v| v.as_array()) {
                            if let Some(first) = arr.first() {
                                return first.as_str().map(String::from);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// 从 tools schema 中提取第一个工具的名称。
/// arguments 简化为 {} — Mock 不实际执行命令，arguments 内容不重要。
fn extract_first_tool_name(tools: &[Value]) -> Option<(String, Value)> {
    for tool in tools {
        if let Some(Value::Object(func)) = tool.get("function") {
            if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                return Some((name.to_string(), json!({})));
            }
        }
    }
    None
}

/// 生成 call id
fn next_call_id() -> String {
    format!("call_{}", uuid::Uuid::new_v4().to_string().replace("-", ""))
}

/// 构造一个 429 Too Many Requests 错误响应
fn make_429_response(model: &str) -> ChatCompletionResponse {
    let call_num = CALL_COUNT.fetch_add(1, Ordering::SeqCst);
    ChatCompletionResponse {
        id: format!("chatcmpl-mock-{}", call_num + 1),
        model: model.to_string(),
        choices: vec![Choice {
            index: 0,
            message: AssistantMessage {
                role: "assistant".to_string(),
                content: Some("Error: 429 Too Many Requests — rate limit exceeded, please retry after a short wait.".to_string()),
                tool_calls: None,
            },
            finish_reason: "error".to_string(),
            tool_calls: None,
        }],
        usage: Some(json!({
            "prompt_tokens": 10,
            "completion_tokens": 20,
            "total_tokens": 30
        })),
    }
}

/// 处理一轮 Chat Completions 请求
pub fn chat_completion(request: ChatCompletionRequest) -> ChatCompletionResponse {
    // 模拟 429 错误（用于测试重试逻辑）
    if consume_transient_error() {
        return make_429_response(&request.model);
    }

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

    // 第1轮：无 tool_result
    // 如果有 tools schema，从中提取 device_name 和工具名，构建带 device_name 的 tool_call
    if !request.tools.is_empty() {
        if let Some(device_name) = extract_first_device_name(&request.tools) {
            if let Some((tool_name, tool_args)) = extract_first_tool_name(&request.tools) {
                let tool_call_id = next_call_id();

                // 构建带 device_name 的 arguments
                let mut arguments = tool_args.as_object().cloned().unwrap_or_default();
                arguments.insert("device_name".to_string(), json!(device_name));

                return ChatCompletionResponse {
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
                                    name: tool_name.clone(),
                                    arguments: json!(arguments).to_string(),
                                },
                            }]),
                        },
                        finish_reason: "tool_calls".to_string(),
                        tool_calls: Some(vec![ToolCall {
                            index: Some(0),
                            id: tool_call_id,
                            typ: "function".to_string(),
                            function: FunctionCall {
                                name: tool_name,
                                arguments: json!(arguments).to_string(),
                            },
                        }]),
                    }],
                    usage: Some(json!({
                        "prompt_tokens": 80,
                        "completion_tokens": 30,
                        "total_tokens": 110
                    })),
                };
            }
        }

        // tools 非空但无法解析，返回 stop 提示
        return ChatCompletionResponse {
            id: response_id,
            model: request.model,
            choices: vec![Choice {
                index: 0,
                message: AssistantMessage {
                    role: "assistant".to_string(),
                    content: Some("No tools available with valid device routing.".to_string()),
                    tool_calls: None,
                },
                finish_reason: "stop".to_string(),
                tool_calls: None,
            }],
            usage: Some(json!({
                "prompt_tokens": 50,
                "completion_tokens": 10,
                "total_tokens": 60
            })),
        };
    }

    // tools 为空：返回文本，不尝试 tool_call
    ChatCompletionResponse {
        id: response_id,
        model: request.model,
        choices: vec![Choice {
            index: 0,
            message: AssistantMessage {
                role: "assistant".to_string(),
                content: Some("No tools are currently registered. Please wait for devices to connect and register their tools.".to_string()),
                tool_calls: None,
            },
            finish_reason: "stop".to_string(),
            tool_calls: None,
        }],
        usage: Some(json!({
            "prompt_tokens": 40,
            "completion_tokens": 20,
            "total_tokens": 60
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_round_with_tools_returns_tool_call_with_device_name() {
        let tools = vec![
            json!({
                "type": "function",
                "function": {
                    "name": "shell",
                    "description": "Run a shell command",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "device_name": {
                                "type": "string",
                                "enum": ["DASHU", "mac-mini"],
                                "description": "The target device"
                            },
                            "command": {
                                "type": "string",
                                "description": "The shell command"
                            }
                        },
                        "required": ["device_name", "command"]
                    }
                }
            })
        ];
        let request = ChatCompletionRequest {
            messages: vec![
                json!({"role": "system", "content": "You are a helpful assistant."}),
                json!({"role": "user", "content": "run ls"}),
            ],
            tools,
            model: "mock".to_string(),
        };
        let response = chat_completion(request);
        assert_eq!(response.choices[0].finish_reason, "tool_calls");
        let tc = &response.choices[0].tool_calls.as_ref().unwrap()[0];
        assert_eq!(tc.function.name, "shell");
        let args: serde_json::Value = serde_json::from_str(&tc.function.arguments).unwrap();
        assert_eq!(args.get("device_name").and_then(|v| v.as_str()), Some("DASHU"));
    }

    #[test]
    fn test_first_round_without_tools_returns_text() {
        let request = ChatCompletionRequest {
            messages: vec![
                json!({"role": "system", "content": "You are a helpful assistant."}),
                json!({"role": "user", "content": "list files"}),
            ],
            tools: vec![],
            model: "mock".to_string(),
        };
        let response = chat_completion(request);
        assert_eq!(response.choices[0].finish_reason, "stop");
        assert!(response.choices[0].message.content.as_ref().unwrap().contains("No tools"));
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

    #[test]
    fn test_429_error_response() {
        set_mock_transient_errors(1);
        let request = ChatCompletionRequest {
            messages: vec![],
            tools: vec![],
            model: "mock".to_string(),
        };
        let request2 = request.clone();
        let response = chat_completion(request);
        assert_eq!(response.choices[0].finish_reason, "error");
        assert!(response.choices[0].message.content.as_ref().unwrap().contains("429"));
        // Next call should be normal
        let response2 = chat_completion(request2);
        assert_eq!(response2.choices[0].finish_reason, "stop");
        clear_mock_transient_errors();
    }
}
