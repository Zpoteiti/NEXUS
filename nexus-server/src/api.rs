/// 职责边界：
/// 1. 专门为 Vue WebUI 提供标准的 HTTP REST API。
/// 2. 负责非对话类的 CRUD 操作。例如：拉取历史会话列表、重命名会话、拉取所有向量记忆文档、查询在线设备和可用工具等。
/// 3. 直接调用 `db.rs` 和 `state.rs`，【绝对不与消息总线 bus 交互】。
///
/// 参考 nanobot：
/// - 替代 `nanobot/session/manager.py` 中的 `list_sessions` 等文件查询方法，将其转化为 JSON API 接口。

// TODO: 实现 GET /api/sessions
// TODO: 实现 GET /api/sessions/:id/messages
// TODO: 实现 GET /api/devices
// TODO: 实现 GET /api/memories