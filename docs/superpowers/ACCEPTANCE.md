# NEXUS 验收标准

> 最后更新：2026-03-30

---

## ✅ M0 — 文档基建（已验收）

四个模块 README 均已落地，包含定位、边界、架构决策、模块关系、环境要求五个章节，认证机制统一为 Device Token。

---

## ✅ M1 — 握手能跑通（已验收）

1. 启动 Server：`cargo run --package nexus-server`，日志显示监听端口。
2. 启动 Client：`cargo run --package nexus-client`，终端出现 `LoginSuccess`。
3. 无效 token 启动 Client，收到 `LoginFailed` 后断开。
4. 超过一个心跳周期设备仍在线，断心跳后按超时策略剔除。

---

## ✅ M2 — 工具注册能跑通（已验收）

1. Client 上线后 Server 日志出现工具注册事件，含 `device_id` 与工具数量。
2. Server 内存可查询对应设备的 `tools` 字段非空。
3. 修改本地工具集后，心跳 hash 变化触发重注册，Server 工具列表更新。

---

## 🔄 M3 — Agent Loop 跑通（验收条件）

1. 触发一次会话请求（通过 Channel 发消息或直接注入）：

   日志顺序：`LLM 调用 → 工具下发 (ToolExecutionRequest) → 工具结果 (ToolExecutionResult) → 最终回复`。

2. 多轮对话：第二轮 LLM 请求的 messages 数组包含前一轮的 user/assistant/tool 消息。

3. 断线测试：强制断开 nexus-client，挂起中的工具请求立即返回失败，agent_loop 继续处理下一条消息。

4. 参数错误：向 Client 发送无效工具参数，返回退出码 `-3` 含校验失败信息。

---

## 🔄 M4 — WebUI 能用（验收条件）

1. 启动 gateway：

   ```bash
   cargo run --package nexus-gateway
   ```

   浏览器访问 `http://localhost:9090`，可见聊天页面。

2. 认证：输入有效凭据后建立 WebSocket 连接；无效凭据收到错误提示。

3. 聊天：发送一条会触发工具调用的消息，页面实时显示工具执行过程与最终 AI 回复。

4. 连接保持：关闭并重开浏览器 tab，重新连接后会话历史可见。

---

## ⬜ M5 — 完善与扩展（验收条件）

1. 记忆：

   ```sql
   SELECT count(*) FROM memory_chunks;
   ```

   多轮对话后计数大于 0；后续相关提问的 Agent 回复体现已召回记忆。

2. MCP 工具：挂载 MCP Server 后，`mcp_` 前缀工具出现在设备工具列表并可调用。

3. Skills：`workspace/skills/` 新增 Skill 后重连 Client，`skill_` 工具出现在列表并可执行。

4. Discord/Telegram：对应渠道发消息触发 Agent 回复，日志可见完整 ReAct 链路。

5. 管理后台：管理员可查看设备列表、用户列表，普通用户访问管理页被拒绝。
