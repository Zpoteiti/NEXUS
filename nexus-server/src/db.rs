/// 职责边界：
/// 1. 负责所有与 PostgreSQL 的交互 (SQLx 操作)。
/// 2. 处理 users、sessions、messages、memory_chunks 四张表的增删改查。
/// 3. 所有函数均为纯粹的 async CRUD，不包含任何业务逻辑。
///    业务逻辑（如 consolidation 触发判断、JWT 签发）由上层模块（memory.rs、auth.rs）负责。
///
/// 参考 nanobot：
/// - 这个文件替代了 `nanobot/agent/session.py`（会话管理）和 `nanobot/agent/memory.py`（长期记忆）。
/// - nanobot 基于本地文件（JSONL session 文件、MEMORY.md、HISTORY.md），Nexus 改为 PostgreSQL。

// ─────────────────────────────────────────────────────────────────────────────
// 【users 表】
// 存储注册用户的基本信息、密码哈希、权限标记，以及 soul（角色设定）和 user_preferences（回复偏好）。
// ─────────────────────────────────────────────────────────────────────────────

// TODO: pub async fn create_user(
//           db: &PgPool,
//           email: &str,
//           password_hash: &str,
//           is_admin: bool,
//       ) -> Result<UserId>
//   在 users 表中插入新用户记录，返回自动生成的 user_id（UUID）。
//   调用方：auth.rs register()

// TODO: pub async fn get_user_by_email(
//           db: &PgPool,
//           email: &str,
//       ) -> Result<Option<User>>
//   按 email 查询 users 表，返回完整 User 记录（含 password_hash、is_admin 等字段）。
//   找不到时返回 Ok(None)，不报错。
//   调用方：auth.rs login()

// TODO: pub async fn get_user_soul(
//           db: &PgPool,
//           user_id: UserId,
//       ) -> Result<Option<String>>
//   从 users 表读取 soul 字段（Markdown 文本，对应 nanobot 的 SOUL.md）。
//   Soul 为空时返回 Ok(None)，context.rs 跳过段 2 的该部分注入。
//   调用方：context.rs build_system_prompt()

// TODO: pub async fn get_user_preferences(
//           db: &PgPool,
//           user_id: UserId,
//       ) -> Result<Option<UserPreferences>>
//   从 users 表读取 user_preferences 字段（JSON，对应 nanobot 的 USER.md：语言、语气、回复长度等）。
//   偏好为空时返回 Ok(None)，context.rs 跳过段 2 的该部分注入。
//   调用方：context.rs build_system_prompt()

// TODO: pub async fn update_user_preferences(
//           db: &PgPool,
//           user_id: UserId,
//           prefs: &UserPreferences,
//       ) -> Result<()>
//   将 user_preferences 序列化为 JSON 后更新 users 表对应行。
//   调用方：api.rs（Settings 页"user_preferences 配置"保存接口）

// ─────────────────────────────────────────────────────────────────────────────
// 【sessions 表】
// 存储对话会话的元数据，包含所属用户、创建时间、last_consolidated 游标。
// last_consolidated 游标指向最后一条已经过 consolidation 的 message_id，
// get_session_history() 只返回该游标之后的消息。
// 参考 nanobot：nanobot/agent/session.py  Session.last_consolidated（L16-99）
// ─────────────────────────────────────────────────────────────────────────────

// TODO: pub async fn create_session(
//           db: &PgPool,
//           user_id: UserId,
//       ) -> Result<SessionId>
//   在 sessions 表中插入新会话记录，last_consolidated 游标初始为 None。
//   返回自动生成的 session_id（UUID）。
//   调用方：agent_loop.rs run_agent_loop()（用户首次发消息时创建 session）

// TODO: pub async fn get_session_history(
//           db: &PgPool,
//           session_id: SessionId,
//           last_consolidated_cursor: Option<MessageId>,
//       ) -> Result<Vec<ChatMessage>>
//   从 messages 表取出属于该 session、message_id 大于 last_consolidated_cursor 的所有消息，
//   按 created_at 升序排列（即未经 consolidation 的最新消息窗口）。
//   cursor 为 None 时返回全部消息（新 session）。
//   context.rs 在拿到结果后还会进行 MAX_HISTORY_MESSAGES 截断和孤儿 tool_result 修剪。
//   调用方：context.rs build_message_history()、memory.rs consolidate()

// TODO: pub async fn list_sessions(
//           db: &PgPool,
//           user_id: UserId,
//       ) -> Result<Vec<SessionMeta>>
//   返回该用户的所有 session 列表（session_id、created_at、消息条数等摘要字段），
//   按 created_at 倒序，供左侧历史 session 抽屉面板展示。
//   调用方：api.rs GET /api/sessions

