/// 职责边界：
/// 1. 负责在每次调用 LLM 前，拼接出完整的 Prompt（System Prompt + History + RAG Memory）。
///
/// 参考 nanobot：
/// - 【核心参考】nanobot/agent/context.py  ContextBuilder.build_system_prompt() L56-98
/// - nanobot 从本地 SOUL.md / USER.md 文件读取 soul 与 user_preferences，
///   Nexus 改为从 db.rs 的 users/preferences 表动态读取，其余结构一致。

use crate::state::AppState;
use crate::tools_registry::build_tools_schema;

/// Call an OpenAI-compatible embeddings endpoint and return the embedding vector.
/// On any failure, returns an empty Vec (never blocks the flow).
pub async fn embed_text(config: &crate::config::EmbeddingConfig, text: &str) -> Vec<f32> {
    use reqwest::Client;
    use std::sync::LazyLock;

    static CLIENT: LazyLock<Client> = LazyLock::new(|| {
        Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("embed client")
    });

    let url = format!("{}/embeddings", config.api_base);
    let mut body = serde_json::json!({
        "model": config.model,
        "input": text,
    });
    // Only include dimensions if non-zero (some models like Qwen don't support Matryoshka)
    if config.dimensions > 0 {
        body["dimensions"] = serde_json::json!(config.dimensions);
    }

    let response = match CLIENT
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .json(&body)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            tracing::warn!("embed_text request failed: {}", e);
            return Vec::new();
        }
    };

    if !response.status().is_success() {
        tracing::warn!("embed_text HTTP {}", response.status());
        return Vec::new();
    }

    let json: serde_json::Value = match response.json().await {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("embed_text parse error: {}", e);
            return Vec::new();
        }
    };

    json.get("data")
        .and_then(|d| d.get(0))
        .and_then(|d| d.get("embedding"))
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_f64().map(|f| f as f32))
                .collect()
        })
        .unwrap_or_default()
}

/// Semaphore-guarded wrapper around `embed_text` for concurrency control.
pub async fn embed_text_throttled(
    config: &crate::config::EmbeddingConfig,
    text: &str,
    semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
) -> Vec<f32> {
    let _permit = semaphore.acquire().await.expect("semaphore closed");
    embed_text(config, text).await
}

/// 系统提示词各段之间的分隔符（与 nanobot 保持一致）
const SECTION_SEPARATOR: &str = "\n\n---\n\n";

/// 历史消息窗口最大条数
const MAX_HISTORY_MESSAGES: usize = 500;

