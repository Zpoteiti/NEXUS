use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use crate::bus::InboundEvent;
use crate::state::AppState;

/// 每个 session 的句柄
pub struct SessionHandle {
    pub lock: Arc<RwLock<()>>,    // 防止并发写数据库
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

    /// 获取或创建 session。
    /// - 新 session：返回 (true, Some((inbox_tx, inbox_rx)))
    ///   - tx 由调用方注册到 Bus
    ///   - rx 传给 spawn 的 agent_loop
    /// - 已有 session：返回 (false, None)
    pub async fn get_or_create_session(&self, session_id: &str) -> (bool, Option<(mpsc::Sender<InboundEvent>, mpsc::Receiver<InboundEvent>)>) {
        let mut sessions = self.sessions.write().await;

        if sessions.contains_key(session_id) {
            return (false, None);
        }

        // 创建新 session
        let (inbox_tx, inbox_rx) = mpsc::channel(64);
        let handle = SessionHandle {
            lock: Arc::new(RwLock::new(())),
        };
        sessions.insert(session_id.to_string(), handle);

        (true, Some((inbox_tx, inbox_rx)))
    }

    /// 获取 session 的锁（用于 DB 写操作）
    pub async fn get_session_lock(&self, session_id: &str) -> Option<Arc<RwLock<()>>> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).map(|h| h.lock.clone())
    }

    /// 移除 session
    pub async fn remove_session(&self, session_id: &str) {
        let mut sessions = self.sessions.write().await;
        sessions.remove(session_id);
    }

}

/// 确保 session 存在并发布 inbound 事件。
/// Channel 只需构造 InboundEvent，然后调用此函数——session 创建逻辑统一在这里。
pub async fn ensure_session_and_publish(
    state: &Arc<AppState>,
    event: InboundEvent,
) {
    let session_id = event.session_id.clone();
    let (is_new, channels) = state.session_manager.get_or_create_session(&session_id).await;
    if is_new {
        if let Some((inbox_tx, inbox_rx)) = channels {
            state.bus.register_session(session_id.clone(), inbox_tx);
            let state_clone = state.clone();
            let sid = session_id.clone();
            tokio::spawn(async move {
                crate::agent_loop::run_session(sid, inbox_rx, state_clone).await;
            });
        }
    }
    state.bus.publish_inbound(event).await;
}
