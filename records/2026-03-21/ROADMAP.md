# NEXUS Roadmap（2026-03-21）

## M0 — 文档基建
**目标描述**  
本里程碑建立四个子项目的架构决策文档，统一模块边界、通信拓扑与认证口径。文档重点解释“为什么这样设计”，用于后续开发与 AI 新会话快速对齐。完成后，所有后续实现都以这些文档为准绳。

**核心文件列表**
- `nexus-common/README.md`
- `nexus-server/README.md`
- `nexus-client/README.md`
- `nexus-webui/README.md`

**难度**：🟢 简单  
**完成后可验证的现象**
- 四个 README 均包含“定位、边界、架构决策、模块关系、环境与运行”五个章节。
- 文档中认证机制统一描述为 Device Token，且通信链路保持 User ↔ WebUI ↔ Server ↔ Client(s)。

---

## M1 — 握手能跑通
**目标描述**  
本里程碑打通 Client 与 Server 的最小握手闭环：建立 `/ws` 连接、提交 Device Token、完成登录确认。目标是先让最小认证链路稳定，不引入模型与前端变量。完成后可作为后续所有功能的网络基础。

**核心文件列表**
- `nexus-server/src/ws.rs`
- `nexus-server/src/auth.rs`
- `nexus-server/src/state.rs`
- `nexus-server/src/main.rs`
- `nexus-client/src/session.rs`
- `nexus-client/src/main.rs`

**难度**：🟡 中等  
**完成后可验证的现象**
- Client 启动后，Server 日志出现设备上线记录，Client 日志出现 `LoginSuccess`。
- 无需 WebUI 即可完成设备认证并保持心跳在线状态。

---

## M2 — 工具注册能跑通
**目标描述**  
本里程碑让 Client 将本地工具能力注册到 Server，并通过心跳 hash 维持工具快照一致性。Server 侧应可查询到每台设备的工具 Schema。完成后，Agent Loop 才具备“可调度能力集合”。

**核心文件列表**
- `nexus-client/src/discovery.rs`
- `nexus-client/src/tools/mod.rs`
- `nexus-client/src/tools/shell.rs`
- `nexus-client/src/session.rs`
- `nexus-server/src/state.rs`
- `nexus-server/src/tools_registry.rs`

**难度**：🟡 中等  
**完成后可验证的现象**
- Client 首次上线后立即上报 `RegisterTools`，Server 状态表可读取工具 Schema。
- 修改本地工具集后，心跳 hash 变化并触发重新注册。

---

## M3 — Agent Loop 跑通（核心里程碑）
**目标描述**  
本里程碑打通完整 ReAct 回路：用户消息进入 Server，Server 调模型，模型触发工具调用，Client 执行并回传，Server 再喂回模型得到最终回复。该里程碑是系统价值闭环的核心。完成后，NEXUS 具备端到端执行能力。

**核心文件列表**
- `nexus-server/src/agent_loop.rs`
- `nexus-server/src/providers/mod.rs`
- `nexus-server/src/providers/openai.rs`
- `nexus-server/src/context.rs`
- `nexus-server/src/state.rs`
- `nexus-server/src/ws.rs`
- `nexus-client/src/executor.rs`
- `nexus-client/src/guardrails.rs`
- `nexus-client/src/process.rs`

**难度**：🔴 困难  
**完成后可验证的现象**
- 用 `curl` 发送消息后，日志可观察到“模型请求 → 工具请求下发 → 工具结果回传 → 最终回复”全链路。
- 当设备断线时，挂起请求被清理，Agent 不会永久阻塞。

---

## M4 — WebUI 能用
**目标描述**  
本里程碑上线可用前端：用户可登录、发起会话、查看实时回复与工具调用过程。前端仅通过 Server 的 REST 与 `/ws/chat` 交互。完成后，系统可由浏览器端直接使用。

**核心文件列表**
- `nexus-webui/src/main.ts`
- `nexus-webui/src/router/index.ts`
- `nexus-webui/src/stores/user.ts`
- `nexus-webui/src/stores/app.ts`
- `nexus-webui/src/api/rest.ts`
- `nexus-webui/src/api/ws.ts`
- `nexus-webui/src/views/AuthView.vue`
- `nexus-webui/src/views/ChatView.vue`

**难度**：🟡 中等  
**完成后可验证的现象**
- 浏览器登录后可获得 Device Token 并进入聊天页。
- 聊天页可看到实时回复与工具调用过程流。

---

## M5 — 完善与扩展
**目标描述**  
本里程碑补齐增强能力：记忆系统、MCP Client、Skills、多渠道与管理能力。目标是在不破坏核心链路的前提下，增强系统可扩展性与可运营性。完成后，NEXUS 进入可持续迭代阶段。

**核心文件列表**
- `nexus-server/src/memory.rs`
- `nexus-server/src/db.rs`
- `nexus-server/src/channels/telegram.rs`
- `nexus-server/src/channels/mod.rs`
- `nexus-client/src/mcp_client.rs`
- `nexus-client/src/skills.rs`
- `nexus-webui/src/views/AdminView.vue`
- `nexus-webui/src/views/Settings.vue`

**难度**：🔴 困难  
**完成后可验证的现象**
- 记忆可写入并通过向量检索回注到上下文。
- 可挂载外部 MCP 工具、启用 Skills、启用 Telegram 渠道与管理后台能力。
