/// 职责边界：
/// 1. 负责在本地启动并连接第三方 MCP Server（通过 stdio 子进程交互）。
/// 2. 初始化时发送 `tools/list` 获取外部工具，并在命名前加上 `mcp_` 前缀（命名隔离）。
/// 3. 收到执行请求时，将请求透传给对应的 MCP Server 并返回结果。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Child;
use std::process::Stdio;
use tokio::time::{timeout, Duration};

use crate::config::{McpServerConfig, TransportType};
use crate::tools::ToolError;

/// MCP JSON-RPC 请求
#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

/// MCP JSON-RPC 响应
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[serde(default)]
    jsonrpc: Option<String>,
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

/// MCP Server 会话。
pub struct McpSession {
    server_name: String,
    child: Child,
    request_id: u64,
    tool_name_map: HashMap<String, String>,
    tool_timeout: u64,
}

impl McpSession {
    /// 连接 MCP Server 并完成初始化握手。
    pub async fn connect(config: &McpServerConfig) -> Result<Self, String> {
        let tool_timeout = config.tool_timeout.unwrap_or(30);

        let child = tokio::process::Command::new(&config.command)
            .args(&config.args)
            .envs(config.env.as_ref().unwrap_or(&HashMap::new()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to spawn MCP server '{}': {}", config.name, e))?;

        let mut session = Self {
            server_name: config.name.clone(),
            child,
            request_id: 0,
            tool_name_map: HashMap::new(),
            tool_timeout,
        };

        session.initialize().await?;
        Ok(session)
    }

    /// 发送 JSON-RPC 请求并等待响应。
    async fn send_request(&mut self, method: &str, params: Option<Value>) -> Result<Value, String> {
        self.request_id += 1;
        let id = self.request_id;

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };

        let request_json = serde_json::to_string(&request)
            .map_err(|e| format!("failed to serialize request: {}", e))?;

        // 获取 stdin
        if let Some(ref mut stdin) = self.child.stdin {
            stdin
                .write_all(request_json.as_bytes())
                .await
                .map_err(|e| format!("failed to write to MCP stdin: {}", e))?;
            stdin
                .write_all(b"\n")
                .await
                .map_err(|e| format!("failed to write newline: {}", e))?;
        }

        // 读取响应
        let deadline = Duration::from_secs(30);
        let mut buf = Vec::new();
        if let Some(ref mut stdout) = self.child.stdout {
            let n = timeout(deadline, stdout.read_to_end(&mut buf))
                .await
                .map_err(|_| "timeout reading MCP stdout".to_string())?
                .map_err(|e| format!("failed to read MCP stdout: {}", e))?;
            if n == 0 {
                return Err("MCP stdout closed".to_string());
            }
        }

        let response_str = String::from_utf8(buf)
            .map_err(|e| format!("failed to decode MCP stdout as UTF-8: {}", e))?;

        let lines: Vec<&str> = response_str.lines().collect();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let resp: JsonRpcResponse = serde_json::from_str(line)
                .map_err(|e| format!("failed to parse MCP response: {} (line: {})", e, line))?;
            if resp.id == Some(id) {
                if let Some(err) = resp.error {
                    return Err(format!("MCP error {}: {}", err.code, err.message));
                }
                return resp.result.ok_or_else(|| "no result in MCP response".to_string());
            }
        }

        Err("no matching response from MCP server".to_string())
    }

