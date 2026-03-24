/**
 * 职责边界：
 * 1. 专门管理与 `nexus-server/ws/chat` 的 WebSocket 长连接。
 * 2. 负责发送用户的聊天输入，并接收 Agent 的流式回复和状态更新。
 * 3. 维护断线自动重连机制，并将收到的消息 dispatch 给全局状态库 (Pinia)。
 */

// TODO: class ChatWebSocketManager { connect(), sendMessage(), onMessage() ... }