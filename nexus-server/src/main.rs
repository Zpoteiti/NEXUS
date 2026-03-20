/// 职责边界：
/// 1. 负责程序的启动、环境变量读取 (.env)、数据库连接池 (PgPool) 的初始化。
/// 2. 调用 bus::init() 创建消息管道，初始化 AppState，启动 ChannelManager。
/// 3. 挂载 Axum 的路由（HTTP API 路由来自 api.rs，WebSocket 路由来自 ws.rs）。
/// 4. 绝对不要在这里写具体的 WebSocket 收发逻辑或 LLM 提示词逻辑。

mod agent_loop;
mod api;
mod bus;
mod channels;
mod context;
mod db;
mod providers;
mod state;
mod tools_registry;
mod ws;

#[tokio::main]
async fn main() {
    // TODO: 初始化 dotenvy
    // TODO: 连接 PostgreSQL（db.rs）
    // TODO: 调用 bus::init() 获取四端点
    // TODO: 初始化 AppState（state.rs）
    // TODO: 启动 ChannelManager（channels/mod.rs）
    // TODO: 启动 Axum 服务器，挂载 api.rs 和 ws.rs 的路由
    todo!("Initialize the server boilerplate")
}