    /// MCP 初始化握手。
    async fn initialize(&mut self) -> Result<(), String> {
        let params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "nexus-client",
                "version": "0.1.0"
            }
        });

        self.send_request("initialize", Some(params)).await?;
        let _ = self.send_notification("initialized", Some(json!({}))).await;
        Ok(())
    }

    /// 发送 JSON-RPC 通知（无响应）。
    async fn send_notification(&mut self, method: &str, params: Option<Value>) -> Result<(), String> {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });

        let json_str = serde_json::to_string(&notification)
            .map_err(|e| format!("failed to serialize notification: {}", e))?;

        if let Some(ref mut stdin) = self.child.stdin {
            stdin.write_all(json_str.as_bytes()).await.ok();
            stdin.write_all(b"\n").await.ok();
        }
        Ok(())
    }

    /// 列出所有可用工具。
    pub async fn list_tools(&mut self) -> Result<Vec<Value>, String> {
        let result = self.send_request("tools/list", None).await?;

        let tools = result
            .get("tools")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut schemas = Vec::new();
        for tool in tools {
            let original_name = tool
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let wrapped_name = format!("mcp_{}_{}", self.server_name, original_name);
            self.tool_name_map.insert(wrapped_name.clone(), original_name);

            let description = tool
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let input_schema = tool
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}}));

            // 规范化 schema，参考 nanobot _normalize_schema_for_openai()
            let normalized_schema = normalize_schema_for_openai(&input_schema);

            let schema = json!({
                "type": "function",
                "function": {
                    "name": wrapped_name,
                    "description": description,
                    "parameters": normalized_schema
                }
            });
            schemas.push(schema);
        }

        Ok(schemas)
    }

    /// 调用 MCP 工具。
    pub async fn call_tool(
        &mut self,
        wrapped_name: &str,
        arguments: Value,
    ) -> Result<String, ToolError> {
        let original_name = self
            .tool_name_map
            .get(wrapped_name)
            .ok_or_else(|| ToolError::NotFound(format!("MCP tool not found: {}", wrapped_name)))?
            .clone();

        let params = json!({
            "name": original_name,
            "arguments": arguments
        });

        let result = timeout(
            Duration::from_secs(self.tool_timeout),
            self.send_request("tools/call", Some(params)),
        )
        .await
        .map_err(|_| ToolError::Timeout(self.tool_timeout))?
        .map_err(|e| ToolError::ExecutionFailed(format!("MCP tool call failed: {}", e)))?;

        let output = parse_call_result(&result);
        Ok(output)
    }
}

/// 从 oneOf/anyOf 的选项列表中提取"单个非 null 分支"。
/// 参考 nanobot: `_extract_nullable_branch()`
fn extract_nullable_branch(options: &[Value]) -> Option<(Value, bool)> {
    let mut non_null_items: Vec<&Value> = Vec::new();
    let mut saw_null = false;

    for option in options {
        if let Some(obj) = option.as_object() {
            if obj.get("type").and_then(|t| t.as_str()) == Some("null") {
                saw_null = true;
                continue;
            }
            non_null_items.push(option);
        } else {
            return None;
        }
    }

    if saw_null && non_null_items.len() == 1 {
        Some((non_null_items[0].clone(), true))
    } else {
        None
    }
}

/// 规范化 MCP schema 中 LLM 不友好的模式：
/// - `type: [string, null]` → `type: string, nullable: true`
/// - `oneOf`/`anyOf` 提取单个非 null 分支并合并，设置 `nullable: true`
/// - 递归规范化 `properties` 和 `items`
/// 参考 nanobot: `_normalize_schema_for_openai()`
fn normalize_schema_for_openai(schema: &Value) -> Value {
    let mut result = schema.clone();

    // 处理 type 字段为列表的情况: [type, null] → { type, nullable: true }
    if let Some(arr) = result.get("type").and_then(|t| t.as_array()) {
        let non_null: Vec<&str> = arr
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|s| *s != "null")
            .collect();
        if arr.iter().any(|v| v.as_str() == Some("null")) && non_null.len() == 1 {
            let single_type = non_null[0].to_string();
            let nullable = true;
            // 构建新的对象，替换 type 和添加 nullable
            if let Some(obj) = result.as_object_mut() {
                obj.insert("type".to_string(), serde_json::Value::String(single_type));
                obj.insert("nullable".to_string(), serde_json::Value::Bool(nullable));
            }
        }
    }

    // 处理 oneOf / anyOf: 提取单个非 null 分支，合并属性，设置 nullable
    for key in &["oneOf", "anyOf"] {
        if let Some(options) = result.get(*key).and_then(|v| v.as_array()) {
            if let Some((branch, _)) = extract_nullable_branch(options) {
                if let Some(branch_obj) = branch.as_object() {
                    // 收集要合并的条目
                    let mut merged: serde_json::Map<String, Value> = serde_json::Map::new();
                    // 先复制 result 的现有字段（除了 oneOf/anyOf）
                    if let Some(result_obj) = result.as_object() {
                        for (k, v) in result_obj {
                            if *key != k {
                                merged.insert(k.clone(), v.clone());
                            }
                        }
                    }
                    // 再合并 branch 的字段
                    for (k, v) in branch_obj {
                        if !merged.contains_key(k) {
                            merged.insert(k.clone(), v.clone());
                        }
                    }
                    merged.insert("nullable".to_string(), serde_json::Value::Bool(true));
                    result = serde_json::Value::Object(merged);
                }
                break;
            }
        }
    }

    // 递归规范化 properties
    if let Some(props) = result.get("properties").and_then(|p| p.as_object()) {
        let mut new_props = serde_json::Map::new();
        for (name, prop) in props {
            if prop.is_object() || prop.is_array() {
                new_props.insert(name.clone(), normalize_schema_for_openai(prop));
            } else {
                new_props.insert(name.clone(), prop.clone());
            }
        }
        if let Some(obj) = result.as_object_mut() {
            obj.insert("properties".to_string(), serde_json::Value::Object(new_props));
        }
    }

    // 递归规范化 items
    if let Some(items) = result.get("items") {
        if items.is_object() || items.is_array() {
            let normalized_items = normalize_schema_for_openai(items);
            if let Some(obj) = result.as_object_mut() {
                obj.insert("items".to_string(), normalized_items);
            }
        }
    }

    // 确保 object 类型有 properties 和 required 字段
    if result.get("type").and_then(|t| t.as_str()) == Some("object") {
        if let Some(obj) = result.as_object_mut() {
            obj.entry("properties").or_insert(serde_json::Value::Object(serde_json::Map::new()));
            obj.entry("required").or_insert(serde_json::Value::Array(Vec::new()));
        }
    }

    result
}

