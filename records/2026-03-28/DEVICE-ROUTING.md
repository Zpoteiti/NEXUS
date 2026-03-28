# NEXUS 设备路由设计文档

> 本文档定义 NEXUS 多设备场景下的工具路由机制。
>
> **参考实现**：OpenClaw 分布式架构（Gateway 中心化 + Node 外设模式）

---

## 1. 背景与问题

### 1.1 场景

一个用户（user_id）可以拥有多台设备（device）。每台设备注册自己的工具集（shell、MCP、Skills 等）。当 Agent 需要执行工具时，必须明确路由到哪台设备。

### 1.2 OpenClaw 参考架构

```
┌─────────────────────────────────────────────────────────────┐
│                         Gateway                              │
│                   (127.0.0.1:18789)                         │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  · 掌握所有 session、认证、channel、状态              │   │
│  │  · 接收外部消息（Telegram/Discord/...）              │   │
│  │  · 路由工具调用到指定 Node                           │   │
│  │  · WebSocket 统一入口（operator + node 共用端口）    │   │
│  └─────────────────────────────────────────────────────┘   │
│                            ↑                                │
│           ┌─────────────────┼─────────────────┐            │
│           │                 │                 │            │
│    ┌──────┴──────┐   ┌──────┴──────┐   ┌──────┴──────┐    │
│    │   iOS      │   │  macOS      │   │  Android    │    │
│    │   Node     │   │  Node       │   │  Node       │    │
│    │            │   │  (menubar)  │   │             │    │
│    └────────────┘   └─────────────┘   └─────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

**关键设计**：

- **Gateway 是唯一中心**，掌握所有 session、认证、状态
- **Node 是外设**，不运行 gateway 服务，通过 WebSocket 连接
- **设备配对机制**：Node 连接 → Gateway 生成 pairing request → 用户审批
- **工具路由**：通过配置 `tools.exec.host=node` + `tools.exec.node="<id>"` 转发

NEXUS 的 `nexus-server` ↔ `nexus-client` 架构与 OpenClaw 完全一致。

### 1.3 现有协议支持

NEXUS 的 `ExecuteToolRequest` 已具备路由字段：

```rust
pub struct ExecuteToolRequest {
    pub request_id: String,
    pub tool: String,
    pub params: Value,
    pub device_id: Option<DeviceId>,    // ← 目标设备 ID
    pub device_name: Option<String>,
}
```

**核心问题**：

1. LLM 无法正确填写 `device_id`（32位 hash，人类不可读）
2. Server 端缺少 `device_name` → `device_id` 的高效查找路径
3. Agent 上下文缺少设备列表，LLM 无从判断工具该路由到哪台设备

---

## 2. 设计原则

1. **user_id 对 LLM 不可见** — 系统自动从会话中获取，LLM 只能指定 device_name
2. **device_name 客户端配置** — `device_name` 在 Client 配置文件中设置，不是自动生成的
3. **device_name 人类可读** — LLM 可以正确指定 "在 mac-mini 上执行"
4. **O(1) 查找** — 通过索引直接映射 user_id + device_name → device_id
5. **按用户隔离** — device_name 在同一用户下唯一，不同用户可以重名
6. **Schema 自描述** — Server 修饰 schema 注入 `device_name` enum，LLM 原生能力驱动路由

---

## 3. 路由方案：Schema 自描述路由

### 3.1 核心思想

利用 LLM 的原生工具调用能力：

1. **Server 注入 `device_name` 参数**：Client 上报的原生 schema（尤其是 MCP 工具）不包含 `device_name`。Server 在构建 LLM 请求时，**修饰**每个工具的 schema，注入 `device_name` enum 参数
2. **设备状态体现在 enum 中**：在线设备正常显示，离线设备显示 `status=offline`
3. **设备注销后从 schema 消失**：下次 LLM 调用时 schema 已不包含该设备
4. **路由失败 → 自然重试**：LLM 调用不存在/离线设备 → 报错 → LLM 重新选择设备

> **关键理解**：`device_name` 是 NEXUS 路由机制的一部分，**不是工具本身的参数**。外部 MCP server 不知道也不关心多设备场景。

### 3.2 LLM 请求体设计

```json
{
  "model": "gpt-4o",
  "messages": [
    {
      "role": "system",
      "content": "You are a helpful assistant that can execute shell commands on remote devices.\n\nAvailable devices:\n- mac-mini      | status: online\n- ubuntu-server | status: online"
    },
    {
      "role": "user",
      "content": "Check the disk usage on the ubuntu server."
    }
  ],
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "run_shell_command",
        "description": "Run a shell command on a specified device.",
        "parameters": {
          "type": "object",
          "properties": {
            "device_name": {
              "type": "string",
              "enum": ["mac-mini", "ubuntu-server"],
              "description": "The device to run the command on."
            },
            "command": {
              "type": "string",
              "description": "The shell command to execute."
            }
          },
          "required": ["device_name", "command"]
        }
      }
    }
  ],
  "tool_choice": "auto"
}
```
【重要】在本例子中，client上报工具注册时的参数只包含 `command` ，不包含 `device_name`。device name 这个参数由服务器注入。

### 3.3 设备状态与 schema 联动

| 事件 | device_name enum 变化 | LLM 行为 |
|------|---------------------|---------|
| 设备注册 | 新增设备到 enum | LLM 知道可以使用该设备 |
| 设备离线 | enum 中该设备标记 `status=offline` | LLM 会避免选择，或选择后收到错误并重试 |
| 设备注销 | 从 enum 中移除 | LLM 不会再尝试调用该设备 |
| 设备重连 | 重新加入 enum | LLM 重新可以使用 |

### 3.4 路由决策流程

```
LLM 决定调用 "run_shell_command"
    │
    ▼
