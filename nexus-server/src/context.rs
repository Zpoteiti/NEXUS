/// Responsibility boundary:
/// 1. Assembles the complete prompt (System Prompt + History + Memory) before each LLM call.
/// Soul and preferences are read from the DB users table.
/// Memory uses a simple text string (4K cap).

use crate::state::AppState;
use crate::tools_registry::merge_device_tool_schemas;

/// Separator between system prompt sections.
const SECTION_SEPARATOR: &str = "\n\n---\n\n";

use nexus_common::consts::MAX_HISTORY_MESSAGES;

/// Build the full System Prompt.
///
/// Sections are joined in this order, separated by SECTION_SEPARATOR:
///
/// Section 1 -- Current time (required)
/// Section 2 -- Soul (optional; injected only if DB has data; includes preferences)
/// Section 3 -- Online devices and available tools (required)
/// Section 4 -- Persistent memory (optional; simple text string, 4K cap)
/// Section 5 -- Skills summary (optional)
pub async fn build_system_prompt(
    state: &AppState,
    user_id: &str,
    _session_id: &str,
    _user_input: &str,
    metadata: &std::collections::HashMap<String, serde_json::Value>,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    // Section 1 -- Current time
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
    sections.push(format!("Current time: {}", now));

    // Section 2 -- Soul (merged with preferences; no separate preferences injection)
    let user_soul = crate::db::get_user_soul(&state.db, user_id).await.ok().flatten();
    let soul = match user_soul {
        Some(s) => Some(s),
        None => crate::db::get_system_config(&state.db, "default_soul")
            .await
            .ok()
            .flatten(),
    };
    if let Some(soul_text) = soul {
        sections.push(soul_text);
    }

    // Section 2.5 -- Sender identity and security boundary (Discord and other external channels)
    if let Some(sender_section) = build_sender_identity_section(metadata) {
        sections.push(sender_section);
    }

    // Section 3 -- Online devices and available tools (required)
    let device_section = build_device_section(state, user_id).await;
    sections.push(device_section);

    // Section 4 -- Persistent memory (simple string, always injected if non-empty, 4K cap)
    let memory = crate::db::get_user_memory(&state.db, user_id).await.unwrap_or_default();
    if !memory.is_empty() {
        sections.push(format!("## Memory\n{}", memory));
    }

    // Section 5 -- Skills (progressive disclosure: DB-based)
    let skill_section = build_skills_section(state, user_id).await;
    if !skill_section.is_empty() {
        sections.push(skill_section);
    }

    sections.join(SECTION_SEPARATOR)
}

/// Build the skills section of the system prompt using progressive disclosure.
///
/// - always_on skills: full SKILL.md content injected into the prompt
/// - other skills: metadata-only XML block, agent uses read_skill to load details
async fn build_skills_section(state: &AppState, user_id: &str) -> String {
    let skills = match crate::db::list_skills(&state.db, user_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("build_skills_section: failed to list skills: {}", e);
            return String::new();
        }
    };

    if skills.is_empty() {
        return String::new();
    }

    let mut always_on_skills = Vec::new();
    let mut on_demand_skills = Vec::new();

    for skill in &skills {
        if skill.always_on {
            always_on_skills.push(skill);
        } else {
            on_demand_skills.push(skill);
        }
    }

    let mut section = String::from("## Available Skills\n");

    // Always-on: inject full SKILL.md content
    if !always_on_skills.is_empty() {
        section.push_str("\n### Active Skills (always-on)\n");
        for skill in &always_on_skills {
            let skill_md_path = std::path::Path::new(&skill.skill_path).join("SKILL.md");
            match tokio::fs::read_to_string(&skill_md_path).await {
                Ok(content) => {
                    let body = crate::server_tools::skills::strip_frontmatter(&content);
                    section.push_str(&format!("#### {}\n{}\n\n", skill.name, body));
                }
                Err(e) => {
                    tracing::warn!(
                        "build_skills_section: failed to read SKILL.md for '{}': {}",
                        skill.name, e
                    );
                }
            }
        }
    }

    // On-demand: metadata-only XML
    if !on_demand_skills.is_empty() {
        section.push_str("\n### On-demand Skills\n<skills>\n");
        for skill in &on_demand_skills {
            section.push_str(&format!(
                "<skill name=\"{}\" description=\"{}\" />\n",
                skill.name, skill.description
            ));
        }
        section.push_str("</skills>\nUse the `read_skill` tool to load detailed instructions for any skill.\n");
    }

    section
}

