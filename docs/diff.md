# NEXUS M3 vs nanobot 架构对比分析

> 本文档对比 NEXUS（Rust 分布式 Agent 系统）M3 分支与 nanobot（Python 单体 Agent 框架）的实现差异，
> 分析每个设计决策背后的原因，以及是否为更优选择。

---

## 1. 整体架构：分布式 vs 单体

| 维度 | nanobot | NEXUS |
|------|---------|-------|
| 架构 | 单进程单体，agent loop + tool 执行在同一进程 | Server/Client 分离，Server 编排 agent loop，Client 远程执行 tool |
| 语言 | Python 3.11+，asyncio | Rust 1.85+，tokio |
| 通信 | 进程内函数调用 | WebSocket + JSON-RPC（Server↔Client），HTTP REST（管理 API） |
| 部署 | 单机部署，一个进程搞定 | 多机部署：Server 中心化，Client 可部署在任意机器 |

**为什么 NEXUS 要做分离？**
- 核心需求：让 LLM 能操控多台远程设备（家里的 NAS、公司的服务器、笔记本……）
- nanobot 是「一个 bot 一台机器」，NEXUS 是「一个 bot 多台机器」
- Server 不信任 Client，Client 不暴露 API——只建立出站 WebSocket 连接

**是否值得？** 如果只在一台机器上用，这个架构是过度设计。但如果目标就是远程多设备编排，这是合理的最小架构。

---

## 2. Agent Loop

| 维度 | nanobot | NEXUS |
|------|---------|-------|
| 位置 | `agent/runner.py` | `nexus-server/src/agent_loop.rs` |
| 最大迭代 | 200（可配置） | 无硬编码限制（consts.rs 定义了 40 但未使用） |
| 并发工具 | `asyncio.gather()` 并行执行 `concurrency_safe` 工具 | 所有 tool call 串行发送到 Client |
| 流式输出 | 支持 streaming delta + hook 系统 | 不支持 streaming |
| 生命周期 hook | `AgentHook` 抽象类，`before_iteration` / `after_iteration` | 无 hook 系统 |
| 检查点 | 每批 tool 执行后 `_emit_checkpoint()` | 无检查点 |

**M3 做了什么：**
- 修复了 OpenAI 兼容协议问题：并行 tool_calls 必须合并为单个 assistant 消息
- 修复了 loop detection 分支：必须为所有 tool_call_id 返回结果
- DB 历史重建：合并连续的 assistant tool_call 行

**差异分析：**
- nanobot 的 hook 系统和 checkpoint 是成熟框架的标志，NEXUS 目前不需要——它的 agent loop 更简单
- 并发工具执行在 NEXUS 架构下更复杂（需要跨网络协调），目前串行是合理的折中
- **缺失：** NEXUS 应该加上最大迭代限制，防止 runaway loop

---

## 3. MCP 集成

| 维度 | nanobot | NEXUS |
|------|---------|-------|
| 连接方式 | MCP Python SDK (`mcp` 包)，支持 stdio/SSE/streamableHttp | 自研 JSON-RPC over stdio（手写协议） |
| 配置来源 | `config.json` 中的 `tools.mcp_servers` | REST API `PUT /api/devices/{name}/mcp`，通过握手/心跳下发 |
| 命名规则 | `mcp_{server}_{tool}` | `mcp_{server}_{tool}`（相同） |
| 工具路由 | `ToolRegistry.prepare_call()` 按名称查找 | `McpClientManager.call_tool()` 遍历 session 查找 |
| 热加载 | 懒连接，首次消息时连 | 心跳检测配置变化，但实际连接只初始化一次（bug） |

**M3 做了什么：**
- MCP 配置从环境变量迁移到 REST API
- 通过心跳下发 MCP 配置（与 FsPolicy 同模式）
- 修复了 stdio `read_to_end()` 阻塞 bug → 改为逐行 `read_line()`
- 修复了工具名解析 bug（`mcp_MiniMax_web_search` 被错误分割）→ 改为遍历已注册 session

**差异分析：**
- nanobot 用官方 MCP SDK，更标准；NEXUS 手写协议，更轻量但容易有 bug（如 read_to_end）
- NEXUS 的 REST API 配置方式更适合多设备场景：管理员可以远程配置每台设备的 MCP
- nanobot 的 `enabled_tools` 过滤功能更精细，NEXUS 目前是全量注册
- **已知 bug：** 心跳更新 MCP 配置后不会重新连接 MCP Server（`MCP_INITIALIZED` 是 one-shot）

