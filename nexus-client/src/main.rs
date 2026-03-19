/// 职责边界：
/// 1. 客户端的启动入口。
/// 2. 负责读取配置、建立与 Server 的 WebSocket 连接 (`/ws?device_id=...`)。
/// 3. 开启接收循环，将 Server 下发的指令分发给 executor 或 mcp_client。

mod executor;
mod mcp_client;

#[tokio::main]
async fn main() {
    // TODO: 建立 WebSocket 连接
    // TODO: 注册本地工具与 MCP 工具 (RegisterTools)
    // TODO: 开启心跳循环 (Heartbeat)
    // TODO: 开启接收指令的大循环
    todo!("Implement client entrypoint")
}