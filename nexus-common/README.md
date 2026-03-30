# nexus-common

nexus-common 是 NEXUS 的协议与常量基座。它定义了 Server 与 Client 之间的"共享语言"，是整个系统唯一的跨端契约来源。**该 crate 已冻结，不接受功能性修改。**

---

## 一、模块结构

```
nexus-common/src/
├── lib.rs        — 模块导出
├── protocol.rs   — WebSocket 消息结构（核心）
├── consts.rs     — 跨端共享常量
└── error.rs      — 标准错误载荷与错误码枚举
```

---

## 二、protocol.rs — 消息协议

### 设计思路

Server 与 Client 之间只有一条 WebSocket 连接（路径 `/ws`）。所有消息都是 JSON 文本帧，用 `serde_json` 序列化/反序列化。

消息方向分两类：

- `ServerToClient`：Server 主动下发给 Client 的指令
- `ClientToServer`：Client 上报给 Server 的事件

两者都用 `#[serde(tag = "type", content = "data")]` 做枚举序列化，序列化后的 JSON 长这样：

```json
{ "type": "RequireLogin", "data": { "message": "Please authenticate" } }
{ "type": "SubmitToken",  "data": { "token": "nexus_dev_...", "device_id": "my-mac", "device_name": "my-mac", "protocol_version": "1.0" } }
```

### ServerToClient 枚举

| 变体 | 触发时机 | 关键字段 |
|------|----------|----------|
| `RequireLogin` | Client 建立 WebSocket 连接后立即发送 | `message: String`（提示文本） |
| `LoginSuccess` | Token 验证通过，设备注册成功 | `user_id`, `device_id` |
| `LoginFailed` | Token 无效或已吊销 | `reason: String` |
| `ExecuteToolRequest` | Agent Loop 需要 Client 执行工具 | 见下方 `ExecuteToolRequest` 结构体 |

### ClientToServer 枚举

| 变体 | 触发时机 | 关键字段 |
|------|----------|----------|
| `SubmitToken` | 收到 `RequireLogin` 后 | `token`, `device_id`, `device_name`, `protocol_version` |
| `RegisterTools` | 登录成功后、工具集变更后 | `device_id`, `schemas: Vec<Value>` |
| `Heartbeat` | 每 15 秒一次 | `device_id`, `tools_hash`, `status` |
| `ToolExecutionResult` | 工具执行完毕 | 见下方 `ToolExecutionResult` 结构体 |
| `ToolStdoutStream` | 工具执行中的流式输出 | `request_id`, `chunk_data` |

**`SubmitToken.protocol_version`**：Client 声明自己遵循的协议版本，Server 在握手阶段校验是否等于 `consts::PROTOCOL_VERSION`（当前为 `"1.0"`）。不匹配时发送 `LoginFailed { reason: "Protocol version mismatch" }` 并断开连接。

**`RegisterTools.device_id` / `Heartbeat.device_id`**：握手完成后 Server 已知该连接对应的 device_id，此处重复携带作为 sanity check。Server 应校验是否与握手时注册的 device_id 一致，不一致则断开连接。

**`Heartbeat.status` 合法值**：`"online"`（空闲，可接受工具调用）或 `"busy"`（正在执行工具）。M1 阶段 Client 只发送 `"online"`，`"busy"` 留待 M3 executor 实现后启用。

### ExecuteToolRequest 结构体

```rust
pub struct ExecuteToolRequest {
    pub request_id: String,   // 格式："{device_id}:{uuid_v4()}"，用于挂起/唤醒匹配
    pub tool_name: String,    // 工具名称，例如 "shell"、"mcp_github_search"
    pub arguments: Value,     // 工具参数，JSON 对象
}
```

### ToolExecutionResult 结构体

```rust
pub struct ToolExecutionResult {
    pub request_id: String,
    pub exit_code: i32,   // 语义见下方"退出码约定"
    pub output: String,   // 执行结果或错误信息
}
```

### 退出码约定（exit_code）

| 值 | 含义 | 触发方 |
|----|------|--------|
| `0` | 执行成功 | Client executor |
| `1` | 执行失败（stderr 或业务错误） | Client executor |
| `-1` | 执行超时（被 timeout kill） | Client executor |
| `-2` | 被取消（设备断线，Server drop 了 Sender） | Server ws.rs |
| `-3` | 参数校验失败（guardrails 拦截，未执行） | Client executor |

这套退出码让 Server 侧的 Agent Loop 可以对不同失败原因做差异化处理，例如 `-2` 说明设备断线可以重试，`-3` 说明参数有误应让 LLM 自我纠正。

### ToolStdoutStream 结构体

```rust
pub struct ToolStdoutStream {
    pub request_id: String,
    pub chunk_data: String,  // 工具执行中的实时输出片段
}
```

用于长时间运行的工具（如 shell 命令）在执行过程中实时回传 stdout，让 Server 可以流式转发给 WebUI 展示进度。

---

## 三、consts.rs — 共享常量

