# M1 Client 收尾计划

**最后更新**: 2026-03-28
**状态**: ✅ P1 全部完成 — Client 可进入 freeze

---

## 已完成 ✅

| 功能 | 文件 | 备注 |
|------|------|------|
| LocalTool trait | `tools/mod.rs` | `LocalTool` trait + `ToolError` |
| Shell 执行 | `tools/shell.rs` | 超时、截断、stderr 附加 |
| 安全校验 | `guardrails.rs` | 正则预编译 + SSRF DNS 解析 |
| MCP 客户端 (Stdio) | `mcp_client.rs` | JSON-RPC 握手、工具发现、调用 |
| MCP Schema 规范化 | `mcp_client.rs` | nullable/oneOf/anyOf 规范化 |
| Skill 加载 | `skills.rs` | frontmatter 解析、scan、requirements |
| 统一发现 | `discovery.rs` | 内置工具 + MCP + Skills 聚合 + hash |
| 会话管理 | `session.rs` | WebSocket 连接、重连，心跳、工具注册 |
| 工具路由 | `executor.rs` | shell/MCP 路由分发 |
| 配置加载 | `config.rs` | env var 解析 |
| 协议定义 | `nexus-common/protocol.rs` | ClientToServer / ServerToClient 消息 |
| 主循环 | `main.rs` | 连接 → 注册 → 主循环 |
| 文件系统工具 | `tools/fs.rs` | read_file / write_file / list_dir / stat + workspace 限制 |

---

## P1 — M1 完成

### 1. 文件系统工具 (`tools/fs.rs`)

**nanobot 参考**: `nanobot/agent/tools/fs.py`

**原因**: Agent 没有读文件能力，Skill 系统无法闭环（`read_skill_content()` 需要 Agent 自己读 SKILL.md）。

**需要实现的工具**:

| 工具名 | 功能 | nanobot 对应 |
|--------|------|-------------|
| `read_file` | 读文件内容（文本 or 图片） | `ReadFileTool` |
| `write_file` | 写文件 | `WriteFileTool` |
| `list_dir` | 列出目录 | `ListDirTool` |
| `stat` | 文件元数据 | 无（新增） |

**read_file 返回格式**（参考 nanobot `build_image_content_blocks`）:

```rust
// 文本文件 → 直接返回带行号的内容
ToolExecutionResult {
    output: "1| fn main() { ... }\n...\n(End of file — 150 lines total)".to_string(),
    exit_code: 0,
}

// 图片文件 → 返回结构化 content block，告知 Server "这是一个图片"
// Server 的 Agent 自己决定如何处理（发送/展示/忽略）
ToolExecutionResult {
    output: "[Image: screenshot.png, 240KB]".to_string(),
    exit_code: 0,
    // 注意：不传 bytes，Server 根据需要自行决定下载方式
}
```

**关键约束**:
- `restrict_to_workspace`: 所有路径必须在 workspace 内
- `read_file` 不做 FileAttachment 传输 — 返回文本内容或图片元信息即可
- 大文件支持行号分页（offset/limit），参考 nanobot `_MAX_CHARS = 128_000`

---

## P2 — M1 freeze 后作为后续里程碑

### 2. MCP HTTP 传输 (`Sse` / `StreamableHttp`)

**当前状态**: `config.rs` 已定义 `url`/`headers` 字段，`mcp_client.rs` 中 `initialize()` 时跳过非 Stdio 类型。

**原因**: HTTP MCP Server 是常见场景（Claude Desktop、Cursor 等），但实现依赖 `mcp` crate 的 HTTP 客户端，复杂度高。

**需要**: `mcp` crate 已经有 SSE/Streamable HTTP 支持。

---

### 3. 工具执行 Hook 系统

**原因**: `protocol.rs` 已定义 `ToolStdoutStream`，但当前 executor 无流式支持。若需要实时流式输出，需要 hook 机制。

**nanobot 参考**: `before_execute_tools`, `on_stream`, `on_stream_end` hooks in `AgentLoop`。

---

### 4. Frontmatter YAML 解析完善

**当前状态**: `skills.rs` 使用启发式行解析，`requires.bins` / `requires.env` 判断不完整。

**可选改进**: 引入 `serde_yaml` 完整解析 YAML frontmatter，但当前实现能用，不是 blocker。

---

## 文件发送架构澄清（重要）

### nanobot vs NEXUS 的本质差异

```
nanobot（单进程）:
  Agent + Tools + Channels 全部在同一进程
  → message 工具直接 callback → Bus → Channel Adapter
  → OutboundMessage.media = ["/path/to/file"]  ← 文件路径，channel adapter 自己读

NEXUS（分布式）:
  Client (Tools) ←→ Server (Agent + Channels)
  → Client 没有 message 工具
  → 文件发送是 Server 的主动行为，不走 ToolExecutionResult 协议
```

### NEXUS 文件发送的正确理解

**文件发送不是工具调用**。当 Agent 需要发送文件给用户时：

```
Agent: "send the chart.png to user"
    ↓
Server (agent_loop):
    │  recognizes intent → 决定发送文件
    │  确定用户所在 channel: discord:123
    ↓
DiscordAdapter.send_file(path="/tmp/chart.png")
    → Discord REST API multipart upload
```

**Client 端 `read_file` 的作用**：
- 让 Agent 能够**分析**文件内容（读 SKILL.md、读配置文件等）
- Agent 看完之后自己决定：要把这个发给用户吗？ → 发给 Server → Server 走 channel adapter

**结论**：
- `read_file` 返回文本内容 or 图片 content block 即可
- **不需要** FileAttachment 结构通过 `ToolExecutionResult` 传输
- 文件发送是 **Server 端职责**，由 Agent 主动决策，不走工具协议

---

## Client Freeze 前检查清单

```
P1:
  ✅ tools/fs.rs — read_file, write_file, list_dir, stat (2026-03-28)

P2 (freeze 后):
  ☐ MCP Sse/StreamableHttp 传输
  ☐ Tool Hook 系统
  ☐ Frontmatter YAML 解析
```

---

## 架构决策记录

| 决策 | 结论 |
|------|------|
| `read_file` 返回格式 | 文本直接返回；图片返回 `[Image: filename, size]` 形式的元信息 |
| 文件发送方式 | Server Agent 主动决策，走 channel adapter，不通过工具协议 |
| FileAttachment 传输 | M1 不需要，后续视情况而定 |

---

*本文件记录 M1 client freeze 前剩余工作，P2 项属于后续里程碑范围。*
