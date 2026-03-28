# NEXUS Client 实现计划

基于 nanobot 参考实现，完整规划 nexus-client 各模块的工程实现。

**最后更新**: 2026-03-28

---

## 架构决策确认

| 决策项 | 结论 |
|--------|------|
| Skill 目录 | `workspace/skills/{name}/SKILL.md`（与 nanobot 一致） |
| Skill 性质 | SOP 文档，**不是工具**。Agent 用 `read_file` 读取正文 |
| read_skill 工具 | **不需要** |
| always=true skill | 全文注入 system prompt（**Server 侧决定**） |
| always=false skill | system prompt 只有 name + description |
| Skill 注册方式 | **不通过** RegisterTools。skill name+desc 通过单独字段发给 Server |
| skills_hash | skill name + description 列表的哈希，变化时触发 RegisterTools 更新 Server |
| MCP SDK | **mcp crate**（Rust 官方 MCP SDK，FastMCP 无 Rust 版） |
| 初期范围 | 仅实现 `shell` 内置工具，其他内置工具后续迭代 |
| restrict_to_workspace | 初期实现（env.rs） |
| 超时/截断/退出码 | 引用 `nexus-common/src/consts.rs`，禁止硬编码 |

---

## 总体架构

```
main.rs (入口)
    │
    ├── session.rs (WebSocket 连接 + 心跳 + 热加载检测)
    │       │
    │       └── discovery.rs (统一发现: 内置工具 + MCP 工具 + Skills)
    │               ├── discover_tools() → tools schemas
    │               ├── discover_skill_summaries() → skill name + description
    │               └── skills_hash + tools_hash 变化检测
    │
    └── 主循环 (recv ExecuteToolRequest)
            │
            └── executor.rs (工具路由器)
                    ├── guardrails.rs (安全校验)
                    └── tools/shell.rs (原生 shell 执行)
```

---

## 核心概念澄清

### Skill vs Tool

```
Tool (工具)
  - 通过 RegisterTools 注册
  - Server 下发 ExecuteToolRequest 执行
  - 例如: shell, mcp_filesystem_listDir

Skill (技能 SOP)
  - 不注册为工具
  - Skill name + description → RegisterTools.skill_summaries → Server 存 AppState
  - Agent 需要时，用 read_file 工具读 workspace/skills/{name}/SKILL.md
  - always=true 的 skill 全文由 Server 注入了 system prompt (非 client 职责)
```

### RegisterTools 消息扩展

```rust
ClientToServer::RegisterTools {
    device_id: ...,
    device_name: ...,
    schemas: Vec<ToolSchema>,           // 内置工具 + MCP 工具
    skill_summaries: Vec<SkillSummary>, // skill name + description
}
```

### Hash 统一检测

```
每次心跳 tick:
  ├─ tools_hash = hash(tools schemas)
  └─ skills_hash = hash(skill name + description)

  任一变化 → RegisterTools { schemas, skill_summaries }
```

---

## 模块 1: tools/mod.rs — LocalTool Trait

### nanobot 参考

`nanobot/nanobot/agent/tools/base.py` — `Tool` trait

### 设计

```rust
#[async_trait]
pub trait LocalTool: Send + Sync {
    fn name(&self) -> &'static str;
    fn schema(&self) -> Value;
    async fn execute(&self, args: Value) -> Result<String, ToolError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("guardrail blocked: {0}")]
    Blocked(String),
    #[error("timeout after {0}s")]
    Timeout(u64),
    #[error("tool not found: {0}")]
    NotFound(String),
    #[error("invalid params: {0}")]
    InvalidParams(String),
    #[error("execution failed: {0}")]
    ExecutionFailed(String),
}

/// exit_code 映射 (来自 nexus-common):
///   ToolError::Timeout         → EXIT_CODE_TIMEOUT (-1)
///   ToolError::Blocked         → EXIT_CODE_CANCELLED (-2)
///   其他                         → EXIT_CODE_ERROR (1)
```

### 依赖

- `async-trait = "0.1"`
- `thiserror = "2"`

---

## 模块 2: tools/shell.rs — Shell 执行器

### nanobot 参考

`nanobot/nanobot/agent/tools/shell.py` — `ExecTool.execute()`

### 设计