| 常量 | 值 | 用途 |
|------|----|------|
| `PROTOCOL_VERSION` | `"1.0"` | 握手时版本校验（未来用） |
| `HEARTBEAT_INTERVAL_SEC` | `15` | Client 心跳发送间隔（秒） |
| `DEFAULT_MCP_TOOL_TIMEOUT_SEC` | `30` | MCP 工具单次调用超时 |
| `MAX_AGENT_ITERATIONS` | `40` | Agent ReAct 循环上限 |
| `MAX_HISTORY_MESSAGES` | `500` | 单次 LLM 调用携带的最大历史条数 |
| `MAX_TOOL_OUTPUT_CHARS` | `10000` | 工具输出截断阈值 |
| `TOOL_OUTPUT_HEAD_CHARS` | `5000` | 截断后保留头部字符数 |
| `TOOL_OUTPUT_TAIL_CHARS` | `5000` | 截断后保留尾部字符数 |
| `EXIT_CODE_SUCCESS` | `0` | 同上方退出码约定 |
| `EXIT_CODE_ERROR` | `1` | 同上 |
| `EXIT_CODE_TIMEOUT` | `-1` | 同上 |
| `EXIT_CODE_CANCELLED` | `-2` | 同上 |
| `EXIT_CODE_VALIDATION_FAILED` | `-3` | 同上 |
| `DEVICE_TOKEN_PREFIX` | `"nexus_dev_"` | Device Token 格式前缀 |
| `DEVICE_TOKEN_RANDOM_LEN` | `32` | Token 随机部分长度 |

**工具输出截断策略**：超过 10000 字符时，保留前 5000 + 后 5000，中间插入 `"... (X chars truncated) ..."`。这样既保留命令头尾的关键上下文，又控制了传输体积。

---

## 四、error.rs — 错误载荷

### NexusErrorPayload

跨网络传输的标准错误结构，用于 Server 告知 Client 失败原因，或 Client 告知 Server 执行失败详情：

```rust
pub struct NexusErrorPayload {
    pub code: String,    // 错误码字符串，例如 "AUTH_FAILED"
    pub message: String, // 人类可读的错误详情
}
```

### NexusErrorCode 枚举

| 枚举变体 | 字符串值 | 含义 |
|----------|----------|------|
| `AuthFailed` | `"AUTH_FAILED"` | Token 无效或用户不存在 |
| `AuthTokenExpired` | `"AUTH_TOKEN_EXPIRED"` | Token 已过期（未来用） |
| `ExecutionTimeout` | `"EXECUTION_TIMEOUT"` | 工具执行超时 |
| `ExecutionCancelled` | `"EXECUTION_CANCELLED"` | 工具执行被取消 |
| `ValidationFailed` | `"VALIDATION_FAILED"` | 参数校验失败 |
| `DeviceNotFound` | `"DEVICE_NOT_FOUND"` | 目标设备不在线 |
| `ProtocolMismatch` | `"PROTOCOL_MISMATCH"` | 协议版本不兼容 |
| `InternalError` | `"INTERNAL_ERROR"` | 服务端内部错误 |

---

## 五、关键设计决策

### 决策 1：不使用 JWT，只用 Device Token

**是什么**：认证凭据格式固定为 `nexus_dev_` + 32 位随机字符，存在数据库 `device_tokens` 表中，可按需吊销。

**为什么**：
- JWT 是无状态的，一旦签发无法主动吊销，不适合需要精确控制单设备访问权限的场景。
- Device Token 存数据库，吊销只需把 `revoked` 字段置为 `true`，即时生效。
- 认证流程更简单：Client 提交 token，Server 查库，一步完成，没有签名验证的复杂性。

**影响范围**：`nexus-server/src/db.rs`（verify_device_token）、`nexus-server/src/ws.rs`（握手流程）、`nexus-client/src/session.rs`（提交 token）。

### 决策 2：device_id 由 Client 自报，Server 不校验归属

**是什么**：`SubmitToken` 消息里的 `device_id` 完全信任 Client 上报的值（通常是 hostname），Server 不在 `device_tokens` 表里存 `device_id` 字段做二次校验。

**为什么**：
- Device Token 生成时用户还不知道 Client 的 hostname（Client 还没启动），无法预先绑定。
- Token 本身就是认证凭据，持有 token 即有权接入，`device_id` 只是路由标识符，不是安全锁。
- 首次登录时 Server 会把 Client 上报的 `device_name` 回写到 `device_tokens` 表，供 WebUI 展示可读名称。

### 决策 3：工具 Schema 统一用 `serde_json::Value`，不定义强类型结构体

**是什么**：`RegisterTools` 的 `schemas: Vec<Value>` 和 `ExecuteToolRequest` 的 `arguments: Value` 都用动态 JSON，不引入额外的 `ToolSchema` 结构体。

**为什么**：内置工具、MCP 工具、Skill 工具三类来源的 Schema 格式各有差异，用强类型结构体需要为每类维护单独的反序列化逻辑，而动态 `Value` 可以透传任意格式，减少新增工具时的编译期改动成本。

### 决策 4：request_id 格式绑定设备归属

**是什么**：`ExecuteToolRequest.request_id` 格式为 `"{device_id}:{uuid_v4()}"`。

**为什么**：Server 侧的挂起等待表（`pending: HashMap<String, oneshot::Sender>`）需要在设备断线时清理该设备的所有挂起请求。用前缀格式可以通过 `starts_with("{device_id}:")` 高效过滤，无需在 value 里额外存 device_id 字段。

---

## 六、与其他模块的关系

| 模块 | 关系 |
|------|------|
| `nexus-server` | 依赖 common，用 `ServerToClient` 下发指令，收 `ClientToServer` 事件 |
| `nexus-client` | 依赖 common，收 `ServerToClient` 指令，发 `ClientToServer` 事件 |
| `nexus-gateway` | 不直接依赖 common（gateway 与 server 之间协议独立定义） |

---

## 七、环境要求

- Rust 1.85+，edition 2024
- 依赖：`serde`（derive feature）、`serde_json`
- 无网络、无数据库、无异步运行时依赖
- 作为 workspace 内部库使用，无独立进程