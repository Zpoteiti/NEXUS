# nexus-server

## 1. 一句话定位
nexus-server 是 NEXUS 的中枢编排层，负责连接 WebUI 与 Client 设备，驱动 Agent Loop 并维护全局状态。

## 2. 职责边界
### 负责什么
- 对外提供两类接口：`/api/*`（REST）与 `/ws`、`/ws/chat`（WebSocket）。
- 维护在线设备路由表、工具挂起等待表、会话与记忆数据。
- 运行 ReAct 主循环：调模型、下发工具、收集结果、产出回复。
- 执行设备鉴权、设备上线/下线生命周期管理。

### 不负责什么
- 不在服务端本机执行工具命令。
- 不定义跨端协议结构本体（协议由 `nexus-common` 提供且冻结）。
- 不承担 WebUI 页面渲染职责。

## 3. 架构决策（What + Why）
### 决策 A：工具调用采用 oneshot 挂起/唤醒
- What：Agent 发起工具调用后，为每个 `request_id` 建立 `tokio::sync::oneshot` 通道并挂起等待回传。
- Why：保证一次工具调用只会被一次结果唤醒，语义简单且并发下不串线。

### 决策 B：`request_id` 绑定设备归属
- What：`request_id` 使用 `"{device_id}:{uuid_v4()}"` 格式。
- Why：断线清理时可按前缀高效定位该设备的全部挂起请求。

### 决策 C：设备断线必须清理全部挂起 Sender
- What：设备连接退出时，立刻从挂起表移除并 drop 对应 `oneshot::Sender`。
- Why：若不清理，Agent 侧 Receiver 会永久等待，导致会话卡死。

### 决策 D：认证统一为 Device Token
- What：设备与用户会话均使用数据库可查、可吊销的 Device Token。
- Why：统一认证机制可降低实现复杂度并提升运维可控性，支持精确吊销单设备访问权限。

### 决策 E：消息分发采用 bus.rs 统一总线
- What：系统内消息统一走 `InboundEvent / OutboundEvent`，由 ChannelManager 分发到具体渠道实现。
- Why：将“渠道接入”和“Agent 推理”解耦，新增渠道时不改核心循环。

### 决策 F：模型错误响应不写入历史
- What：当模型返回错误完成态时，不落库到会话消息。
- Why：错误文本进入历史会污染后续上下文，容易形成持续失败闭环。

### 决策 G：工具声明统一为 `serde_json::Value`
- What：服务端存储和分发工具声明时只处理 JSON 值，不定义额外工具 Schema 结构体。
- Why：兼容内置、MCP、Skill 三类工具的异构字段并减少协议演进摩擦。

## 4. 与其他模块的关系
### 依赖谁
- 依赖 `nexus-common` 获取协议结构、共享常量与错误码语义。
- 依赖 PostgreSQL 16+（启用 pgvector）进行会话、用户、记忆持久化。

### 被谁依赖
- 被 `nexus-webui` 作为唯一后端依赖。
- 被 `nexus-client` 作为控制平面与消息调度中心依赖。

### 通信方式
- WebUI ↔ Server：HTTP REST（`/api/*`）+ WebSocket（`/ws/chat`）。
- Client ↔ Server：WebSocket（`/ws`）。

## 5. 环境要求与运行方式
### 环境要求
- 操作系统：仅 Linux
- 部署方式：仅 Docker Compose
- Rust 1.85+，edition 2024
- PostgreSQL 16+，必须安装 pgvector

### 关键环境变量清单
- `DATABASE_URL`：数据库连接串
- `ADMIN_TOKEN`：管理员注册校验令牌
- `LLM_API_KEY`：模型服务访问密钥
- `LLM_API_BASE`：模型服务地址
- `LLM_MODEL_NAME`：模型名称
- `SERVER_PORT`：服务监听端口
- `HEARTBEAT_TIMEOUT_SEC`：心跳超时剔除秒数
- `MAX_AGENT_ITERATIONS`：Agent 循环最大轮次
- `CONTEXT_WINDOW_TOKENS`：上下文窗口阈值
- `MAX_TOKENS`：单次响应输出上限
- `TEMPERATURE`：采样温度

### 运行方式
- 在 Linux 主机准备 `.env` 与数据库后，通过 Docker Compose 启动服务。
- 启动后由 WebUI 通过 `/api/*` 与 `/ws/chat` 访问，由 Client 通过 `/ws` 接入。
