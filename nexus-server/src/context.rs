//! Build full LLM prompt: system + soul + memory + skills + devices + history.

use crate::db::messages::Message;
use crate::db::users::User;
use crate::providers::openai::{ChatMessage, FunctionCall, ToolCall};
use crate::state::AppState;
use serde_json::Value;

/// Channel-agnostic sender identity for security boundaries.
/// Constructed by each channel (Discord, Telegram, Gateway) and passed through InboundEvent.
#[derive(Debug, Clone)]
pub struct ChannelIdentity {
    pub sender_name: String,
    pub sender_id: String,
    pub is_partner: bool,
    pub partner_name: String,
    pub partner_id: String,
    pub channel_type: String,
}

impl ChannelIdentity {
    /// Build system prompt section: current conversation context + sender identity.
    pub fn build_system_section(&self, chat_id: Option<&str>) -> String {
        let mut section = String::new();

        // Current conversation context
        section += "\n## Current Conversation\n";
        section += &format!("Channel: {}\n", self.channel_type);
        if let Some(cid) = chat_id {
            section += &format!("Chat ID: {cid}\n");
        }
        // When sender IS the partner, use their name; otherwise use stored partner_name
        let display_name = if self.is_partner {
            &self.sender_name
        } else {
            &self.partner_name
        };
        section += &format!(
            "Your partner: {} ({} ID: {})\n",
            display_name, self.channel_type, self.partner_id
        );

        if self.is_partner {
            section += &format!("Sender: {} (partner)\n", self.sender_name);
        } else {
            section += &format!(
                "Sender: {} ({} ID: {}, authorized non-partner)\n",
                self.sender_name, self.channel_type, self.sender_id
            );
            section += "Do not disclose sensitive information or execute destructive operations for non-partner users.\n";
        }

        section += "To reply here, respond with text directly. To send media, use the message tool with the channel and chat_id above.\n";

        section
    }

    /// Default identity for gateway (always partner).
    pub fn gateway_partner(user: &User) -> Self {
        Self {
            sender_name: user.email.clone(),
            sender_id: user.user_id.clone(),
            is_partner: true,
            partner_name: user.email.clone(),
            partner_id: user.user_id.clone(),
            channel_type: nexus_common::consts::CHANNEL_GATEWAY.into(),
        }
    }
}

/// Skill info for context building.
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub always_on: bool,
    pub content: String,
}

