/// Responsibility boundary:
/// 1. Assembles the complete prompt (System Prompt + History + Memory) before each LLM call.
/// Soul is read from the DB users table.
/// Memory uses a simple text string (4K cap).

use crate::state::AppState;
use crate::tools_registry::merge_device_tool_schemas;

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

struct CachedSkill {
    content: String,
    mtime: std::time::SystemTime,
}

static SKILL_CONTENT_CACHE: LazyLock<RwLock<HashMap<String, CachedSkill>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Cache for default_soul to avoid repeated DB queries.
static DEFAULT_SOUL_CACHE: LazyLock<RwLock<Option<(String, Instant)>>> =
    LazyLock::new(|| RwLock::new(None));
const DEFAULT_SOUL_TTL: Duration = Duration::from_secs(300); // 5 min TTL

async fn get_default_soul_cached(db: &sqlx::PgPool) -> Option<String> {
    // Check cache
    {
        let cache = DEFAULT_SOUL_CACHE.read().await;
        if let Some((ref soul, ts)) = *cache {
            if ts.elapsed() < DEFAULT_SOUL_TTL {
                return Some(soul.clone());
            }
        }
    }
    // Cache miss or expired
    let soul = crate::db::get_system_config(db, "default_soul").await.ok().flatten();
    let mut cache = DEFAULT_SOUL_CACHE.write().await;
    *cache = soul.as_ref().map(|s| (s.clone(), Instant::now()));
    soul
}

/// Read a skill file with mtime-based caching.
async fn read_skill_cached(path: &std::path::Path) -> Option<String> {
    let key = path.to_string_lossy().to_string();

    // Read content first
    let content = tokio::fs::read_to_string(path).await.ok()?;
    // Then get mtime — if file was modified during read, we get the newer mtime,
    // which means we'll re-read on next access (safe direction)
    let mtime = tokio::fs::metadata(path).await.ok()?.modified().ok()?;

    // Check if we already have this exact version cached
    {
        let cache = SKILL_CONTENT_CACHE.read().await;
        if let Some(cached) = cache.get(&key) {
            if cached.mtime == mtime {
                return Some(cached.content.clone());
            }
        }
    }

    // Update cache
    let mut cache = SKILL_CONTENT_CACHE.write().await;
    cache.insert(key, CachedSkill { content: content.clone(), mtime });
    Some(content)
}

/// Separator between system prompt sections.
const SECTION_SEPARATOR: &str = "\n\n---\n\n";


/// Build the full System Prompt.
///
/// Sections are joined in this order, separated by SECTION_SEPARATOR:
///
/// Section 1 -- Current time (required)
/// Section 2 -- Soul (optional; injected only if DB has data)
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

    // Fire all independent async work concurrently.
    // We speculatively fetch default_soul alongside user_soul so that
    // a cache-miss on user_soul doesn't add another round-trip.
    let (user_soul, default_soul, device_section, memory, skill_section) = tokio::join!(
        crate::db::get_user_soul(&state.db, user_id),
        get_default_soul_cached(&state.db),
        build_device_section(state, user_id),
        crate::db::get_user_memory(&state.db, user_id),
        build_skills_section(state, user_id),
    );

    // Section 2 -- Soul (prefer user-specific, fall back to default)
    let soul = user_soul.ok().flatten().or(default_soul);
    if let Some(soul_text) = soul {
        sections.push(soul_text);
    }

    // Section 2.5 -- Sender identity and security boundary (Discord and other external channels)
    if let Some(sender_section) = build_sender_identity_section(metadata) {
        sections.push(sender_section);
    }

    // Section 3 -- Online devices and available tools (required)
    sections.push(device_section);

    // Section 4 -- Persistent memory (simple string, always injected if non-empty, 4K cap)
    let memory = memory.unwrap_or_default();
    if !memory.is_empty() {
        sections.push(format!("## Memory\n{}", memory));
    }

    // Section 5 -- Skills (progressive disclosure: DB-based)
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
            match read_skill_cached(&skill_md_path).await {
                Some(content) => {
                    let body = crate::server_tools::skills::strip_frontmatter(&content);
                    section.push_str(&format!("#### {}\n{}\n\n", skill.name, body));
                }
                None => {
                    tracing::warn!(
                        "build_skills_section: failed to read SKILL.md for '{}'",
                        skill.name,
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
    let mut lines = vec![
        "You can execute tools on the following devices:".to_string(),
    ];

    // O(user's devices) via devices_by_user index instead of iterating all devices
    if let Some(user_devices) = state.devices_by_user.get(user_id) {
        for (device_name, device_key) in user_devices.iter() {
            if let Some(device_state) = state.devices.get(device_key) {
                let status = if device_state.last_seen.elapsed().as_secs() > 60 {
                    "offline"
                } else {
                    "online"
                };
                let tool_count = device_state.tools.len();
                lines.push(format!(
                    "- {} | status: {} | {} tool(s)",
                    device_name, status, tool_count
                ));
            }
        }
    }

    lines.join("\n")
}

/// Get all tool schemas for devices belonging to this user (with device_name enum injected by server).
/// Results are cached per-user and invalidated when the global tool schema generation changes
/// (i.e., when devices register/unregister tools or connect/disconnect).
pub async fn get_all_tools_schema(
    state: &AppState,
    user_id: &str,
) -> Vec<serde_json::Value> {
    let current_gen = state.tool_schema_generation.load(Ordering::Acquire);

    // Check cache: if generation matches, return cached schemas
    if let Some(entry) = state.tool_schema_cache.get(user_id) {
        let (cached_gen, ref cached_schemas) = *entry;
        if cached_gen == current_gen {
            return cached_schemas.clone();
        }
    }

    // Cache miss — rebuild from scratch
    let mut all_schemas: Vec<serde_json::Value> = Vec::new();

    // Collect (device_name, tools) pairs for all devices belonging to this user
    let device_tools: Vec<(String, Vec<serde_json::Value>)> = state.devices
        .iter()
        .filter(|entry| entry.value().user_id == user_id && !entry.value().tools.is_empty())
        .map(|entry| (entry.value().device_name.clone(), entry.value().tools.clone()))
        .collect();

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
            let decorated = crate::tools_registry::inject_device_name_into_schemas(&mcp_schemas, nexus_common::consts::SERVER_DEVICE_NAME);
            all_schemas.extend(decorated);
        }
    }

    // Store in cache with current generation
    state.tool_schema_cache.insert(user_id.to_string(), (current_gen, all_schemas.clone()));

    all_schemas
}

/// Build the message history for LLM context.
///
/// Pulls all non-compressed messages from db::get_session_history.
/// Context budget enforcement is handled by consolidation (memory.rs),
/// which compresses old messages when remaining tokens drop below 16K.
pub async fn build_message_history(
    state: &AppState,
    session_id: &str,
) -> Vec<serde_json::Value> {
    match crate::db::get_session_history(&state.db, session_id).await {
        Ok(messages) => messages,
        Err(e) => {
            tracing::warn!("get_session_history failed: {}", e);
            Vec::new()
        }
    }
}

