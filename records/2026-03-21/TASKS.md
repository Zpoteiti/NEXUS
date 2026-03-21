# NEXUS 任务分解（按依赖顺序）

## M0（已完成）

M0-T1：编写 nexus-common 架构 README（nexus-common/README.md）  
依赖 ：无  
涉及文件 ：`nexus-common/README.md`  
任务描述 ：明确协议冻结、Device Token 规则、退出码语义、工具输出截断策略与模块边界。  
完成标志 ：文件已落地且包含五个必需章节，状态：已完成。  

M0-T2：编写 nexus-server 架构 README（nexus-server/README.md）  
依赖 ：M0-T1  
涉及文件 ：`nexus-server/README.md`  
任务描述 ：明确挂起唤醒机制、断线清理、消息总线架构、模型错误不入历史与运行配置。  
完成标志 ：文件已落地且包含五个必需章节，状态：已完成。  

M0-T3：编写 nexus-client 架构 README（nexus-client/README.md）  
依赖 ：M0-T2  
涉及文件 ：`nexus-client/README.md`  
任务描述 ：明确三阶段启动、重连全握手、心跳 hash 热拔插、guardrails、工具命名前缀。  
完成标志 ：文件已落地且包含五个必需章节，状态：已完成。  

M0-T4：编写 nexus-webui 架构 README（nexus-webui/README.md）  
依赖 ：M0-T2  
涉及文件 ：`nexus-webui/README.md`  
任务描述 ：明确双连接分工、路由守卫集中化、前端认证与技术栈边界。  
完成标志 ：文件已落地且包含五个必需章节，状态：已完成。  

---

## M1（握手能跑通）

M1-T0：实现服务端配置加载与启动骨架（nexus-server/src/config.rs, main.rs）  
依赖 ：无  
涉及文件 ：`nexus-server/src/config.rs`、`nexus-server/src/main.rs`  
任务描述 ：实现环境变量加载、数据库初始化、全局状态与路由挂载。  
完成标志 ：启动后监听端口成功，日志显示 Server ready。  

M1-T1：实现 Device Token 校验与登录握手（nexus-server/src/auth.rs, ws.rs）  
依赖 ：M1-T0  
涉及文件 ：`nexus-server/src/auth.rs`、`nexus-server/src/ws.rs`  
任务描述 ：实现 RequireLogin → SubmitToken → LoginSuccess/LoginFailed 完整流程。  
完成标志 ：Client 提交有效 token 后收到 LoginSuccess，无效 token 收到 LoginFailed 并断开。  

M1-T2：实现 Client 会话连接与认证提交（nexus-client/src/session.rs, main.rs）  
依赖 ：M1-T1  
涉及文件 ：`nexus-client/src/session.rs`、`nexus-client/src/main.rs`  
任务描述 ：建立 `/ws` 连接并按握手序列提交 Device Token。  
完成标志 ：Client 终端出现 LoginSuccess，Server 记录设备在线。  

M1-T3：实现上线状态登记与心跳最小链路（nexus-server/src/state.rs, nexus-client/src/session.rs）  
依赖 ：M1-T2  
涉及文件 ：`nexus-server/src/state.rs`、`nexus-client/src/session.rs`  
任务描述 ：握手成功后登记设备并定时上报心跳。  
完成标志 ：超过一个心跳周期后设备仍在线，断心跳后按超时策略剔除。  

---

## M2（工具注册能跑通）

M2-T0：实现本地工具发现与 schema 组装（nexus-client/src/discovery.rs, tools/mod.rs, tools/shell.rs）  
依赖 ：M1-T3  
涉及文件 ：`nexus-client/src/discovery.rs`、`nexus-client/src/tools/mod.rs`、`nexus-client/src/tools/shell.rs`  
任务描述 ：发现内置工具并组装为统一 JSON Schema 列表。  
完成标志 ：Client 可打印非空工具 schema 数量。  

M2-T1：实现 RegisterTools 上报与服务端落状态（nexus-client/src/session.rs, nexus-server/src/state.rs）  
依赖 ：M2-T0  
涉及文件 ：`nexus-client/src/session.rs`、`nexus-server/src/state.rs`  
任务描述 ：登录后上报工具列表，Server 将 schema 写入设备快照。  
完成标志 ：Server 内存状态可查询到对应 device_id 的 tools 列表。  

M2-T2：实现心跳 hash 热拔插与自动重注册（nexus-client/src/session.rs, nexus-server/src/ws.rs）  
依赖 ：M2-T1  
涉及文件 ：`nexus-client/src/session.rs`、`nexus-server/src/ws.rs`  
任务描述 ：hash 变化时补发全量 RegisterTools，Server 覆盖旧快照。  
完成标志 ：新增/删除工具后下一个心跳周期内 Server 端工具列表同步更新。  

---

## M3（Agent Loop 跑通）

M3-T0：实现模型 Provider 统一接口与重试策略（nexus-server/src/providers/mod.rs, openai.rs）  
依赖 ：M2-T2  
涉及文件 ：`nexus-server/src/providers/mod.rs`、`nexus-server/src/providers/openai.rs`  
任务描述 ：实现 chat_with_retry、工具调用解析与错误降级返回。  
完成标志 ：构造瞬时错误时可见退避重试日志，最终返回标准响应结构。  

M3-T1：实现 Agent Loop 主状态机（nexus-server/src/agent_loop.rs, context.rs）  
依赖 ：M3-T0  
涉及文件 ：`nexus-server/src/agent_loop.rs`、`nexus-server/src/context.rs`  
任务描述 ：实现消息拼装、模型调用、工具调用循环与最终回复。  
完成标志 ：单次请求可完成至少一轮“模型文本回复”闭环。  