```
1. 超时控制: tokio::time::timeout
   - 默认: HEARTBEAT_INTERVAL_SEC * 4 = 60s (nexus-common)
   - 最大: 600s
2. 拒绝模式 (deny_patterns):
   r"\brm\s+-[rf]{1,2}\b"      // rm -rf, rm -r
   r"\bdel\s+/[fq]\b"           // Windows: del /f, del /q
   r"\bdd\s+if=\b"              // dd
   r":\(\)\s*\{.*\};"           // fork bomb
   r"\b(shutdown|reboot|poweroff)\b"
   r">\s*/dev/sd"               // 直接写盘
3. 允许模式 (allow_patterns, 可选)
4. restrict_to_workspace: 路径遍历检测 + 绝对路径必须在工作区内
5. 输出截断 (来自 nexus-common):
   MAX_TOOL_OUTPUT_CHARS = 10_000
   前 TOOL_OUTPUT_HEAD_CHARS + 后 TOOL_OUTPUT_TAIL_CHARS
6. PATH 追加 (path_append)
7. Windows: cmd.exe /c, POSIX: sh -c
8. Stderr 非空时附加: "STDERR:\n{stderr}"
```

---

## 模块 3: guardrails.rs — 安全校验

### nanobot 参考

- `nanobot/nanobot/agent/tools/shell.py` — `ExecTool._guard_command()`
- `nanobot/nanobot/security/network.py` — `contains_internal_url()`

### 设计

```rust
/// 检查 shell 命令，返回 Ok(()) 或 Err("blocked reason")
pub fn check_shell_command(cmd: &str) -> Result<(), String>

/// 命令中是否包含指向内部地址的 URL
pub fn contains_internal_url(command: &str) -> bool

/// 验证单个 URL 的目标地址是否安全
pub fn validate_url_target(url: &str) -> Result<(), String>
```

### 阻塞内部网段

```
0.0.0.0/8, 10.0.0.0/8, 100.64.0.0/10, 127.0.0.0/8, 169.254.0.0/16,
172.16.0.0/12, 192.168.0.0/16, ::1/128, fc00::/7, fe80::/10
```

---

## 模块 4: mcp_client.rs — MCP 客户端

### nanobot 参考

`nanobot/nanobot/agent/tools/mcp.py` — `connect_mcp_servers()`, `MCPToolWrapper`

### SDK

**使用 `mcp` crate**（Rust 官方 MCP SDK）

### 设计

```rust
// McpToolWrapper 实现 LocalTool trait
// 工具名: mcp_{server_name}_{original_tool_name}

// 支持传输类型:
enum TransportType {
    Stdio { command, args, env },
    Sse { url, headers },
    StreamableHttp { url, headers },
}

// Schema 标准化: nullable union 处理
// result.content 解析: TextContent → text, 其他 → str(block)
```

### config.rs McpServerConfig 需补充字段

当前缺少: `type`, `url`, `headers`, `tool_timeout`

---

## 模块 5: skills.rs — Skill 加载（整合到 discovery.rs）

### nanobot 参考

`nanobot/nanobot/agent/skills.py` — `SkillsLoader`

### 设计

```
目录结构: workspace/skills/{name}/SKILL.md

SKILL.md frontmatter:
---
name: xxx
description: xxx
always: false
requires:
  bins: [git]
  env: [GITHUB_TOKEN]
---

skills.rs 提供:
1. scan_skills(skills_dir) → Vec<SkillSummary> { name, description, always }
   - 热加载: mtime 缓存，变化重新解析
   - 供 discovery.rs 调用
2. read_skill(name) → String (SKILL.md 原文，供 Agent 自行读取)
   - Agent 用 read_file 工具读 workspace/skills/{name}/SKILL.md
   - 此函数用于 check_requirements 等内部校验
3. check_requirements(name) → bool (bins/env 检查)
```

**Skill 不通过 `RegisterTools.schemas` 注册为工具。**

---

## 模块 6: env.rs — 环境隔离

### nanobot 参考

`nanobot/nanobot/config/paths.py`, `nanobot/nanobot/agent/tools/shell.py`

### 设计

```rust
pub fn get_workspace_root() -> PathBuf   // NEXUS_WORKSPACE env > ~/.nexus/workspace

pub fn sanitize_path(path: &str, restrict: bool) -> Result<PathBuf, String>
  // restrict=true: 检查路径是否在工作区内

pub fn min_env() -> HashMap<String, String>
  // 仅保留 PATH, HOME, USER, TMP/TEMP
```

---

## 模块 7: discovery.rs — 统一发现（补全 + 整合 skills）

### 现状

只有 `discover_builtin_tools()`，MCP 调用被注释。skills.rs 独立在外。

### 补全设计（方案 B）

