/// 职责边界：
/// 负责对话记忆的整合（Consolidation）逻辑，是 db.rs 纯 CRUD 之上的业务层。
/// agent_loop.rs 在每轮 LLM 调用前调用 maybe_consolidate()，
/// 由本模块决定是否触发 consolidation、如何执行 consolidation、失败后如何降级。
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【Consolidation 触发时机】
/// ─────────────────────────────────────────────────────────────────────────────
/// maybe_consolidate() 在以下条件同时满足时触发 consolidation：
///   estimate_tokens(messages) > context_window_tokens * 0.5
/// 其中 context_window_tokens 从 ServerConfig 读取（对应 LLM 的上下文窗口大小）。
/// Token 估算采用粗估策略：字符数 / 3（与 nanobot estimate_session_prompt_tokens 一致）。
///
/// 参考 nanobot：nanobot/agent/memory.py  estimate_session_prompt_tokens() L276-291
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【Consolidation 流程（consolidate）】
/// ─────────────────────────────────────────────────────────────────────────────
/// 1. 从 db.rs 取出该 session 尚未经过 consolidation 的最旧一批消息
///    （游标 last_consolidated 之前、按 pick_consolidation_boundary 划定的边界以内）。
///    边界选取规则：从消息列表末尾往前找，在不超过 token 阈值的最远 user-turn 处截断，
///    避免在 assistant/tool 消息中间截断造成上下文破损。
///    参考 nanobot：nanobot/agent/memory.py  pick_consolidation_boundary() L254-274
///
/// 2. 将该批消息发给 LLM，要求其调用内置的 save_memory tool，返回：
///      - history_entry: 带时间戳的一行摘要（例如 "[2025-03-20 14:30] 用户修复了登录 bug"）
///      - memory_update: 更新后的完整记忆文本（合并进 MemoryChunks 表）
///    参考 nanobot：nanobot/agent/memory.py  consolidate() L114-196
///
/// 3. 将 memory_update 做 embedding（调用 context::embed_text()），
///    连同 history_entry 存入 db.rs 的 MemoryChunks 表（pgvector 字段）。
///
/// 4. 在 db.rs 中将被整合的消息标记为 is_consolidated = true，
///    并更新 session 的 last_consolidated 游标（指向被整合消息中最新一条的 id）。
///    此后 db::get_session_history() 只返回游标之后的消息，已 consolidated 的消息不再重复送给 LLM。
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【失败降级策略（3-strike fallback）】
/// ─────────────────────────────────────────────────────────────────────────────
/// 记录连续失败次数（per-session，存内存即可）：
///   - LLM 未调用 save_memory tool → 失败 +1
///   - tool arguments 不合法 / 必填字段缺失 → 失败 +1
///   - 连续失败 3 次后：不再尝试 LLM 摘要，直接将原始消息文本拼接成归档条目，
///     写入 MemoryChunks 并推进 last_consolidated 游标，然后重置失败计数。
/// 参考 nanobot：nanobot/agent/memory.py  L78, L159-175, L201-219
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【参考 nanobot】
/// ─────────────────────────────────────────────────────────────────────────────
/// nanobot/agent/memory.py
///   consolidate()                   L114-219  — consolidation 主流程
///   pick_consolidation_boundary()   L254-274  — 边界选取
///   estimate_session_prompt_tokens() L276-291 — Token 估算
///
/// nanobot 基于本地文件（MEMORY.md + HISTORY.md），Nexus 改为写入 PostgreSQL MemoryChunks 表。

// TODO: pub fn estimate_tokens(messages: &[ChatMessage]) -> usize
//   粗估：所有消息 content 字段的字符数之和除以 3。
//   ChatMessage 类型由 context.rs 或 db.rs 定义。

// TODO: pub async fn maybe_consolidate(
//           session_id: SessionId,
//           db: &PgPool,
//           llm: &dyn LlmProvider,
//           context_window_tokens: usize,
//       )
//   入口函数，由 agent_loop.rs 在每轮 LLM 调用前调用。
//   先调用 estimate_tokens，若未超阈值直接返回；超过则调用 consolidate()。

// TODO: pub async fn consolidate(
//           session_id: SessionId,
//           db: &PgPool,
//           llm: &dyn LlmProvider,
//       )
//   执行一次完整的 consolidation。包含：
//   - 取旧消息 → 调 LLM 摘要 → 写 MemoryChunks → 更新游标
//   - 失败计数管理与 3-strike 降级逻辑

// TODO: 定义模块级私有的失败计数结构（per-session HashMap 或 DashMap）
//   用于跨调用追踪连续失败次数，无需持久化到 DB。
