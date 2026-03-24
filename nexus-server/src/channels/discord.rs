/// 职责边界：
/// 1. 专门且仅处理 discord 平台的 API 交互 (使用类似 `teloxide` 的 Rust crate)。
/// 2. 负责将 discord 的原生 Update 转换为 `bus.rs` 中的 `InboundEvent`，并推入 inbound 队列。
/// 3. 实现 `Channel` Trait，接收 `OutboundEvent` 并转换为 discord 的发送动作。
/// 4. 处理 discord 特有的逻辑，如“正在输入”状态循环、媒体文件的下载/合并。
///
/// 参考 nanobot：
/// - 1:1 移植 `nanobot/channels/discord.py` 的业务逻辑。
/// - 参考它的长轮询启动机制和 `_typing_loop` 反馈体验优化。
///
/// 【平台用户绑定流程】
/// 收到 discord 消息时，先用 sender_id（格式："{user.id}|{user.username}"）
/// 调用 db::get_user_by_channel_identity("discord", sender_id) 查找对应 UserId。
/// 若返回 None（首次接入），则向用户发送绑定引导消息，
/// 要求用户通过 WebUI 登录后在 Settings 页完成渠道绑定，绑定后继续处理消息。
/// 参考 nanobot：nanobot/channels/discord.py sender_id 构造；
///              nanobot/channels/base.py is_allowed()（L79-87）。
///
/// 【Forum Topic / Thread 级别的 Session 隔离】
/// discord 超级群组支持 Forum Topic（thread_id）。
/// 不同 topic 应产生独立的 session_key，避免多话题共享上下文：
///   session_key = "discord:{chat_id}:topic:{thread_id}"（有 thread_id 时）
///   session_key = "discord:{chat_id}"（普通群聊/私聊）
/// 参考 nanobot：nanobot/channels/discord.py _derive_topic_session_key()（L525-530）。
///
/// 【发送侧 429 退避】
/// 调用 discord Bot API 发送消息时，若收到 HTTP 429，
/// 读取响应头中的 retry_after 字段，等待对应秒数后重试。
/// 最多重试 3 次，超限后将错误作为 OutboundEvent 发送失败记录。
/// 参考 nanobot：nanobot/channels/discord.py _send_payload()（L143-160）。


// TODO: 定义 discordChannel struct
// TODO: 为 discordChannel 实现 Channel Trait