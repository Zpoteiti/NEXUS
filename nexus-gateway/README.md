# nexus-gateway

## 1. 一句话定位
nexus-gateway 是 NEXUS 的浏览器接入层，负责管理浏览器 WebSocket 连接，并将用户消息桥接到 nexus-server。

## 2. 职责边界
### 负责什么
- 接受浏览器 WebSocket 连接（`/ws/browser`），分配 `chat_id`，转发用户消息。
- 接受 nexus-server 的 WebSocket 连接（`/ws/nexus`），验证 gateway token，双向路由消息。
- 将来自浏览器的消息格式转换后转发给 nexus-server，将 nexus-server 的回复路由回对应浏览器。

### 不负责什么
- 不执行工具、不运行 Agent Loop。
- 不持久化会话或消息。
- 不直接连接 nexus-client 设备。

## 3. 架构决策（What + Why）
### 决策 A：nexus-server 主动连接 gateway（WS client 模式）
- What：nexus-server 的 `GatewayChannel` 作为 WS client，主动连接 gateway 的 `/ws/nexus`。
- Why：保持 nexus-server 对所有外部渠道的主动连接一致性，简化 gateway 部署（只需对外暴露端口，无需知道 server 地址）。

### 决策 B：独立 Rust binary，不嵌入 nexus-server
- What：nexus-gateway 是独立的 `cargo run --package nexus-gateway` 进程。
- Why：可以独立部署在网络边界，nexus-server 可以在内网运行。

### 决策 C：gateway token 认证
- What：nexus-server 连接 `/ws/nexus` 时必须发送匹配的 `NEXUS_GATEWAY_TOKEN`。
- Why：防止未授权的 nexus-server 实例接入 gateway。

## 4. 与其他模块的关系
### 依赖谁
- 不依赖 nexus-common（协议结构独立定义）。

### 被谁依赖
- 被浏览器（用户）通过 `/ws/browser` 连接。
- 被 nexus-server（GatewayChannel）通过 `/ws/nexus` 连接。

### 通信方式
- Browser ↔ Gateway：WebSocket (`/ws/browser`)
- Gateway ↔ Server：WebSocket (`/ws/nexus`，server 主动连接）

## 5. 环境要求与运行方式
### 环境要求
- Rust 1.85+（edition 2024）

### 环境变量
| 变量 | 默认值 | 说明 |
|------|--------|------|
| `GATEWAY_PORT` | `9090` | 监听端口 |
| `NEXUS_GATEWAY_TOKEN` | —（必填）| nexus-server 认证 token |

### 运行方式
```bash
cd NEXUS
NEXUS_GATEWAY_TOKEN=your-token cargo run --package nexus-gateway
```
