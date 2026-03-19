/// 职责边界：
/// 1. 定义系统内部统一的消息流结构 (InboundEvent, OutboundEvent)。
/// 2. 管理连接 Channels 和 Agent Loop 的 tokio::sync::mpsc 管道。
/// 3. 绝对不涉及任何特定平台（如 Telegram）的特定 API 或字段。
///
/// 参考 nanobot：
/// - 替代 `nanobot/bus/events.py` 中的 `InboundMessage` 和 `OutboundMessage`。
/// - 替代 `nanobot/bus/queue.py` 的队列管理功能。

// TODO: 定义 InboundEvent struct (包含 channel_id, sender_id, content, metadata 等)
// TODO: 定义 OutboundEvent struct
// TODO: 建立 mpsc::channel 初始化的 helper 函数，返回 (Sender, Receiver)