/// 职责边界：
/// 1. 定义异步的 `Channel` Trait，规范各平台 Channel 的统一接口（start、stop、send 等）。
/// 2. 实现 `ChannelManager`，负责系统启动时加载各启用渠道并管理其生命周期。
///    与 bus.rs 的边界约定：
///    - ChannelManager 在启动时从 bus::init() 拿到 outbound_rx。
///    - ChannelManager 运行一个后台 Task 消费 outbound_rx，
///      根据 OutboundEvent.channel_name 路由给对应的 Channel 实例。
///    - 各具体 Channel（telegram / webui）在注册到 ChannelManager 时，
///      从 ChannelManager 拿到一份 inbound_tx 的克隆，
///      用于将平台收到的消息推入总线（inbound 方向）。
/// 3. ChannelManager 不直接处理消息内容，只负责分发路由；
///    消息格式转换由各 Channel 实例（telegram.rs / webui.rs）内部完成。
///
/// 参考 nanobot：
/// - 替代 nanobot/channels/base.py 中的 BaseChannel 抽象类。
/// - 替代 nanobot/channels/manager.py 中的 _dispatch_outbound 分发逻辑。

// TODO: 定义 #[async_trait] pub trait Channel { async fn start(...); async fn send(...); }
// TODO: 实现 ChannelManager 结构体及 register、start_all、dispatch_loop 方法
