/// 职责边界：
/// 客户端启动入口，分三个阶段完成初始化并进入主循环。
///
/// 【阶段一：连接】
/// 1. 调用 config.rs 加载 ClientConfig（Server 地址、device_id、认证凭据等）。
/// 2. 调用 session.rs 建立与 Server 的 WebSocket 长连接（路径：/ws?device_id=...）。
/// 3. 连接成功后，Server 发起登录流程（ServerToClient::RequireLogin）；
///    Client 提交凭据完成认证，Server 将该设备绑定到对应 UserId。
///
/// 【阶段二：发现与注册】
/// 4. 调用 discovery.rs 扫描本地内置工具（如 shell 工具）。
/// 5. 调用 mcp_client.rs 连接并扫描所有外部 MCP Server，获取其工具 Schema。
/// 6. 将内置工具与 MCP 工具的 Schema 聚合，通过 session.rs 的 WebSocket 连接
///    发送 ClientToServer::RegisterTools 给 Server，完成工具注册。
///
/// 【阶段三：主循环】
/// 7. 在 session.rs 中开启心跳循环，定期发送 ClientToServer::Heartbeat
///    （含当前工具列表的 tools_hash，供 Server 检测工具变更）。
/// 8. 开启指令接收大循环，监听 ServerToClient 消息，
///    将 ExecuteToolRequest 等指令分发给 executor.rs 处理，
///    将执行结果封装为 ClientToServer::ToolExecutionResult 回传 Server。

mod config;
mod session;
use tracing::{info, warn};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config = config::load_config();
    let mut session = session::connect_and_loop(config).await;

    while let Some(message) = session.recv().await {
        info!("received server message: {:?}", message);
    }

    warn!("session inbound channel closed");
}
