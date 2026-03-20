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

// TODO: 实现 PATCH /api/user/soul
//   更新当前登录用户的 soul（Agent 人设/角色设定）。
//   调用 db::update_user_soul()。
//   对应 nanobot 用户直接编辑 SOUL.md 的行为，Nexus 改为 REST API 写入。

// TODO: 实现 PATCH /api/user/preferences
//   更新当前登录用户的 user_preferences（语气、回复风格、语言等）。
//   调用 db::update_user_preferences()。
//   调用方：Settings.vue 的"user_preferences 配置"保存按钮。