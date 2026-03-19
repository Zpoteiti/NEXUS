/// 职责边界：
/// 1. 负责所有与 PostgreSQL 的交互 (SQLx 操作)。
/// 2. 处理 User、Session、ChatMessage 和 MemoryChunks (pgvector) 的增删改查。
///
/// 参考 nanobot：
/// - 这个文件替代了 `nanobot/agent/session.py` (会话管理) 和 `nanobot/agent/memory.py` (长期记忆)。
/// - 我们不需要写复杂的文件读写，只需实现纯粹的 async db 查询函数。

// TODO: 实现 get_session_history(session_id)
// TODO: 实现 save_message(message)
// TODO: 实现 vector_search_memory(query_embedding)