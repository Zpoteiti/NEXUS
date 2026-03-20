/// 职责边界：
/// 1. 负责在每次调用 LLM 前，拼接出完整的 Prompt（System Prompt + History + RAG Memory）。
///
/// 参考 nanobot：
/// - 【核心参考】nanobot/agent/context.py  ContextBuilder.build_system_prompt() L56-98
/// - nanobot 从本地 SOUL.md / USER.md 文件读取 soul 与 user_preferences，
///   Nexus 改为从 db.rs 的 users/preferences 表动态读取，其余结构一致。
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【System Prompt 分段结构】
/// ─────────────────────────────────────────────────────────────────────────────
/// 各段按以下顺序拼接，段间以 "\n\n---\n\n" 分隔（与 nanobot 保持一致）：
///
/// 段 1 — 身份与运行时信息（必须段）
///   包含：Agent 名称（Nexus）、当前 UTC 时间、用户 ID、所在 session ID。
///   对应 nanobot：context.py build_system_prompt() 中的 boilerplate 段（L56-98）。
///
/// 段 2 — soul 与 user_preferences（按需段，DB 中有数据才注入）
///   来源：db::get_user_soul(user_id)（对应 nanobot 的 SOUL.md）
///         db::get_user_preferences(user_id)（对应 nanobot 的 USER.md：语言、语气、回复长度等）
///   对应 nanobot：context.py 中 bootstrap 文件加载段（L108-118）。
///
/// 段 3 — 在线设备与可用工具（必须段）
///   来源：AppState 在线设备路由表，取该 user_id 下所有 DeviceState.tools。
///   格式建议：列出每台在线设备的 device_name 及其工具列表（tool name + description），
///   告知 LLM 当前可以调度哪些设备执行哪些工具。
///   Nexus 独有段，nanobot 无对应（nanobot 是单机，工具本地可见）。
///
/// 段 4 — 长期记忆 RAG 注入（按需段，有检索结果才注入）
///   流程：先调用 embed_text(user_input) 生成查询向量，
///         再调用 db::vector_search_memory(user_id, query_embedding, top_k)
///         检索 MemoryChunks 表中相关记忆片段，按相似度排序后注入。
///   对应 nanobot：context.py memory.get_memory_context() 段（L35-37）。
///   注意：nanobot 注入整个 MEMORY.md 全文；Nexus 改为向量检索 top-k 片段，更精准。
///
/// 段 5 — 常驻 Skill 摘要（按需段）
///   来源：从设备上报的 Skill 列表中筛选 always=true 的 Skill，
///         加载对应 SKILL.md 的完整正文注入（非 frontmatter，只注入 Markdown 主体）。
///   对应 nanobot：context.py get_always_skills() + load_skills_for_context()（L39-43）。

// TODO: pub async fn build_system_prompt(
//           session_id: SessionId,
//           user_id: UserId,
//           user_input: &str,
//           online_devices: &HashMap<DeviceId, DeviceState>,
//           db: &PgPool,
//       ) -> String
//   按上述五段顺序拼接，各段以 "\n\n---\n\n" 分隔。
//   段 2/4/5 若无内容则跳过（不插入空段和分隔符）。

// TODO: pub async fn build_message_history(
//           session_id: SessionId,
//           db: &PgPool,
//       ) -> Vec<Message>
//   从 db::get_session_history(session_id) 拉取历史消息（已应用 last_consolidated 游标）。
//   截断规则（参考 nanobot/session/manager.py  get_history() L69-93）：
//     - 最多取末尾 MAX_HISTORY_MESSAGES 条（consts::MAX_HISTORY_MESSAGES = 500）。
//     - 从第一条 role=user 的消息开始，避免历史以 tool_result 开头
//       （孤儿 tool_result 会导致部分 LLM provider 报错）。

// TODO: pub fn embed_text(text: &str, api_key: &str, api_base: &str) -> Vec<f32>
//   调用 LLM Embedding API（例如 OpenAI /v1/embeddings 端点）生成查询向量。
//   返回值直接传入 db::vector_search_memory()。
//   若 embedding 调用失败，返回空 Vec（跳过 RAG 注入，不中断整个上下文构建）。