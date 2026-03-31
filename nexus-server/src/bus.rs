use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};
use dashmap::DashMap;
use chrono::{DateTime, Utc};
use tracing::warn;

/// 用户消息事件（来自任意 Channel）
#[derive(Debug, Clone)]
pub struct InboundEvent {
    pub channel: String,                                      // "webui" | "discord" | "telegram"
    pub sender_id: String,                                    // 用户 ID
    pub chat_id: String,                                      // 会话 ID
    pub content: String,                                       // 消息内容
    pub session_id: String,                                    // Nexus 内部 session 标识
    pub timestamp: Option<DateTime<Utc>>,                    // 消息时间戳
    pub media: Vec<String>,                                    // 媒体 URL 列表
    pub metadata: HashMap<String, serde_json::Value>,         // 运行时控制字段
}

/// Agent 响应事件（发往任意 Channel）
#[derive(Debug, Clone)]
pub struct OutboundEvent {
    pub channel: String,
    pub chat_id: String,
    pub content: String,
    pub media: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// MessageBus - 进程内异步消息总线
///
/// inbound: session 隔离路由。通过 `register_session` 注册后，
///          `publish_inbound` 根据 session_id 路由到对应 session 的 inbox。
/// outbound: 全局队列，ChannelManager 单消费者。
pub struct MessageBus {
    /// session_id → 该 session 的 inbox sender（用于路由）
    inbound_routes: Arc<DashMap<String, mpsc::Sender<InboundEvent>>>,
    /// outbound 队列（ChannelManager 单消费者）
    outbound_tx: Arc<mpsc::Sender<OutboundEvent>>,
    outbound_rx: Arc<Mutex<mpsc::Receiver<OutboundEvent>>>,
    shutdown_tx: broadcast::Sender<()>,
}

impl MessageBus {
    /// 创建新的 MessageBus
    pub fn new() -> Self {
        let (outbound_tx, outbound_rx) = mpsc::channel(256);
        let (shutdown_tx, _) = broadcast::channel(1);
        Self {
            inbound_routes: Arc::new(DashMap::new()),
            outbound_tx: Arc::new(outbound_tx),
            outbound_rx: Arc::new(Mutex::new(outbound_rx)),
            shutdown_tx,
        }
    }

    /// 注册一个 session 的 inbox sender 到路由表
    /// 由 session_manager 在创建新 session 时调用
    pub fn register_session(&self, session_id: String, tx: mpsc::Sender<InboundEvent>) {
        self.inbound_routes.insert(session_id, tx);
    }

    /// 从路由表移除一个 session
    /// 由 session 结束时调用
    pub fn unregister_session(&self, session_id: &str) {
        self.inbound_routes.remove(session_id);
    }

    /// 发布一个 inbound 事件——根据 session_id 路由到对应 session 的 inbox
    pub async fn publish_inbound(&self, event: InboundEvent) {
        let session_id = event.session_id.clone();
        if let Some(tx) = self.inbound_routes.get(&session_id) {
            if tx.send(event).await.is_err() {
                warn!("bus: session {} inbox closed, removing from routes", session_id);
                drop(tx);
                self.inbound_routes.remove(&session_id);
            }
        } else {
            warn!("bus: no route for session_id={}, message dropped", session_id);
        }
    }

    /// 发布一个 outbound 事件到队列
    pub async fn publish_outbound(&self, event: OutboundEvent) {
        // ChannelManager 消费端已关闭时，send 会失败，忽略即可
        let _ = self.outbound_tx.send(event).await;
    }

    /// 消费一个 outbound 事件（阻塞直到有事件或收到 shutdown）
    /// 返回 None 表示收到 shutdown 信号
    pub async fn consume_outbound(&self) -> Option<OutboundEvent> {
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let mut rx = self.outbound_rx.lock().await;

        tokio::select! {
            event = rx.recv() => event,
            _ = shutdown_rx.recv() => None,
        }
    }

    /// 触发 shutdown，所有 consume_outbound 调用返回 None
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}
