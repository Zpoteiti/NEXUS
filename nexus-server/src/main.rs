/// 职责边界：
/// 1. 负责程序的启动、环境变量读取 (.env)、数据库连接池 (PgPool) 的初始化。
/// 2. 挂载 Axum 的路由 (HTTP API 和 WebSocket)。
/// 3. 绝对不要在这里写具体的 WebSocket 收发逻辑或 LLM 提示词逻辑。

mod state;
mod ws;
mod llm;

#[tokio::main]
async fn main() {
    // TODO: 初始化 dotenvy
    // TODO: 连接 PostgreSQL
    // TODO: 初始化 AppState
    // TODO: 启动 Axum 服务器
    todo!("Initialize the server boilerplate")
}