/// 解析 MCP tools/call 结果。
fn parse_call_result(result: &Value) -> String {
    use std::fmt::Write;

    let content = result.get("content").and_then(|v| v.as_array());

    let mut output = String::new();
    if let Some(items) = content {
        for item in items {
            match item.get("type").and_then(|v| v.as_str()) {
                Some("text") => {
                    if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                        writeln!(output, "{}", text).ok();
                    }
                }
                Some("image") => {
                    let mime = item.get("mimeType").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let size = item.get("data").and_then(|v| v.as_str()).map(|s| s.len()).unwrap_or(0);
                    writeln!(output, "[Image: {}, size={}]", mime, size).ok();
                }
                Some("resource") => {
                    if let Some(uri) = item.get("resource").and_then(|r| r.get("uri")).and_then(|v| v.as_str()) {
                        writeln!(output, "[Resource: {}]", uri).ok();
                    }
                }
                _ => {
                    writeln!(output, "{:?}", item).ok();
                }
            }
        }
    }

    if output.is_empty() {
        String::new()
    } else {
        output.trim().to_string()
    }
}

/// MCP 客户端全局管理器。
pub struct McpClientManager {
    sessions: HashMap<String, McpSession>,
}

impl McpClientManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// 初始化所有 MCP 服务器。
    pub async fn initialize(&mut self, servers: &[McpServerConfig]) -> Result<(), String> {
        for server in servers.iter().filter(|s| s.enabled) {
            if server.transport_type != TransportType::Stdio {
                tracing::warn!(
                    "MCP server '{}': transport '{:?}' not implemented yet, skipping",
                    server.name,
                    server.transport_type
                );
                continue;
            }

            match McpSession::connect(server).await {
                Ok(mut session) => {
                    match session.list_tools().await {
                        Ok(schemas) => {
                            tracing::info!(
                                "MCP server '{}': discovered {} tools",
                                server.name,
                                schemas.len()
                            );
                        }
                        Err(e) => {
                            tracing::warn!("MCP server '{}': failed to list tools: {}", server.name, e);
                        }
                    }
                    self.sessions.insert(server.name.clone(), session);
                }
                Err(e) => {
                    tracing::warn!("MCP server '{}': failed to connect: {}", server.name, e);
                }
            }
        }
        Ok(())
    }

    /// 调用 MCP 工具。
    pub async fn call_tool(
        &mut self,
        server_name: &str,
        wrapped_name: &str,
        arguments: Value,
    ) -> Result<String, ToolError> {
        let session = self
            .sessions
            .get_mut(server_name)
            .ok_or_else(|| ToolError::NotFound(format!("MCP server not found: {}", server_name)))?;
        session.call_tool(wrapped_name, arguments).await
    }

    /// 获取所有已连接服务器的名称。
    pub fn server_names(&self) -> Vec<&str> {
        self.sessions.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for McpClientManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 从工具名解析出 server_name 和 original_name。
/// 工具名格式: mcp_{server_name}_{original_tool_name}
pub fn parse_mcp_tool_name(tool_name: &str) -> Option<(&str, &str)> {
    let prefix = "mcp_";
    if !tool_name.starts_with(prefix) {
        return None;
    }
    let rest = &tool_name[prefix.len()..];
    let underscore_count = rest.matches('_').count();

    if underscore_count == 0 {
        // 没有下划线，无法分隔
        return None;
    }

    // tool_name 部分不能包含下划线（否则无法唯一确定分隔位置）
    // 因此：如果有多于1个下划线，用 rsplit（从右找）；只有1个，用 split
    let (server_name_part, tool_name_part) = if underscore_count > 1 {
        let parts: Vec<&str> = rest.rsplitn(2, '_').collect();
        if parts.len() != 2 {
            return None;
        }
        (parts[1], parts[0])
    } else {
        let parts: Vec<&str> = rest.splitn(2, '_').collect();
        if parts.len() != 2 {
            return None;
        }
        (parts[0], parts[1])
    };

    if tool_name_part.contains('_') {
        return None;
    }
    // server_name 必须至少包含一个下划线、短横线，或长度>=4（排除 "no" 这类片段）
    if !server_name_part.contains('_')
        && !server_name_part.contains('-')
        && server_name_part.len() < 4
    {
        return None;
    }
    Some((server_name_part, tool_name_part))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mcp_tool_name() {
        assert_eq!(
            parse_mcp_tool_name("mcp_filesystem_read"),
            Some(("filesystem", "read"))
        );
        assert_eq!(
            parse_mcp_tool_name("mcp_my-server_listDir"),
            Some(("my-server", "listDir"))
        );
        assert_eq!(parse_mcp_tool_name("shell"), None);
        assert_eq!(parse_mcp_tool_name("mcp_no_underscore"), None);
    }

    #[test]
    fn test_parse_call_result_text() {
        let result = json!({
            "content": [
                {"type": "text", "text": "hello world"}
            ]
        });
        assert_eq!(parse_call_result(&result), "hello world");
    }

    #[test]
    fn test_parse_call_result_empty() {
        let result = json!({});
        assert_eq!(parse_call_result(&result), "");
    }

    #[test]
    fn test_normalize_nullable_type() {
        // type: [string, null] → type: string, nullable: true
        let schema = json!({
            "type": ["string", "null"]
        });
        let normalized = normalize_schema_for_openai(&schema);
        assert_eq!(normalized.get("type").and_then(|v| v.as_str()), Some("string"));
        assert_eq!(normalized.get("nullable").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn test_normalize_oneof_nullable() {
        // oneOf with single non-null branch → merged with nullable: true
        let schema = json!({
            "oneOf": [
                {"type": "null"},
                {
                    "type": "object",
                    "properties": {"path": {"type": "string"}}
                }
            ]
        });
        let normalized = normalize_schema_for_openai(&schema);
        assert_eq!(normalized.get("nullable").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(normalized.get("type").and_then(|v| v.as_str()), Some("object"));
        assert!(normalized.get("properties").is_some());
        assert!(!normalized.get("oneOf").is_some()); // oneOf removed
    }

    #[test]
    fn test_normalize_anyof_nullable() {
        // anyOf with single non-null branch → merged with nullable: true
        let schema = json!({
            "anyOf": [
                {"type": "null"},
                {"type": "string"}
            ]
        });
        let normalized = normalize_schema_for_openai(&schema);
        assert_eq!(normalized.get("nullable").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(normalized.get("type").and_then(|v| v.as_str()), Some("string"));
    }

    #[test]
    fn test_normalize_nested_properties() {
        // nested properties should be recursively normalized
        let schema = json!({
            "type": "object",
            "properties": {
                "config": {
                    "type": ["object", "null"],
                    "properties": {
                        "enabled": {"type": "boolean"}
                    }
                }
            }
        });
        let normalized = normalize_schema_for_openai(&schema);
        let config = normalized
            .get("properties")
            .and_then(|p| p.get("config"))
            .and_then(|c| c.as_object())
            .expect("config should be an object");
        assert_eq!(config.get("type").and_then(|v| v.as_str()), Some("object"));
        assert_eq!(config.get("nullable").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn test_normalize_object_has_required() {
        // object types should have required field defaulting to []
        let schema = json!({
            "type": "object",
            "properties": {"name": {"type": "string"}}
        });
        let normalized = normalize_schema_for_openai(&schema);
        assert!(normalized.get("required").is_some());
    }

    #[test]
    fn test_normalize_passthrough_simple() {
        // simple schema without nullable/oneOf should pass through unchanged
        let schema = json!({
            "type": "string",
            "description": "a simple string"
        });
        let normalized = normalize_schema_for_openai(&schema);
        assert_eq!(normalized.get("type").and_then(|v| v.as_str()), Some("string"));
        assert_eq!(normalized.get("description").and_then(|v| v.as_str()), Some("a simple string"));
    }
}