LLM 从 device_name enum 中选择一个设备（例如 ubuntu-server）
    │
    ▼
Server 解析 LLM 响应，提取 device_name
    │
    ▼
find_device_by_name(user_id, "ubuntu-server")
    │
    ├─► 找到 device_id → 验证设备在线
    │
    ▼
Server 剥离 ExecuteToolRequest 中的 device_name，
    保留 { request_id, tool, params }
    │
    ▼
路由到目标 Client 的 WebSocket 连接
    │
    ▼
Client 收到 ExecuteToolRequest { request_id, tool, params }
    Client 知道这是发给自己的（WebSocket 点对点）
    │
    ▼
Client executor 执行工具 → 返回 ToolExecutionResult
    │
    ▼
若设备不存在/离线 → 返回错误 "设备 'ubuntu-server' 不存在或已离线"
        LLM 收到错误 → 重新选择设备 → 重试
```

### 3.5 错误处理与重试

| 错误场景 | LLM 错误信息 | LLM 行为 |
|---------|------------|---------|
| 设备不存在 | "设备 '{device_name}' 不存在" | 重新选择设备 |
| 设备离线 | "设备 '{device_name}' 当前离线" | 重新选择在线设备 |
| 工具在设备上不存在 | "设备 '{device_name}' 上没有工具 '{tool_name}'" | 重新选择设备 |
| 发送失败 | "向设备 '{device_name}' 发送请求失败" | 重新选择设备 |

---

## 4. 协议变更

### 4.1 ExecuteToolRequest（Server → Client）

```rust
pub struct ExecuteToolRequest {
    pub request_id: String,
    pub tool: String,                  // 工具名
    pub params: Value,                 // 工具参数（已剥离 device_name）
}
```

> **说明**：`device_name` 仅在 Server 内部用于路由决策（`device_name` → `device_id` 查找），**不**传递给 Client。Server 解析 LLM 响应中的 `device_name`，查找目标设备后，将**剥离了 `device_name` 的原始工具调用**路由到对应 Client。Client 收到时已不包含 `device_name` 字段。

### 4.2 ClientToServer::RegisterTools（变更）

```rust
RegisterTools {
    device_id: DeviceId,
    device_name: String,         // Client 上报时提供 device_name
    schemas: Vec<ToolSchema>,   // 工具 Schema 数组
}
```

### 4.3 ClientToServer::Heartbeat（变更）

```rust
Heartbeat {
    device_id: DeviceId,
    device_name: String,         // 心跳时携带 device_name
    tools_hash: String,          // 工具列表 hash，用于检测变更
    status: DeviceStatus,       // online / busy / offline
}
```

### 4.4 Client 配置：device_name

`device_name` 是客户端配置的，不是自动生成的：

```toml
# nexus-client.conf
[device]
name = "macmini-office"    # 必填：人类可读的设备名称
```

**配置流程**：

```
1. 用户在 Client 配置文件中设置 device_name
2. Client 启动时读取配置，注册时上报给 Server
3. Server 存储 device_name，用于构建 LLM schema enum
4. 用户想改名？修改配置文件，重启 Client
```

**变更 device_name 的影响**：

- Server 维护 `devices_by_user[user_id][device_name] = device_id` 索引
- 改名后：旧 `device_name` 从索引中删除，新 `device_name` 添加
- **LLM 调用时使用新名称**：旧名称不再有效
- **潜在问题**：如果用户改了 device_name 但有正在 pending 的请求，需要考虑如何处理

**设计决策**：

| 选项 | 描述 | 优缺点 |
|------|------|--------|
| A. 配置文件 | 用户手动修改 `nexus-client.conf` | 简单，但改名需要重启 |
| B. Server API | `POST /devices/{id}/rename` | 动态改名，但需要额外实现 |

> 建议先采用 **选项 A（配置文件）**，保持简单。动态改名作为后续功能。

---

## 5. 数据结构变更

### 5.1 state.rs — AppState

```rust
pub struct AppState {
    // device_id → DeviceState
    pub devices: Arc<RwLock<HashMap<DeviceId, DeviceState>>>,

