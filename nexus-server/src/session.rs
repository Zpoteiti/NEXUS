use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

/// 每个 session 的句柄
pub struct SessionHandle {
    pub inbox_tx: mpsc::Sender<InboundEvent>,
    pub lock: Arc<RwLock<()>>,    // 防止并发写数据库
}

/// 用户消息事件（暂时放在这里，后面 Task 1 会移到 bus.rs）
#[derive(Debug, Clone)]
pub struct InboundEvent {
    pub channel: String,
    pub sender_id: String,
    pub chat_id: String,
    pub content: String,
    pub session_id: String,
}

pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<String, SessionHandle>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 获取或创建 session 的 inbox sender。
    /// - 新 session：创建 channel，返回 tx，调用方 spawn agent_loop 时使用返回的 rx
    /// - 已有 session：直接返回已存在的 tx
    pub async fn get_or_create_session(&self, session_id: &str) -> (mpsc::Sender<InboundEvent>, bool) {
        // 快速路径：已有则返回
        {
            let sessions = self.sessions.read().await;
            if let Some(handle) = sessions.get(session_id) {
                return (handle.inbox_tx.clone(), false);
            }
        }

        // 未找到，创建新的 session channel
        let (inbox_tx, _inbox_rx) = mpsc::channel(64);
        let handle = SessionHandle {
            inbox_tx: inbox_tx.clone(),
            lock: Arc::new(RwLock::new(())),
        };

        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id.to_string(), handle);

        (inbox_tx, true)  // true = 新创建，调用方应用返回的 _inbox_rx spawn agent_loop
    }

    /// 获取 session 的锁（用于 DB 写操作）
    pub async fn get_session_lock(&self, session_id: &str) -> Option<Arc<RwLock<()>>> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).map(|h| h.lock.clone())
    }

    /// 获取所有活跃 session ids
    pub async fn list_sessions(&self) -> Vec<String> {
        let sessions = self.sessions.read().await;
        sessions.keys().cloned().collect()
    }
}