---

## 4. 记忆系统

| 维度 | nanobot | NEXUS |
|------|---------|-------|
| 存储 | 文件系统（`MEMORY.md` + `HISTORY.md`） | PostgreSQL + pgvector |
| 检索 | 全文（LLM 读 MEMORY.md） | 向量相似度搜索（RAG） |
| 合并策略 | LLM 驱动：让 LLM 调用 `save_memory` 更新 MEMORY.md | LLM 驱动：`save_memory` tool + 自动 consolidation |
| 去重 | 隐式（LLM 负责不写重复内容） | 显式：写入时 cosine > 0.92 跳过 |
| 触发条件 | token 超出 context_window 时 | 消息数 > 阈值时 |
| 嵌入模型 | 无（纯文本） | 可配置（REST API 设置 embedding model） |

**M3 做了什么：**
- 实现了 embedding config REST API，支持热切换嵌入模型
- 写入时去重（cosine > 0.92 = 跳过）
- `embed_text_throttled` 信号量控制嵌入并发
- 切换模型后自动后台 re-embed 所有记忆
- consolidation 输出长度限制为 `max_input_length * 0.8`

**差异分析：**
- NEXUS 的 RAG 记忆明显更强大：向量检索比全文匹配更精准
- nanobot 的文件系统方案更简单、可移植、可人工编辑
- NEXUS 的显式去重比 nanobot 依赖 LLM 判断更可靠
- **权衡：** RAG 需要额外的嵌入服务和数据库，运维复杂度更高

---

## 5. 上下文管理

| 维度 | nanobot | NEXUS |
|------|---------|-------|
| 历史长度 | 500 条（`get_history(max_messages=500)`） | 500 条（共享常量 `MAX_HISTORY_MESSAGES`） |
| 截断策略 | 从 `last_consolidated` 开始，对齐到 user turn | 从末尾取 N 条，`find_legal_start` 跳到首个 user 消息 |
| 孤儿处理 | `find_legal_message_start()` 跳过开头的 tool result | `truncate_and_fix_orphans()` 类似但有 bug |
| token 估算 | tiktoken / provider API | 无（按消息条数截断） |
| system prompt | identity + bootstrap + memory + skills | identity + soul + 设备列表 + 技能 + RAG 记忆 |

**M3 做了什么：**
- 将 `MAX_HISTORY_MESSAGES` 从 server 本地重定义改为使用共享常量
- 实现了 always-on skill 注入到 system prompt

**差异分析：**
- nanobot 按 token 截断更精确，NEXUS 按条数截断可能浪费或超出 context window
- nanobot 的 consolidation boundary 对齐到 user turn 更安全
- **NEXUS 应改进：** 加入 token 估算，避免发送超出 context_window 的请求

---

## 6. 工具执行与安全

| 维度 | nanobot | NEXUS |
|------|---------|-------|
| 沙箱 | `restrict_to_workspace` 配置项 | `FsPolicy` enum：Sandbox / Whitelist / Unrestricted |
| 策略粒度 | 全局一个策略 | 每设备独立策略，REST API 可改，心跳热加载 |
| 路径校验 | `allowed_dir` 限制 | `sanitize_path_with_policy` + 符号链接防逃逸 |
| shell 安全 | 超时 + workspace 限制 | guardrails 检查 + SSRF DNS 解析 + 策略校验 + 超时 |
| 工具输出 | 16,000 字符截断 | 10,000 字符截断（头尾各 5,000） |

**M3 做了什么：**
- 实现了 per-device FsPolicy（Sandbox/Whitelist/Unrestricted）
- 通过心跳热加载策略变更
- 符号链接写入防逃逸：写操作 canonicalize parent directory
- shell 命令的 FsPolicy 执行检查
- 修复了 `working_dir` 参数被解析但未传递给执行的 bug
- 修复了 TOCTOU 漏洞（文件上传路径验证后用原始路径 I/O）

**差异分析：**
- NEXUS 的 per-device 策略更灵活：同一用户可以让家里的 NAS 全开、公司服务器只读
- nanobot 的全局策略更简单但不支持多设备差异化
- NEXUS 的安全模型更严格（SSRF 检查、符号链接防护），适合暴露在不可信网络中

---

## 7. Channel 集成（Discord）