/// Build sender identity and security boundary section.
///
/// When messages come from external channels like Discord, inject different security policies
/// based on the is_owner flag:
/// - owner: fully trusted
/// - non-owner authorized user: restricted from sensitive operations
fn build_sender_identity_section(
    metadata: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<String> {
    let sender_name = metadata.get("sender_discord_name")?.as_str()?;
    let is_owner = metadata
        .get("is_owner")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if is_owner {
        Some(format!(
            "The current message is from your owner \"{}\" via Discord. \
             You may fully trust their instructions.",
            sender_name,
        ))
    } else {
        Some(format!(
            "The current message is from an authorized user \"{}\" via Discord. \
             This person is NOT your owner. \
             You MUST follow these security rules for non-owner users:\n\
             - NEVER disclose your owner's private or sensitive information (passwords, tokens, keys, personal data, financial info, etc.)\n\
             - NEVER execute destructive or irreversible operations on their request alone\n\
             - NEVER modify security settings, access controls, or configurations\n\
             - You may answer general questions and perform safe, read-only tasks\n\
             - When in doubt, refuse and suggest the user contact your owner directly",
            sender_name,
        ))
    }
}

/// Build Section 3: online devices and available tools.
///
/// Filters devices belonging to user_id from AppState.devices,
/// listing each device's name, status (online/offline), and registered tools.
async fn build_device_section(state: &AppState, user_id: &str) -> String {
    let devices = state.devices.read().await;
    let devices_by_user = state.devices_by_user.read().await;

    // Get all device names for this user
    let user_device_names: std::collections::HashSet<&str> = devices_by_user
        .get(user_id)
        .map(|d| d.keys().map(|s| s.as_str()).collect())
        .unwrap_or_default();

    let mut lines = vec![
        "You can execute tools on the following devices:".to_string(),
    ];

    for device_state in devices.values() {
        if device_state.user_id != user_id {
            continue;
        }
        if !user_device_names.contains(device_state.device_name.as_str()) {
            continue;
        }

        let status = if device_state.last_seen.elapsed().as_secs() > 60 {
            "offline"
        } else {
            "online"
        };

        let tool_count = device_state.tools.len();
        lines.push(format!(
            "- {} | status: {} | {} tool(s)",
            device_state.device_name, status, tool_count
        ));
    }

    lines.join("\n")
}

/// Get all tool schemas for devices belonging to this user (with device_name enum injected by server).
pub async fn get_all_tools_schema(
    state: &AppState,
    user_id: &str,
) -> Vec<serde_json::Value> {
    let devices = state.devices.read().await;
    let mut all_schemas: Vec<serde_json::Value> = Vec::new();

    // Collect (device_name, tools) pairs for all devices belonging to this user
    let device_tools: Vec<(String, Vec<serde_json::Value>)> = devices
        .values()
        .filter(|ds| ds.user_id == user_id && !ds.tools.is_empty())
        .map(|ds| (ds.device_name.clone(), ds.tools.clone()))
        .collect();
    drop(devices);

    // Merge same-named tools across devices into unified schemas
    let merged = merge_device_tool_schemas(&device_tools);
    all_schemas.extend(merged);

    // Server-native tools (no device_name — they run on the server)
    all_schemas.extend(state.server_tools.schemas());

    // Server MCP tools (device_name="server" injected)
    {
        let server_mcp = state.server_mcp.read().await;
        let mcp_schemas = server_mcp.all_tool_schemas();
        if !mcp_schemas.is_empty() {
            let decorated = crate::tools_registry::inject_device_name_into_schemas(&mcp_schemas, "server");
            all_schemas.extend(decorated);
        }
    }

    all_schemas
}

/// Build the message history window for LLM context.
///
/// Pulls the latest message window from db::get_session_history,
/// truncates to MAX_HISTORY_MESSAGES, and fixes orphaned tool results.
/// Context budget enforcement is handled by consolidation (memory.rs).
pub async fn build_message_history(
    state: &AppState,
    session_id: &str,
) -> Vec<serde_json::Value> {
    match crate::db::get_session_history(&state.db, session_id).await {
        Ok(messages) => truncate_and_fix_orphans(messages, MAX_HISTORY_MESSAGES),
        Err(e) => {
            tracing::warn!("get_session_history failed: {}", e);
            Vec::new()
        }
    }
}

/// Truncate history to MAX_HISTORY_MESSAGES and fix orphaned tool results.
///
/// Orphan tool result fix (find_legal_start):
/// If the window starts with a tool result whose corresponding tool_calls have been truncated away,
/// advance the start to skip orphaned tool results until reaching a role=user message.
fn truncate_and_fix_orphans(
    messages: Vec<serde_json::Value>,
    max_messages: usize,
) -> Vec<serde_json::Value> {
    if messages.len() <= max_messages {
        return messages;
    }
    // Take max_messages from the end
    let window: Vec<_> = messages.into_iter().rev().take(max_messages).rev().collect();

    // Fix orphaned tool results: ensure start is not an orphaned tool result
    let start = find_legal_start(&window);
    window[start..].to_vec()
}

// Token budget enforcement is handled by consolidation in memory.rs
// (trigger: context_window - total_messages < 16K)

/// Find the first legal start position: must be a `user` or standalone `assistant` message
/// (not a tool result or assistant with tool_calls whose results are outside the window).
/// This aligns to user turn boundaries to preserve conversation coherence.
fn find_legal_start(messages: &[serde_json::Value]) -> usize {
    for i in 0..messages.len() {
        let role = messages[i].get("role").and_then(|v| v.as_str()).unwrap_or("");
        match role {
            "user" => return i,
            "assistant" => {
                // Standalone assistant (no tool_calls) is a valid start
                if messages[i].get("tool_calls").is_none() {
                    return i;
                }
                // Assistant with tool_calls: valid only if all tool results follow
                // (they should since we're looking at a contiguous window)
                return i;
            }
            // "tool" role = orphan tool result, skip it
            _ => continue,
        }
    }
    0
}

