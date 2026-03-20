/// 职责边界：
/// 1. 定义系统内部统一的消息流结构（InboundEvent、OutboundEvent）。
///    - InboundEvent：平台消息 → agent_loop，包含 channel_name、sender_id、content 等字段
///    - OutboundEvent：agent_loop → 平台，包含 channel_name（用于路由）、recipient_id、content 等字段
/// 2. 提供唯一的初始化函数 `init()`，创建两对 mpsc channel 并返回四个端点：
///    (inbound_tx, inbound_rx, outbound_tx, outbound_rx)
///    Sender/Receiver 归属约定：
///    - inbound_tx  → 各 Channel 实例持有（telegram / webui 等），向总线推送平台消息
///    - inbound_rx  → agent_loop 持有，从总线拉取待处理消息
///    - outbound_tx → agent_loop 持有，向总线推送回复消息
///    - outbound_rx → ChannelManager 持有，分发给具体 Channel 实例
/// 3. bus.rs 本身不运行任何后台 Task，不持有任何 Sender/Receiver，
///    只负责创建管道和定义消息类型，生命周期管理由调用方（main.rs）负责。
///
/// 参考 nanobot：
/// - 替代 nanobot/bus/events.py 中的 InboundMessage / OutboundMessage 定义。
/// - 替代 nanobot/bus/queue.py 的队列初始化逻辑。

// TODO: 定义 InboundEvent struct（channel_name, sender_id, content, metadata 等）
// TODO: 定义 OutboundEvent struct（channel_name, recipient_id, content 等）
// TODO: 实现 pub fn init() -> (inbound_tx, inbound_rx, outbound_tx, outbound_rx)
