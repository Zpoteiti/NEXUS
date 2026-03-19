/// 职责边界：
/// 1. 专门且仅处理 Telegram 平台的 API 交互 (使用类似 `teloxide` 的 Rust crate)。
/// 2. 负责将 Telegram 的原生 Update 转换为 `bus.rs` 中的 `InboundEvent`，并推入 inbound 队列。
/// 3. 实现 `Channel` Trait，接收 `OutboundEvent` 并转换为 Telegram 的发送动作。
/// 4. 处理 Telegram 特有的逻辑，如“正在输入”状态循环、媒体文件的下载/合并。
///
/// 参考 nanobot：
/// - 1:1 移植 `nanobot/channels/telegram.py` 的业务逻辑。
/// - 参考它的长轮询启动机制和 `_typing_loop` 反馈体验优化。

// TODO: 定义 TelegramChannel struct
// TODO: 为 TelegramChannel 实现 Channel Trait