    // 新增：user_id → { device_name → device_id } 二级索引，O(1) 查找
    pub devices_by_user: Arc<RwLock<HashMap<UserId, HashMap<DeviceName, DeviceId>>>>,

    // 待完成的工具调用响应
    pub pending: Arc<RwLock<HashMap<RequestId, oneshot::Sender<ToolExecutionResult>>>>,
}
```

### 5.2 索引维护规则

| 操作 | 触发时机 | 维护内容 |
|------|---------|---------|
| 设备注册 | ws.rs 收到 RegisterTools | `devices_by_user[user_id][device_name] = device_id` |
| 设备注销 | 设备断线超时 | `devices_by_user[user_id].remove(&device_name)` |
| 设备重连 | 收到新的 RegisterTools | 更新索引（同一 device_name 可能映射到新 device_id） |

### 5.3 Schema 动态构建

每次 LLM 请求前，Server 修饰 Client 上报的 schema，注入 `device_name` 参数：

```rust
/// 修饰工具 schema，注入 device_name 参数
///
/// 原始 schema（MCP 工具等）：
/// {
///   “name”: “run_shell_command”,
///   “parameters”: {
///     “properties”: { “command”: { “type”: “string” } }
///   }
/// }
///
/// 修饰后 schema：
/// {
///   “name”: “run_shell_command”,
///   “parameters”: {
///     “properties”: {
///       “device_name”: { “enum”: [“mac-mini”, “ubuntu-server”] },  // ← 注入
///       “command”: { “type”: “string” }                            // ← 保留原有
///     },
///     “required”: [“device_name”, “command”]
///   }
/// }
pub async fn build_tools_schema(
    state: &AppState,
    user_id: &UserId,
    original_schemas: Vec<ToolSchema>,  // Client 上报的原始 schema
) -> Vec<Value> {
    // 1. 获取当前用户的设备列表
    let devices = state.devices_by_user.read().await;
    let user_devices = devices.get(user_id);

    let device_enum: Vec<String> = user_devices
        .map(|d| d.keys().cloned().collect())
        .unwrap_or_default();

    // 2. 遍历每个工具 schema，注入 device_name 参数
    original_schemas
        .into_iter()
        .map(|schema| {
            // 合并 properties：原有参数 + device_name
            let mut properties = schema.parameters.get(“properties”)
                .cloned()
                .unwrap_or_default();

            properties[“device_name”] = json!({
                “type”: “string”,
                “enum”: device_enum,
                “description”: “The device to run the command on.”
            });

            // 合并 required：原有 required + device_name
            let mut required = schema.parameters.get(“required”)
                .and_then(|r| r.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>())
                .unwrap_or_default();

            if !required.contains(&”device_name”.to_string()) {
                required.push(“device_name”.to_string());
            }

            json!({
                “type”: “function”,
                “function”: {
                    “name”: schema.name,
                    “description”: schema.description,
                    “parameters”: {
                        “type”: “object”,
                        “properties”: properties,
                        “required”: required
                    }
                }
            })
        })
        .collect()
}
```

**关键点**：
- Server **修饰**（mutation）Client 上报的 schema，**不修改原始数据**
- `device_name` 参数被添加到**每个工具**的 schema 中
- 原有工具参数（如 `command`）被保留

---

## 6. 查找函数

### 6.1 tools_registry.rs — find_device_by_name

```rust
/// 根据 user_id 和 device_name 查找 device_id
///
/// 返回值：
///   Some(device_id) — 找到
///   None            — 设备不存在或不属于该用户
pub async fn find_device_by_name(
    state: &AppState,
    user_id: &UserId,
    device_name: &str,
) -> Option<DeviceId> {
    let devices = state.devices_by_user.read().await;
    devices.get(user_id)?.get(device_name).cloned()
}
```

### 6.2 route_tool — 核心路由函数

```rust
/// 根据 device_name 路由工具调用
pub async fn route_tool(
    state: &AppState,
    session: &Session,
    req: ExecuteToolRequest,
) -> Result<ToolResponse> {
    // 1. 查找目标设备
    let device_id = find_device_by_name(state, &session.user_id, &req.device_name)
        .await
        .ok_or_else(|| RouteError::DeviceNotFound(req.device_name.clone()))?;

    // 2. 验证设备在线
    let device = state.devices.read().await;
    let device_state = device.get(&device_id)
        .ok_or_else(|| RouteError::DeviceOffline)?;

    if device_state.status == DeviceStatus::Offline {
        return Err(RouteError::DeviceOffline.into());
    }

    // 3. 发送请求到对应 client
    let ws_tx = &device_state.ws_tx;
    ws_tx.send(Message::ExecuteToolRequest(req))
        .map_err(|_| RouteError::SendFailed)?;

    // 4. 等待结果（通过 pending 表）
    Ok(/* 挂起等待响应 */)
}
```

---

## 7. Agent 上下文设计

### 7.1 上下文内容

system prompt 中仅列出可用设备（不带工具清单）：

```
You are a helpful assistant that can execute shell commands on remote devices.

