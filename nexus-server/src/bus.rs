use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};

/// 用户消息事件（来自任意 Channel）
#[derive(Debug, Clone)]
pub struct InboundEvent {
    pub channel: String,      // "webui" | "discord" | "telegram" | ...
    pub sender_id: String,   // 发消息的用户 ID
    pub chat_id: String,     // 会话 ID
    pub content: String,      // 消息内容
    pub session_id: String,  // Nexus 内部 session 标识
}

/// Agent 响应事件（发往任意 Channel）
#[derive(Debug, Clone)]
pub struct OutboundEvent {
    pub channel: String,
    pub chat_id: String,
    pub content: String,
}

/// 简化的 MessageBus：用 Vec + RwLock 替代 async Queue
/// inbound: 待处理的 inbound 事件队列
/// outbound: 待发送的 outbound 事件队列
pub struct MessageBus {
    pub inbound: Arc<RwLock<Vec<InboundEvent>>>,
    pub outbound: Arc<RwLock<Vec<OutboundEvent>>>,
    shutdown_tx: broadcast::Sender<()>,
}

impl MessageBus {
    pub fn new() -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);
        Self {
            inbound: Arc::new(RwLock::new(Vec::new())),
            outbound: Arc::new(RwLock::new(Vec::new())),
            shutdown_tx,
        }
    }

    /// 发布一个 inbound 事件到队列尾部
    pub async fn publish_inbound(&self, event: InboundEvent) {
        self.inbound.write().await.push(event);
    }

    /// 消费一个 inbound 事件（从队列头部取）
    pub async fn consume_inbound(&self) -> InboundEvent {
        loop {
            let event = {
                let mut inbound = self.inbound.write().await;
                if !inbound.is_empty() {
                    Some(inbound.remove(0))
                } else {
                    None
                }
            };
            if let Some(e) = event {
                return e;
            }
            // 队列空，短暂休眠后重试
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// 发布一个 outbound 事件到队列尾部
    pub async fn publish_outbound(&self, event: OutboundEvent) {
        self.outbound.write().await.push(event);
    }

    /// 消费一个 outbound 事件（从队列头部取），支持 shutdown 信号
    pub async fn consume_outbound(&self) -> Option<OutboundEvent> {
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        loop {
            // 先检查是否有事件
            let event = {
                let mut outbound = self.outbound.write().await;
                if !outbound.is_empty() {
                    Some(outbound.remove(0))
                } else {
                    None
                }
            };
            if let Some(e) = event {
                return Some(e);
            }
            // 队列空，等待事件或 shutdown 信号
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
                _ = shutdown_rx.recv() => {
                    return None;
                }
            }
        }
    }

    /// 触发 shutdown，所有 consumer 的 consume_outbound 会返回 None
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}