M3-T2：实现 oneshot 挂起/唤醒与 request_id 规则（nexus-server/src/agent_loop.rs, state.rs, ws.rs）  
依赖 ：M3-T1  
涉及文件 ：`nexus-server/src/agent_loop.rs`、`nexus-server/src/state.rs`、`nexus-server/src/ws.rs`  
任务描述 ：按 `"{device_id}:{uuid_v4()}"` 生成请求标识并完成等待表唤醒。  
完成标志 ：工具结果回传后对应请求立即解除挂起并继续下一轮循环。  

M3-T3：实现客户端执行链路与标准退出码（nexus-client/src/executor.rs, guardrails.rs, process.rs）  
依赖 ：M3-T2  
涉及文件 ：`nexus-client/src/executor.rs`、`nexus-client/src/guardrails.rs`、`nexus-client/src/process.rs`  
任务描述 ：执行前强制校验，按统一退出码回传结果并支持超时控制。  
完成标志 ：成功、失败、超时、取消、校验失败五类结果均可复现。  

M3-T4：实现断线清理防永久挂起（nexus-server/src/ws.rs, state.rs）  
依赖 ：M3-T2  
涉及文件 ：`nexus-server/src/ws.rs`、`nexus-server/src/state.rs`  
任务描述 ：设备断线时 drop 该设备全部挂起 Sender。  
完成标志 ：断线后挂起中的请求立即返回失败结果，Agent Loop 不阻塞。  

---

## M4（WebUI 能用）

M4-T0：搭建前端入口与路由守卫（nexus-webui/src/main.ts, router/index.ts）  
依赖 ：M1-T1  
涉及文件 ：`nexus-webui/src/main.ts`、`nexus-webui/src/router/index.ts`  
任务描述 ：初始化应用、配置路由、集中实现登录与管理员守卫。  
完成标志 ：未登录访问受限页自动跳转登录页。  

M4-T1：实现用户状态与认证持久化（nexus-webui/src/stores/user.ts, api/rest.ts）  
依赖 ：M4-T0  
涉及文件 ：`nexus-webui/src/stores/user.ts`、`nexus-webui/src/api/rest.ts`  
任务描述 ：登录后保存 Device Token 并用于后续请求。  
完成标志 ：刷新页面后仍保持登录态并可访问受限接口。  

M4-T2：实现聊天实时通道（nexus-webui/src/api/ws.ts, views/ChatView.vue, stores/app.ts）  
依赖 ：M4-T1, M3-T3  
涉及文件 ：`nexus-webui/src/api/ws.ts`、`nexus-webui/src/views/ChatView.vue`、`nexus-webui/src/stores/app.ts`  
任务描述 ：建立 `/ws/chat` 连接并渲染消息流与工具执行过程。  
完成标志 ：浏览器发送消息后可实时看到工具调用过程与最终回复。  

M4-T3：实现设置页与基础管理页（nexus-webui/src/views/Settings.vue, AdminView.vue）  
依赖 ：M4-T1  
涉及文件 ：`nexus-webui/src/views/Settings.vue`、`nexus-webui/src/views/AdminView.vue`  
任务描述 ：完成用户偏好配置入口与管理员功能入口。  
完成标志 ：普通用户无法访问管理页，管理员可进入管理页。  

---

## M5（完善与扩展）

M5-T0：实现记忆整合与向量检索（nexus-server/src/memory.rs, db.rs, context.rs）  
依赖 ：M3-T1  
涉及文件 ：`nexus-server/src/memory.rs`、`nexus-server/src/db.rs`、`nexus-server/src/context.rs`  
任务描述 ：实现 consolidation、向量入库、RAG 检索注入。  
完成标志 ：数据库出现 memory_chunks 数据，后续对话可命中相关记忆片段。  

M5-T1：实现 MCP Client 完整接入（nexus-client/src/mcp_client.rs, discovery.rs）  
依赖 ：M2-T2  
涉及文件 ：`nexus-client/src/mcp_client.rs`、`nexus-client/src/discovery.rs`  
任务描述 ：拉起 MCP 进程、拉取工具、注册与调用。  
完成标志 ：可调用 `mcp_` 前缀工具并拿到可读结果。  

M5-T2：实现 Skills 扫描、注册与执行（nexus-client/src/skills.rs, executor.rs）  
依赖 ：M2-T2  
涉及文件 ：`nexus-client/src/skills.rs`、`nexus-client/src/executor.rs`  
任务描述 ：扫描 SKILL.md 生成 schema，支持 `skill_` 前缀工具执行。  
完成标志 ：新增一个 skill 后可被注册并被模型成功调用。  

M5-T3：实现 Telegram 渠道（nexus-server/src/channels/telegram.rs, channels/mod.rs）  
依赖 ：M3-T1  
涉及文件 ：`nexus-server/src/channels/telegram.rs`、`nexus-server/src/channels/mod.rs`  
任务描述 ：接入 Telegram 收发链路并接入总线分发。  
完成标志 ：Telegram 端发消息可触发 Agent 回复。  

M5-T4：完善管理后台（nexus-webui/src/views/AdminView.vue, api/rest.ts）  
依赖 ：M4-T3  
涉及文件 ：`nexus-webui/src/views/AdminView.vue`、`nexus-webui/src/api/rest.ts`  
任务描述 ：补齐设备、用户、统计与配置管理接口与页面。  
完成标志 ：管理员可在后台查看设备与用户列表并执行管理操作。  