Available devices:
- mac-mini      | status: online
- ubuntu-server | status: online
- gpu-server    | status: offline
- raspberry-pi  | status: offline
```

### 7.2 上下文生成时机

- 用户发起会话时构建
- 每次 Agent Loop 开始时刷新（设备状态可能变化）
- 设备状态变更时推送更新（WebSocket 推送）

### 7.3 状态说明

| 状态 | 含义 | LLM 行为 |
|------|------|---------|
| online | 设备在线，可执行工具 | ✅ 正常选择 |
| busy | 设备忙碌（正在执行其他工具） | ⚠️ 可选择，但可能等待 |
| offline | 设备离线 | ❌ 不应选择；若选择则报错并重试 |

---

## 8. 错误处理

| 错误场景 | LLM 错误信息 | LLM 行为 |
|---------|------------|---------|
| 设备不存在 | "设备 '{device_name}' 不存在或已离线" | 重新选择设备 |
| 设备离线 | "设备 '{device_name}' 当前离线，无法执行工具" | 重新选择在线设备 |
| 工具在设备上不存在 | "设备 '{device_name}' 上没有工具 '{tool_name}'" | 重新选择设备 |
| 发送失败 | "向设备 '{device_name}' 发送请求失败" | 重新选择设备 |

---

## 9. 文件变更清单

| 文件 | 变更内容 |
|------|---------|
| `nexus-common/src/protocol.rs` | `ExecuteToolRequest` 使用 `device_name`；`Heartbeat`/`RegisterTools` 携带 `device_name` |
| `nexus-server/src/state.rs` | `AppState` 加 `devices_by_user` 索引 |
| `nexus-server/src/ws.rs` | 注册/注销/重连时维护 `devices_by_user` 索引 |
| `nexus-server/src/tools_registry.rs` | 实现 `find_device_by_name`、`route_tool`、`build_tools_schema` |
| `nexus-server/src/agent_loop.rs` | 调用 `route_tool` 路由工具；调用 `build_tools_schema` 构建 LLM 请求 |
| `nexus-server/src/context.rs` | 实现设备列表上下文生成 |

---

## 10. 安全考量

1. **用户隔离**：device_name 查找受 user_id 约束，LLM 无法跨用户路由
2. **设备归属验证**：每次路由都通过 `devices_by_user[user_id]` 索引，不直接查 `devices[device_id]`
3. **ws_tx 有效性**：即使拿到 device_id，也检查 `devices.get(device_id).ws_tx` 是否有效
4. **设备状态验证**：路由前检查设备状态，拒绝路由到 offline 设备

---

## 11. OpenClaw 关键发现

### 11.1 设备配对机制（值得借鉴）

OpenClaw 的设备配对流程：

```
Node 连接 → Gateway 收到 device identity →
生成 device pairing request →
用户通过 `openclaw devices approve <requestId>` 审批
```

**可借鉴点**：NEXUS 目前 client 是"注册即可用"，未来可增加管理员审批机制增强安全性。

### 11.2 OpenClaw 不支持的功能（我们的差异化优势）

| 功能 | OpenClaw | NEXUS（规划） |
|------|----------|--------------|
| 跨用户设备共享 | ❌ | ✅（通过 session 共享） |
| 工具 Schema 验证 | 基础 | 完整 JSON Schema 验证 |
| 多 agent 协作 | 单 agent | 多 agent orchestration |
| 分布式状态同步 | 无 | heartbeat + tools_hash 同步 |

---

## 12. 待定事项

- [ ] 设备 busy 状态如何设定和清除（建议：执行工具时设 busy，完成后清除）
- [ ] 工具不存在于某设备时的具体错误码
- [ ] Schema 中如何区分不同工具（当前简化为单一 `run_shell_command`，实际需要多工具支持）
- [ ] 设备配对审批机制（可选，增强安全性）