// ─────────────────────────────────────────────────────────────────────────────
// 【messages 表】
// 存储每条对话消息，包含 role（user/assistant/tool）、content、tool_call_id 等字段，
// 以及 is_consolidated 标记（被 memory.rs consolidation 处理过的消息标记为 true）。
// 参考 nanobot：nanobot/agent/session.py  Session.messages[]（append-only，L16-99）
// ─────────────────────────────────────────────────────────────────────────────

// TODO: pub async fn save_message(
//           db: &PgPool,
//           session_id: SessionId,
//           role: &str,
//           content: &str,
//           tool_call_id: Option<&str>,
//       ) -> Result<MessageId>
//   向 messages 表追加一条新消息，is_consolidated 默认为 false。
//   返回自动生成的 message_id，供 memory.rs 更新 last_consolidated 游标使用。
//   调用方：agent_loop.rs（每轮 LLM 交互后追加 user/assistant/tool 消息）

// TODO: pub async fn mark_messages_consolidated(
//           db: &PgPool,
//           session_id: SessionId,
//           up_to_message_id: MessageId,
//       ) -> Result<()>
//   将 messages 表中属于该 session、message_id <= up_to_message_id 的行
//   批量更新 is_consolidated = true。
//   调用方：memory.rs consolidate()（完成 consolidation 后标记已处理消息）

// TODO: pub async fn update_last_consolidated_cursor(
//           db: &PgPool,
//           session_id: SessionId,
//           message_id: MessageId,
//       ) -> Result<()>
//   将 sessions 表中该 session 的 last_consolidated 字段更新为 message_id。
//   与 mark_messages_consolidated 配合使用：先标记消息，再推进游标。
//   调用方：memory.rs consolidate()

// ─────────────────────────────────────────────────────────────────────────────
// 【memory_chunks 表（pgvector）】
// 存储经 consolidation 生成的长期记忆片段，每条记录含向量 embedding 和原始文本。
// 对应 nanobot 的 MEMORY.md（全量记忆文本）+ HISTORY.md（带时间戳的摘要日志），
// Nexus 将两者合并为结构化的 memory_chunks 表，以支持向量相似度检索。
// 参考 nanobot：nanobot/agent/memory.py  MemoryStore（L75-219）
// ─────────────────────────────────────────────────────────────────────────────

// TODO: pub async fn save_memory_chunk(
//           db: &PgPool,
//           user_id: UserId,
//           session_id: SessionId,
//           embedding: Vec<f32>,
//           text: &str,
//           history_entry: &str,
//       ) -> Result<ChunkId>
//   向 memory_chunks 表插入一条新记忆片段：
//     - embedding：由 context::embed_text() 生成的向量（pgvector 类型）
//     - text：memory_update 全文（供精确检索）
//     - history_entry：带时间戳的单行摘要（如 "[2025-03-20 14:30] 修复了登录 bug"）
//   调用方：memory.rs consolidate()（步骤 3）

// TODO: pub async fn vector_search_memory(
//           db: &PgPool,
//           user_id: UserId,
//           query_embedding: Vec<f32>,
//           top_k: usize,
//       ) -> Result<Vec<MemoryChunk>>
//   对 memory_chunks 表按 embedding <-> query_embedding 余弦距离升序排列，
//   返回属于该 user_id 的前 top_k 条记录（含 text 和 history_entry 字段）。
//   使用 pgvector 的 <-> 操作符，需在 embedding 列建立 ivfflat 或 hnsw 索引。
//   调用方：context.rs build_system_prompt()（段 4 RAG 注入，经 embed_text() 生成查询向量后调用）

// TODO: pub async fn list_memory_chunks(
//           db: &PgPool,
//           user_id: UserId,
//       ) -> Result<Vec<MemoryChunk>>
//   按 created_at 倒序返回该用户的所有记忆片段列表（不含 embedding 向量字段，仅元数据+文本）。
//   供 Settings 页"Agent 记忆管理"面板展示使用。
//   调用方：api.rs GET /api/memories

// TODO: pub async fn delete_memory_chunk(
//           db: &PgPool,
//           chunk_id: ChunkId,
//           user_id: UserId,
//       ) -> Result<()>
//   从 memory_chunks 表中删除指定记忆片段。
//   user_id 作为二次校验，防止用户越权删除他人记忆。
//   调用方：api.rs（Settings 页记忆删除接口）