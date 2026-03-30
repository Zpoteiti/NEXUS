//! 职责边界：
//! 1. 将 WebUI 页面当做一个普通的聊天渠道 (Channel)，实现 `Channel` Trait。
//! 2. 暴露一个专属的聊天 WebSocket 路由（例如 `/ws/chat`），区别于控制底层手脚的 `/ws`。
//! 3. 接收网页发来的 JSON 聊天记录，转换为统一的 `InboundEvent` 投入总线。
//! 4. 监听总线分发给 `webui` 的 `OutboundEvent`，转为 JSON 推送给前端网页。
//!
//! 参考 nanobot：
//! - 逻辑结构与 `telegram.rs` 完全一致。Telegram 是调外部 SDK 收发，而这里是直接读写 WebSocket stream。

// TODO: 定义 WebUiChannel struct 并实现 Channel Trait
// TODO: 定义 webui_ws_upgrade_handler 和收发循环