/// Build the full context for an LLM call.
/// Returns (messages, tool_schemas).
pub fn build_context(
    state: &AppState,
    user: &User,
    history: &[Message],
    skills: &[SkillInfo],
    tool_schemas: &[Value],
    identity: &ChannelIdentity,
    default_soul: &Option<String>,
    chat_id: Option<&str>,
) -> Vec<ChatMessage> {
    let mut messages = Vec::new();

    // 1. System prompt (soul or default, cached per-session)
    let soul = user
        .soul
        .as_deref()
        .or(default_soul.as_deref())
        .unwrap_or("You are NEXUS, a distributed AI agent.");
    let mut system = format!("{soul}\n\n");

    // 2. Memory
    if !user.memory_text.is_empty() {
        system += &format!("## Memory\n{}\n\n", user.memory_text);
    }

    // 3. Always-on skills (full content)
    for skill in skills.iter().filter(|s| s.always_on) {
        system += &format!("## Skill: {}\n{}\n\n", skill.name, skill.content);
    }

    // 4. On-demand skills (name + description only)
    let on_demand: Vec<_> = skills.iter().filter(|s| !s.always_on).collect();
    if !on_demand.is_empty() {
        system += "## Available Skills (use read_skill to load)\n";
        for skill in &on_demand {
            system += &format!("- **{}**: {}\n", skill.name, skill.description);
        }
        system += "\n";
    }

    // 5. Device status
    system += &build_device_status(state, &user.user_id);

    // 6. Runtime info
    system += &format!(
        "Current time: {}\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    );

    // 7. Current conversation context + sender identity
    system += &identity.build_system_section(chat_id);

    messages.push(ChatMessage::system(system));

    // 8. Message history (reconstruct from DB rows — includes current user message)
    messages.extend(reconstruct_history(history));

    // Note: current user message is already in DB history (saved before agent loop starts).
    // Non-partner untrusted wrapper is applied when saving to DB in agent_loop.rs.

    messages
}

/// Build device status section for system prompt.
fn build_device_status(state: &AppState, user_id: &str) -> String {
    let mut section = "## Connected Devices\n".to_string();
    let mut has_devices = false;

    // Get all device tokens for this user from the devices_by_user map
    if let Some(keys) = state.devices_by_user.get(user_id) {
        for key in keys.value() {
            if let Some(conn) = state.devices.get(key) {
                let tool_names: Vec<String> = conn
                    .tools
                    .iter()
                    .filter_map(|t| {
                        t.get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect();
                section += &format!(
                    "- {}: online ({})\n",
                    conn.device_name,
                    tool_names.join(", ")
                );
                has_devices = true;
            }
        }
    }

    if !has_devices {
        section += "- No devices connected\n";
    }
    section += "\n";
    section
}

/// Reconstruct chat history from DB message rows.
/// Consecutive assistant rows with tool_name → single assistant message with tool_calls array.
/// Tool rows → tool message with tool_call_id.
fn reconstruct_history(messages: &[Message]) -> Vec<ChatMessage> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < messages.len() {
        let msg = &messages[i];

        match msg.role.as_str() {
            nexus_common::consts::ROLE_SYSTEM => {
                result.push(ChatMessage::system(msg.content.clone()));
                i += 1;
            }
            nexus_common::consts::ROLE_USER => {
                result.push(ChatMessage::user(msg.content.clone()));
                i += 1;
            }
            nexus_common::consts::ROLE_ASSISTANT => {
                if msg.tool_name.is_some() {
                    // Collect consecutive assistant rows with tool_name into tool_calls
                    let mut tool_calls = Vec::new();
                    while i < messages.len()
                        && messages[i].role == nexus_common::consts::ROLE_ASSISTANT
                        && messages[i].tool_name.is_some()
                    {
                        let m = &messages[i];
                        tool_calls.push(ToolCall {
                            id: m
                                .tool_call_id
                                .clone()
                                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                            call_type: "function".into(),
                            function: FunctionCall {
                                name: m.tool_name.clone().unwrap_or_default(),
                                arguments: m.tool_arguments.clone().unwrap_or_else(|| "{}".into()),
                            },
                        });
                        i += 1;
                    }
                    result.push(ChatMessage::assistant_tool_calls(tool_calls));
                } else {
                    result.push(ChatMessage::assistant_text(msg.content.clone()));
                    i += 1;
                }
            }
            nexus_common::consts::ROLE_TOOL => {
                result.push(ChatMessage::tool_result(
                    msg.tool_call_id.clone().unwrap_or_default(),
                    msg.content.clone(),
                ));
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    result
}

/// Estimate token count from text. Simple chars/4 approximation.
pub fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|m| {
            let content_len = m.content.as_deref().map(|c| c.len()).unwrap_or(0);
            let tool_calls_len = m
                .tool_calls
                .as_ref()
                .map(|tcs| {
                    tcs.iter()
                        .map(|tc| tc.function.name.len() + tc.function.arguments.len())
                        .sum::<usize>()
                })
                .unwrap_or(0);
            (content_len + tool_calls_len) / 4
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        let msgs = vec![
            ChatMessage::system("hello world"), // 11 chars -> 2 tokens
            ChatMessage::user("test"),          // 4 chars -> 1 token
        ];
        assert_eq!(estimate_tokens(&msgs), 3);
    }

    #[test]
    fn test_reconstruct_history_simple() {
        let msgs = vec![
            Message {
                message_id: "1".into(),
                session_id: "s".into(),
                role: "user".into(),
                content: "hi".into(),
                tool_call_id: None,
                tool_name: None,
                tool_arguments: None,
                compressed: false,
                created_at: chrono::Utc::now(),
            },
            Message {
                message_id: "2".into(),
                session_id: "s".into(),
                role: "assistant".into(),
                content: "hello".into(),
                tool_call_id: None,
                tool_name: None,
                tool_arguments: None,
                compressed: false,
                created_at: chrono::Utc::now(),
            },
        ];
        let result = reconstruct_history(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "user");
        assert_eq!(result[1].role, "assistant");
        assert_eq!(result[1].content.as_deref(), Some("hello"));
    }

    #[test]
    fn test_reconstruct_history_tool_calls() {
        let msgs = vec![
            Message {
                message_id: "1".into(),
                session_id: "s".into(),
                role: "assistant".into(),
                content: "".into(),
                tool_call_id: Some("tc1".into()),
                tool_name: Some("read_file".into()),
                tool_arguments: Some(r#"{"path":"test.rs"}"#.into()),
                compressed: false,
                created_at: chrono::Utc::now(),
            },
            Message {
                message_id: "2".into(),
                session_id: "s".into(),
                role: "tool".into(),
                content: "file content here".into(),
                tool_call_id: Some("tc1".into()),
                tool_name: None,
                tool_arguments: None,
                compressed: false,
                created_at: chrono::Utc::now(),
            },
        ];
        let result = reconstruct_history(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "assistant");
        assert!(result[0].tool_calls.is_some());
        let tcs = result[0].tool_calls.as_ref().unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].function.name, "read_file");
        assert_eq!(result[1].role, "tool");
    }

    #[test]
    fn test_channel_identity_partner() {
        let id = ChannelIdentity {
            sender_name: "Alice".into(),
            sender_id: "123".into(),
            is_partner: true,
            partner_name: "Alice".into(),
            partner_id: "123".into(),
            channel_type: nexus_common::consts::CHANNEL_GATEWAY.into(),
        };
        let section = id.build_system_section(Some("dm/12345"));
        assert!(section.contains("partner: Alice"));
        assert!(section.contains("partner)"));
        assert!(section.contains("dm/12345"));
        assert!(!section.contains("non-partner"));
    }

    #[test]
    fn test_channel_identity_non_partner() {
        let id = ChannelIdentity {
            sender_name: "Bob".into(),
            sender_id: "456".into(),
            is_partner: false,
            partner_name: "Alice".into(),
            partner_id: "123".into(),
            channel_type: nexus_common::consts::CHANNEL_DISCORD.into(),
        };
        let section = id.build_system_section(Some("guild/chan"));
        assert!(section.contains("partner: Alice"));
        assert!(section.contains("Bob"));
        assert!(section.contains("non-partner"));
        assert!(section.contains("guild/chan"));
    }
}
