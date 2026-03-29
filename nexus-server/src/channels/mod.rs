/// 职责边界：
/// 1. 定义异步的 `Channel` Trait，规范各平台 Channel 的统一接口（start、stop、send 等）。
/// 2. 实现 `ChannelManager`，负责系统启动时加载各启用渠道并管理其生命周期。
///    与 bus.rs 的边界约定：
///    - ChannelManager 在启动时持有 bus 的克隆引用。
///    - ChannelManager 运行一个后台 Task 消费 outbound 队列，
///      根据 OutboundEvent.channel 路由给对应的 Channel 实例。
///    - 各具体 Channel（telegram / webui）在注册到 ChannelManager 时，
///      用于将平台收到的消息推入总线（inbound 方向）。
/// 3. ChannelManager 不直接处理消息内容，只负责分发路由；
///    消息格式转换由各 Channel 实例（telegram.rs / webui.rs）内部完成。
///
/// 参考 nanobot：
/// - 替代 nanobot/channels/base.py 中的 BaseChannel 抽象类。
/// - 替代 nanobot/channels/manager.py 中的 _dispatch_outbound 分发逻辑。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::bus::MessageBus;

// ============================================================================
// Channel Trait - 各平台渠道（webui/telegram/discord）需实现此 trait
// ============================================================================

/// Channel trait - 各平台渠道（webui/telegram/discord）需实现此 trait
#[async_trait::async_trait]
pub trait Channel: Send + Sync {
    /// 渠道名称，如 "webui", "telegram", "discord"
    fn name(&self) -> &str;
    /// 发送消息到该渠道
    async fn send_message(&self, chat_id: &str, content: &str) -> Result<(), String>;
}

// ============================================================================
// ChannelManager - 负责消费 OutboundEvent 并路由到正确的 Channel
// ============================================================================

/// ChannelManager - 统一管理所有 Channel 的生命周期和消息路由
pub struct ChannelManager {
    bus: Arc<MessageBus>,
    channels: HashMap<String, Box<dyn Channel>>,
}

impl ChannelManager {
    pub fn new(bus: Arc<MessageBus>) -> Self {
        Self {
            bus,
            channels: HashMap::new(),
        }
    }

    /// 注册一个 channel
    pub fn register<C: Channel + 'static>(&mut self, channel: C) {
        let name = channel.name().to_string();
        info!("ChannelManager: registering channel \"{}\"", name);
        self.channels.insert(name, Box::new(channel));
    }

    /// 启动 ChannelManager - 运行 dispatch loop 消费 OutboundEvent
    pub fn start(mut self) -> JoinHandle<()> {
        info!("ChannelManager: starting dispatch loop");
        tokio::spawn(async move {
            self.dispatch_loop().await;
        })
    }

    /// Dispatch loop - 从 bus 消费 OutboundEvent 并路由到对应 Channel
    async fn dispatch_loop(&mut self) {
        loop {
            let event = self.bus.consume_outbound().await;
            let event = match event {
                Some(e) => e,
                None => {
                    info!("ChannelManager: shutdown signal received, stopping dispatch loop");
                    break;
                }
            };

            let channel_name = &event.channel;
            if let Some(channel) = self.channels.get(channel_name) {
                match channel.send_message(&event.chat_id, &event.content).await {
                    Ok(()) => {}
                    Err(e) => {
                        warn!("ChannelManager: failed to send to channel \"{}\": {}", channel_name, e);
                    }
                }
            } else {
                // Channel 不存在，log 并丢弃
                // M2 阶段这是正常的（telegram/discord 等渠道尚未实现）
                warn!(
                    "ChannelManager: no channel registered for \"{}\", dropping event (chat_id={})",
                    channel_name, event.chat_id
                );
            }
        }
    }
}

// ============================================================================
// Stub Channel 实现 - M2 阶段用于占位
// ============================================================================

/// WebUI Channel stub - M2 阶段只是 log，实际的消息推送通过 WebSocket 完成
pub struct WebUiChannel;

#[async_trait::async_trait]
impl Channel for WebUiChannel {
    fn name(&self) -> &str {
        "webui"
    }

    async fn send_message(&self, chat_id: &str, content: &str) -> Result<(), String> {
        // M2: WebUI 的消息推送尚未实现
        // 未来会在 ChannelManager 中维护 WebSocket 连接来推送消息
        info!("WebUiChannel: would send to chat_id={}: {}", chat_id, content);
        Ok(())
    }
}
