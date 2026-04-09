//! Server-native tool registry and dispatch.
//! Tools execute on the server, not on client devices.

use crate::state::AppState;
use nexus_common::protocol::ToolExecutionResult;
use serde_json::Value;
use std::sync::Arc;

pub mod cron_tool;
pub mod file_transfer;
pub mod memory;
pub mod message;
pub mod skills;
pub mod web_fetch;

/// All server tool names.
pub const SERVER_TOOL_NAMES: &[&str] = &[
    "save_memory",
    "edit_memory",
    "message",
    "file_transfer",
    "cron",
    "read_skill",
    "install_skill",
    "web_fetch",
];

/// Check if a tool name is a server-native tool.
pub fn is_server_tool(name: &str) -> bool {
    SERVER_TOOL_NAMES.contains(&name)
}

/// Return JSON schemas for all 8 server tools.
pub fn tool_schemas() -> Vec<Value> {
    vec![
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "save_memory",
                "description": "Save text to persistent memory (replaces current memory). 4K char max.",
                "parameters": {
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "edit_memory",
                "description": "Edit persistent memory: append, prepend, or replace content.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "operation": { "type": "string", "enum": ["append", "prepend", "replace"] },
                        "text": { "type": "string" }
                    },
                    "required": ["operation", "text"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "message",
                "description": "Send a message to a channel (gateway or discord), optionally with media files from a device.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "content": { "type": "string" },
                        "channel": { "type": "string", "enum": ["gateway", "discord"] },
                        "chat_id": { "type": "string" },
                        "media": { "type": "array", "items": { "type": "string" } },
                        "from_device": { "type": "string" }
                    },
                    "required": ["content", "channel"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "file_transfer",
                "description": "Transfer a file between devices. Server acts as relay.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "from_device": { "type": "string" },
                        "to_device": { "type": "string" },
                        "file_path": { "type": "string" }
                    },
                    "required": ["from_device", "to_device", "file_path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "cron",
                "description": "Manage scheduled tasks: add, list, or remove cron jobs.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["add", "list", "remove"] },
                        "name": { "type": "string" },
                        "message": { "type": "string" },
                        "cron_expr": { "type": "string" },
                        "every_seconds": { "type": "integer" },
                        "at": { "type": "string" },
                        "timezone": { "type": "string" },
                        "channel": { "type": "string" },
                        "chat_id": { "type": "string" },
                        "delete_after_run": { "type": "boolean" },
                        "deliver": { "type": "boolean" },
                        "job_id": { "type": "string" }
                    },
                    "required": ["action"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_skill",
                "description": "Read the full instructions of an on-demand skill.",
                "parameters": {
                    "type": "object",
                    "properties": { "skill_name": { "type": "string" } },
                    "required": ["skill_name"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "install_skill",
                "description": "Install a skill from a GitHub repo (fetches SKILL.md).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "repo": { "type": "string", "description": "owner/repo" },
                        "branch": { "type": "string" }
                    },
                    "required": ["repo"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "web_fetch",
                "description": "Fetch a URL and return readable content. SSRF-protected.",
                "parameters": {
                    "type": "object",
                    "properties": { "url": { "type": "string" } },
                    "required": ["url"]
                }
            }
        }),
    ]
}

/// Session context passed to server tools for channel/chat routing.
pub struct ToolContext {
    pub user_id: String,
    pub session_id: String,
    pub channel: String,
    pub chat_id: Option<String>,
    pub is_cron: bool,
}

/// Dispatch a server tool call. Returns exit_code + output.
pub async fn execute(
    state: &Arc<AppState>,
    ctx: &ToolContext,
    tool_name: &str,
    arguments: Value,
) -> ToolExecutionResult {
    let request_id = uuid::Uuid::new_v4().to_string();
    let (exit_code, output) = match tool_name {
        "save_memory" => memory::save_memory(state, &ctx.user_id, &arguments).await,
        "edit_memory" => memory::edit_memory(state, &ctx.user_id, &arguments).await,
        "web_fetch" => web_fetch::web_fetch(state, &ctx.user_id, &arguments).await,
        "message" => message::message_tool(state, ctx, &arguments).await,
        "file_transfer" => file_transfer::file_transfer(state, &ctx.user_id, &arguments).await,
        "cron" => cron_tool::cron(state, ctx, &arguments).await,
        "read_skill" => skills::read_skill(state, &ctx.user_id, &arguments).await,
        "install_skill" => skills::install_skill(state, &ctx.user_id, &arguments).await,
        _ => (1, format!("Unknown server tool: {tool_name}")),
    };
    ToolExecutionResult {
        request_id,
        exit_code,
        output,
    }
}
