/// 职责边界：
/// 1. 负责在每次调用 LLM 前，拼接出完整的 Prompt（System Prompt + History + RAG Memory）。
///
/// 参考 nanobot：
/// - 【核心参考】nanobot/agent/context.py  ContextBuilder.build_system_prompt() L56-98
/// - nanobot 从本地 SOUL.md / USER.md 文件读取 soul 与 user_preferences，
///   Nexus 改为从 db.rs 的 users/preferences 表动态读取，其余结构一致。

use crate::state::AppState;
use crate::tools_registry::build_tools_schema;

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
    _user_input: &str,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    // 段 1 — 身份与运行时信息
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
    sections.push(format!(
        "You are Nexus, a distributed AI agent assistant running on {}.",
        now
    ));

    // 段 2 — soul 与 user_preferences（后续实现）

    // 段 3 — 在线设备与可用工具（必须段）
    let device_section = build_device_section(state, user_id).await;
    sections.push(device_section);

    // 段 4 — RAG 注入（后续实现）

    sections.join(SECTION_SEPARATOR)
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

