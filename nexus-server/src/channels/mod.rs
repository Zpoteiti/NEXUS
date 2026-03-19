/// 职责边界：
/// 1. 定义异步的 `Channel` Trait（规范 start, stop, send 等行为）。
/// 2. 实现 `ChannelManager`，负责在系统启动时加载各个启用的渠道，并管理它们的生命周期。
/// 3. 运行一个后台 Task，从 bus 的 outbound 队列消费消息，并根据 msg.channel_name 路由给具体的 Channel 实例。
///
/// 参考 nanobot：
/// - 替代 `nanobot/channels/base.py` 中的 `BaseChannel` 抽象类。
/// - 替代 `nanobot/channels/manager.py` 中的分发逻辑 `_dispatch_outbound`。

// TODO: 定义 #[async_trait] pub trait Channel { ... }
// TODO: 实现 ChannelManager 结构体及其 start_all, dispatch_loop 方法