| 维度 | nanobot | NEXUS |
|------|---------|-------|
| 方式 | discord.py SDK（高级封装） | 自研 Discord Gateway WebSocket 协议 |
| 代码量 | ~200 行（discord.py） | ~1100 行（4 个文件：mod/gateway_conn/protocol/rest） |
| 功能 | 完整：slash commands, attachments, streaming delta | 基础：收发消息、分片、typing indicator |
| 多用户 | 每个 channel 的 `allow_from` 列表 | 每个用户独立 bot token + `allowed_users` |
| 流式 | 支持（`send_delta` + coalesce） | 不支持 |

**M3 做了什么：**
- 从零实现了 Discord Gateway 协议（IDENTIFY, HEARTBEAT, RESUME）
- 实现了 REST API helpers（发送消息、分片、typing indicator）
- 多 bot 支持：每个用户可配置独立 Discord bot

**差异分析：**
- nanobot 用成熟的 discord.py SDK 是明智的——Discord 协议复杂、变化频繁
- NEXUS 自研是因为 Rust 生态中没有同等质量的 Discord SDK
- 自研的代价：代码量大、维护负担重、缺少 slash commands 等高级功能
- **建议：** 考虑用 `serenity` 或 `twilight` 等 Rust Discord crate 替代自研

---

## 8. 认证与多用户

| 维度 | nanobot | NEXUS |
|------|---------|-------|
| 用户模型 | 无用户系统，靠 channel allowlist 控制访问 | PostgreSQL 用户表，bcrypt 密码，JWT token |
| 设备管理 | 无概念 | device_tokens 表，每设备独立 token |
| API 认证 | 无管理 API | JWT 保护的 REST API |
| 多用户 | 单实例单用户（不同 channel 可限不同人） | 设计为多用户，但 M3 阶段实际单用户使用 |

**M3 做了什么：**
- 实现了完整的用户注册/登录/JWT 认证流程
- 设备 token 管理（创建/列出/撤销）
- 409 Conflict 防止重复设备名
- 管理员 API：LLM 配置、Embedding 配置、默认 soul

**差异分析：**
- NEXUS 的认证系统对远程设备管理是必须的——你不能让任何人连上你的 Server
- nanobot 不需要是因为它跑在本地
- NEXUS 当前的单用户模式意味着认证系统的复杂度暂时没有完全发挥价值

---

## 9. 配置管理

| 维度 | nanobot | NEXUS |
|------|---------|-------|
| 格式 | `~/.nanobot/config.json`（Pydantic schema） | `.env` 文件 + PostgreSQL `system_config` 表 |
| 热更新 | 需重启 | REST API 修改 → 心跳下发（FsPolicy, MCP） |
| LLM 配置 | config.json 中的 `agents.defaults` | REST API `PUT /api/llm-config` |
| 嵌入配置 | 无（不用 embedding） | REST API `PUT /api/embedding-config` |

**差异分析：**
- NEXUS 的 REST API 配置适合远程管理场景
- nanobot 的 config.json 更简单、可版本控制
- NEXUS 的心跳热加载是一个好设计——无需重启 Client 就能改策略

---

## 10. 总结：哪些是聪明的决定？

### 做对了的

1. **Server/Client 分离** — 对远程多设备场景是正确架构
2. **FsPolicy per-device + 心跳热加载** — nanobot 不支持这种灵活度
3. **RAG 记忆 + 显式去重** — 比 nanobot 的纯文本方案更强大
4. **MCP 配置 REST API** — 远程配置设备的 MCP Server，比环境变量更实用
5. **axum 0.8 升级** — 保持依赖更新，`{param}` 路由语法更清晰
6. **OpenAI 兼容协议修复** — 三个独立 bug（合并 tool_calls、loop detection、DB 重建）一起修掉

### 需要改进的

1. **Agent loop 缺少最大迭代限制** — 应使用 `MAX_AGENT_ITERATIONS` 常量
2. **MCP 心跳不触发重连** — `MCP_INITIALIZED` one-shot 导致新增 MCP Server 需要重启
3. **按条数截断而非 token** — 可能发送超出 context_window 的请求
4. **无 streaming 支持** — 长时间等待 LLM 响应时用户体验差
5. **Discord 自研协议** — 维护成本高，考虑用 Rust Discord crate
6. **`truncate_and_fix_orphans` 丢弃合法消息** — 应对齐到 user turn 而非简单跳过

---

*生成时间：2026-04-04*
*分支：m3-completion*
*对比基准：nanobot main 分支*
