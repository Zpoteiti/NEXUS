/// 职责边界：
/// 1. 定义和管理全局共享状态 `AppState`，维护已连接的设备路由表。
///
/// 参考 nanobot：
/// - 在 nanobot 中，SessionManager 和 MemoryStore 是基于本地文件的 (`nanobot/agent/memory.py`)。
/// - 在这里，AppState 负责维护实时的网络拓扑，而持久化记忆将直接通过 SQLx 写入 PostgreSQL。

// TODO: 定义 AppState type alias (Arc<RwLock<HashMap<...>>>)