/// 构建完整的 System Prompt。
///
/// 各段按以下顺序拼接，段间以 SECTION_SEPARATOR 分隔：
///
/// 段 1 — 身份与运行时信息（必须段）
/// 段 2 — soul 与 user_preferences（按需段，DB 中有数据才注入）
/// 段 3 — 在线设备与可用工具（必须段）
/// 段 4 — 长期记忆 RAG 注入（按需段，有检索结果才注入）
/// 段 5 — 常驻 Skill 摘要（按需段）
pub async fn build_system_prompt(
    state: &AppState,
    user_id: &str,
    _session_id: &str,
    user_input: &str,
    metadata: &std::collections::HashMap<String, serde_json::Value>,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    // 段 1 — 身份与运行时信息
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
    sections.push(format!(
        "You are Nexus, a distributed AI agent assistant running on {}.",
        now
    ));

    // 段 2 — soul 与 user_preferences
    let user_soul = crate::db::get_user_soul(&state.db, user_id).await.ok().flatten();
    let soul = match user_soul {
        Some(s) => Some(s),
        None => crate::db::get_system_config(&state.db, "default_soul")
            .await
            .ok()
            .flatten(),
    };
    if let Some(soul_text) = soul {
        sections.push(format!("## Personality\n{}", soul_text));
    }

    let user_prefs = crate::db::get_user_preferences(&state.db, user_id)
        .await
        .ok()
        .flatten();
    if let Some(prefs) = user_prefs {
        sections.push(format!("## User Preferences\n{}", prefs));
    }

    // 段 2.5 — 消息发送者身份与安全边界（Discord 等外部渠道）
    if let Some(sender_section) = build_sender_identity_section(metadata) {
        sections.push(sender_section);
    }

    // 段 3 — 在线设备与可用工具（必须段）
    let device_section = build_device_section(state, user_id).await;
    sections.push(device_section);

    // 段 4 — RAG 注入
    let embedding_config = state.config.embedding.read().await.clone();
    if let Some(ref emb_config) = embedding_config {
        let query_emb = embed_text_throttled(emb_config, user_input, &state.embedding_semaphore).await;
        if !query_emb.is_empty() {
            let chunks = crate::db::vector_search_memory(&state.db, user_id, &query_emb, 5)
                .await
                .unwrap_or_default();
            if !chunks.is_empty() {
                let memory_text = chunks
                    .iter()
                    .map(|c| c.memory_text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                sections.push(format!("## Relevant Memory\n{}", memory_text));
            }
        }
    }

    // 段 5 — 常驻 Skill 内容注入（always=true 的 skill 全文注入）
    let always_skills = collect_always_skills(state, user_id).await;
    if !always_skills.is_empty() {
        let mut skill_section = String::from("## Active Skills\n");
        for (name, content) in &always_skills {
            skill_section.push_str(&format!("### {}\n{}\n\n", name, content));
        }
        sections.push(skill_section);
    }

    sections.join(SECTION_SEPARATOR)
}

/// Collect all always=true skills from the user's online devices.
/// Lock order: devices -> devices_by_user (same as build_device_section).
async fn collect_always_skills(state: &AppState, user_id: &str) -> Vec<(String, String)> {
    let devices = state.devices.read().await;
    let devices_by_user = state.devices_by_user.read().await;

    let user_device_keys: Vec<&String> = match devices_by_user.get(user_id) {
        Some(d) => d.values().collect(),
        None => return Vec::new(),
    };

    let mut skills = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for device_key in user_device_keys {
        if let Some(device_state) = devices.get(device_key) {
            for skill in &device_state.skills {
                if skill.always {
                    if let Some(ref content) = skill.content {
                        if seen.insert(skill.name.clone()) {
                            skills.push((skill.name.clone(), content.clone()));
                        }
                    }
                }
            }
        }
    }

    skills
}

/// 构建发送者身份与安全边界段。
///
/// 当消息来自 Discord 等外部渠道时，根据 is_owner 标记注入不同的安全策略：
/// - owner：完全信任
/// - 非 owner 的授权用户：限制敏感操作
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

/// 构建段 3：在线设备与可用工具列表。
///
/// 从 AppState.devices 中筛选出属于该 user_id 的在线设备，
/// 列出每台设备的 device_name、状态（online/busy）及其注册的工具。
async fn build_device_section(state: &AppState, user_id: &str) -> String {
    let devices = state.devices.read().await;
    let devices_by_user = state.devices_by_user.read().await;

    // 获取该用户的所有设备名称
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

/// 获取该用户所有设备的工具 Schema（Server 已注入 device_name enum）。
pub async fn get_all_tools_schema(
    state: &AppState,
    user_id: &str,
) -> Vec<serde_json::Value> {
    let devices = state.devices.read().await;
    let mut all_schemas: Vec<serde_json::Value> = Vec::new();

    for device_state in devices.values() {
        if device_state.user_id != user_id {
            continue;
        }
        if !device_state.tools.is_empty() {
            let decorated = build_tools_schema(state, user_id, device_state.tools.clone()).await;
            all_schemas.extend(decorated);
        }
    }

    // Built-in server-side tool: save_memory
    // Available in every session so the agent can proactively save important info
    all_schemas.push(serde_json::json!({
        "type": "function",
        "function": {
            "name": "save_memory",
            "description": "Save an important fact, user preference, or context to long-term memory. Use this when the user shares something worth remembering across sessions — preferences, project context, relationships, important decisions. Do NOT wait for the user to ask you to remember; proactively save anything that would be useful in future conversations.",
            "parameters": {
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The fact, preference, or context to remember. Be concise but complete."
                    }
                },
                "required": ["content"]
            }
        }
    }));

    // Built-in server-side tool: send_file
    // Allows the agent to send a file from a connected device to the user
    all_schemas.push(serde_json::json!({
        "type": "function",
        "function": {
            "name": "send_file",
            "description": "Send a file from a connected device to the user. Use this after creating or finding a file that the user should receive. The file will be uploaded and sent as an attachment in the chat.",
            "parameters": {
                "type": "object",
                "properties": {
                    "device_name": {
                        "type": "string",
                        "description": "The device where the file is located"
                    },
                    "file_path": {
                        "type": "string",
                        "description": "Absolute path to the file on the device"
                    }
                },
                "required": ["device_name", "file_path"]
            }
        }
    }));

    all_schemas
}

/// 构建历史消息窗口，供 LLM 上下文使用。
///
/// 从 db::get_session_history 拉取未经 consolidation 的最新消息窗口，
/// 截断至 MAX_HISTORY_MESSAGES 条，并修复孤儿 tool_result。
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

/// 截断历史消息到 MAX_HISTORY_MESSAGES 条，并修复孤儿 tool_result。
///
/// 孤儿 tool_result 修复（_find_legal_start）：
/// 若窗口起点处存在 tool_result 但对应的 tool_calls 已被截断移出，
/// 则自动前移起点，跳过孤立的 tool_result，直到起点为 role=user 的消息。
fn truncate_and_fix_orphans(
    messages: Vec<serde_json::Value>,
    max_messages: usize,
) -> Vec<serde_json::Value> {
    if messages.len() <= max_messages {
        return messages;
    }
    // 从末尾取 max_messages 条
    let window: Vec<_> = messages.into_iter().rev().take(max_messages).rev().collect();

    // 修复孤儿 tool_result：确保起点不为孤立 tool_result
    let start = find_legal_start(&window);
    window[start..].to_vec()
}

fn find_legal_start(messages: &[serde_json::Value]) -> usize {
    for i in 0..messages.len() {
        let role = messages[i].get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role == "user" {
            return i;
        }
    }
    0
}

