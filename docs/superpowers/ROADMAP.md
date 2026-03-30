# NEXUS Roadmap

> 最后更新：2026-03-30

## 架构概览

```
User ↔ Browser ↔ nexus-webui (Rust gateway) ↔ nexus-server ↔ nexus-client(s)
                                                      ↑
                              Discord/Telegram Channel（直连）
```

**nexus-webui** 是独立的 Rust WebSocket gateway，不是前端框架。浏览器通过 `/ws/browser` 连接，nexus-server 的 `WebuiChannel` 通过 `/ws/nexus` 连接。

---

## ✅ M0 — 文档基建（已完成）

四个模块均建立了架构 README，统一了 Device Token 认证方案与通信拓扑。

---

## ✅ M1 — 握手能跑通（已完成）

Client ↔ Server WebSocket 握手、Device Token 认证、心跳保活全部实现。

---

## ✅ M2 — 工具注册能跑通（已完成）

Client 发现内置工具（shell/fs）、MCP 工具、Skills，组装 schema 后通过 `RegisterTools` 上报。Server 存储工具快照，心跳 hash 变化时触发重注册。

---

## 🔄 M3 — Agent Loop 跑通（进行中）

**目标描述**
打通完整 ReAct 回路：用户消息 → Server → LLM → 工具调用 → Client 执行 → 结果回传 → 最终回复。

**当前状态**
- ✅ MessageBus session 隔离路由（DashMap）
- ✅ SessionManager 按需创建 session + spawn agent_loop
- ✅ agent_loop ReAct 状态机（循环调用 + 重复检测）
- ✅ Channel 抽象（主动 WS client 模式）+ ChannelManager
- ✅ WebuiChannel：连接 nexus-webui-server，收消息创建 session，发送回复
- ✅ MockLLM：两轮状态机，验证 ReAct 循环
- ✅ tools_registry：oneshot 挂起/唤醒，断线清理
- ⬜ 真实 LLM Provider（OpenAI/Claude API 接入）
- ⬜ context.rs：多轮会话历史拼装 + 设备列表注入
- ⬜ db.rs：session / message 持久化（create_session、save_message、get_history）
- ⬜ 多设备路由（device_name 注入 schema + 按用户隔离查找）

**核心文件**
- `nexus-server/src/agent_loop.rs`
- `nexus-server/src/providers/openai.rs`
- `nexus-server/src/context.rs`
- `nexus-server/src/tools_registry.rs`
- `nexus-server/src/db.rs`

**完成后可验证的现象**
- 发送消息后日志出现"LLM 调用 → 工具下发 → 工具结果 → 最终回复"完整链路
- 多轮对话历史正确传入 LLM 上下文
- 断线时挂起请求立即失败，agent_loop 不阻塞

---

## 🔄 M4 — WebUI 能用（进行中）

**目标描述**
用户可通过浏览器发起会话、查看实时回复与工具执行过程。

**当前状态**
- ✅ nexus-webui 独立 Rust gateway（`/ws/browser` + `/ws/nexus` 双端点）
- ✅ WebuiChannel 连接 gateway，双向消息桥接
- ⬜ 浏览器端聊天 UI（HTML/CSS/JS，或轻量前端框架）
- ⬜ 用户认证（JWT 或 session token，gateway 侧验证）
- ⬜ 消息流实时渲染（工具调用过程、流式回复）

**核心文件**
- `nexus-webui/src/main.rs`（gateway 服务，已完成）
- `nexus-webui/src/browser.rs`（Browser WS handler，已完成）
- `nexus-webui/src/gateway.rs`（Nexus WS handler，已完成）
- 待新增：浏览器静态资源（`nexus-webui/static/`）

**完成后可验证的现象**
- 启动 `cargo run --package nexus-webui` 后浏览器可访问聊天页面
- 登录后发送消息可实时看到 AI 回复与工具执行过程

---

## ⬜ M5 — 完善与扩展

**目标描述**
补齐记忆系统、MCP Client、Skills、多渠道（Discord/Telegram）与管理能力。

**核心内容**
- 记忆整合：向量写入、RAG 检索、consolidation（`nexus-server/src/memory.rs`）
- MCP Client：nexus-client 拉起 MCP 进程、发现工具、注册与调用
- Skills：扫描 SKILL.md、生成 schema、支持 `skill_` 前缀工具
- Discord/Telegram Channel：接入 DiscordChannel/TelegramChannel
- 管理后台：设备、用户、统计查询 API

**完成后可验证的现象**
- 数据库 `memory_chunks` 表有数据，后续对话可命中历史记忆
- 可挂载 MCP Server 并调用 `mcp_` 前缀工具
- Discord/Telegram 端消息可触发 Agent 回复