```rust
/// 返回: (tools schemas, skill summaries, tools hash, skills hash)
pub async fn discover_all(config: &ClientConfig, skills_dir: &Path)
    -> (Vec<ToolSchema>, Vec<SkillSummary>, String, String)
{
    // Tools
    let mut tools = discover_builtin_tools();   // shell
    if let Some(mcp_tools) = mcp_client::discover_mcp_tools(&config.mcp_servers).await {
        tools.extend(mcp_tools);
    }

    // Skills
    let skill_summaries = skills::scan_skills(skills_dir);

    let tools_hash = compute_hash(&tools);
    let skills_hash = compute_hash(&skill_summaries);

    (tools, skill_summaries, tools_hash, skills_hash)
}

/// 工具 Schema 发现
pub async fn discover_tools(config: &ClientConfig) -> Vec<ToolSchema>

/// Skill 发现（整合自 skills.rs）
pub fn scan_skills(skills_dir: &Path) -> Vec<SkillSummary>
```

### 与 session.rs 的交互

```
session.rs 每次心跳:
  (tools, skill_summaries, tools_hash, skills_hash) = discover_all(config, skills_dir).await

  if tools_hash != last_tools_hash OR skills_hash != last_skills_hash:
       RegisterTools { schemas: tools, skill_summaries }
       last_tools_hash = tools_hash
       last_skills_hash = skills_hash
```

---

## 模块 8: executor.rs — 工具路由器

### nanobot 参考

`nanobot/nanobot/agent/tools/registry.py` — `ToolRegistry.execute()`

### 设计

```rust
// 初始化时注册所有内置工具 + MCP 工具
TOOL_REGISTRY: LazyLock<RwLock<ToolRegistry>>

pub async fn execute_tool_request(req: ExecuteToolRequest) -> ToolExecutionResult
```

### 路由逻辑

```
ExecuteToolRequest { tool_name, arguments }
    │
    ├─ tool_name.starts_with("mcp_") → mcp_client::call_mcp_tool()
    └─ tool_name == "shell" → guardrails::check_shell_command() → tools::shell::execute()
```

---

## 模块 9: main.rs — 主循环（补全）

### 补全设计

```rust
while let Some(message) = session.recv().await {
    match message {
        ServerToClient::ExecuteToolRequest(req) => {
            let result = executor::execute_tool_request(req).await;
            session.send(ClientToServer::ToolExecutionResult(result)).await;
        }
        ServerToClient::Ping => {
            session.send(ClientToServer::Pong).await;
        }
        other => warn!("unhandled: {:?}", other),
    }
}
```

---

## 实现顺序

```
Phase 1: 基础设施
  1. tools/mod.rs       (LocalTool trait + ToolError)
  2. env.rs             (路径 + 环境变量隔离)

Phase 2: shell 执行能力
  3. tools/shell.rs     (核心执行，被 guardrails 保护)

Phase 3: 安全层
  4. guardrails.rs      (安全校验，被 shell.rs 调用)

Phase 4: 工具发现 (方案 B: 统一发现)
  5. mcp_client.rs      (MCP 连接，discovery 调用)
  6. skills.rs          (Skill 扫描，供 discovery 调用)
  7. discovery.rs       (补全 MCP 调用 + 整合 skills)

Phase 5: 执行路由
  8. executor.rs        (路由器)
  9. main.rs            (消息循环)

Phase 6: 集成测试
  10. Mock server 验证全流程
```

---

## 依赖清单 (Cargo.toml additions)

```toml
[dependencies]
nexus-common = { path = "../nexus-common" }  # 共享常量、协议类型
async-trait = "0.1"
thiserror = "2"
tokio = { version = "1", features = ["process", "time", "rt-multi-thread", "macros", "sync"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
regex = "1"
mcp = "0.1"              # Rust 官方 MCP SDK
```

**注意**: 超时、截断、退出码等常量必须从 `nexus-common::consts` 读取，禁止硬编码魔法数字。

---

## 测试策略

| 模块 | 测试内容 |
|------|----------|
| tools/shell.rs | 正常执行、拒绝危险命令、超时、输出截断、路径遍历拦截 |
| guardrails.rs | 内部 URL 检测、路径遍历检测 |
| mcp_client.rs | MCP 服务器连接、工具列表获取、工具调用、错误处理 |
| skills.rs | SKILL.md 解析、mtime 热加载、requirements 检查、skills_hash |
| discovery.rs | 工具聚合正确性 |
| executor.rs | 路由正确性、参数校验、错误传播 |
| main.rs | 端到端消息循环 (mock server) |
