# NEXUS 设备路由设计文档

> 本文档定义 NEXUS 多设备场景下的工具路由机制。
>
> **参考实现**：OpenClaw 分布式架构（Gateway 中心化 + Node 外设模式）
>
> **状态**：设计已批准，待实现（M3 剩余工作）

---

## 1. 背景与问题

### 1.1 场景

一个用户（user_id）可以拥有多台设备（device）。每台设备注册自己的工具集（shell、MCP、Skills 等）。当 Agent 需要执行工具时，必须明确路由到哪台设备。

### 1.2 OpenClaw 参考架构

```
┌─────────────────────────────────────────────────────────────┐
│                         Gateway                              │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  · 掌握所有 session、认证、channel、状态              │   │
│  │  · 接收外部消息（Telegram/Discord/...）              │   │
│  │  · 路由工具调用到指定 Node                           │   │
│  └─────────────────────────────────────────────────────┘   │
│                            ↑                                │
│    ┌──────────┐   ┌──────────┐   ┌──────────┐             │
│    │  macOS   │   │  iOS     │   │ Android  │             │
│    │  Node    │   │  Node    │   │  Node    │             │
│    └──────────┘   └──────────┘   └──────────┘             │
└─────────────────────────────────────────────────────────────┘
```

NEXUS 的 `nexus-server` ↔ `nexus-client` 架构与 OpenClaw 完全一致。

### 1.3 核心问题

1. LLM 无法正确填写 `device_id`（32位 hash，人类不可读）
2. Server 端缺少 `device_name` → `device_id` 的高效查找路径
3. Agent 上下文缺少设备列表，LLM 无从判断工具路由到哪台设备

---

## 2. 设计原则

1. **user_id 对 LLM 不可见** — 系统自动从会话获取，LLM 只能指定 device_name
2. **device_name 客户端配置** — 在 Client 配置文件中设置，人类可读
3. **O(1) 查找** — `devices_by_user[user_id][device_name] = device_id` 二级索引
4. **按用户隔离** — device_name 在同一用户下唯一，不同用户可重名
5. **Schema 自描述** — Server 在构建 LLM 请求时注入 `device_name` enum，由 LLM 原生能力驱动路由

---

## 3. 路由方案：Schema 自描述路由

### 3.1 核心思想

Client 上报的原始工具 schema 不包含 `device_name`。Server 在构建 LLM 请求时**修饰**每个工具的 schema，注入 `device_name` enum 参数：

```json
{
  "name": "run_shell_command",
  "parameters": {
    "properties": {
      "device_name": {
        "type": "string",
        "enum": ["mac-mini", "ubuntu-server"],
        "description": "The device to run the command on."
      },
      "command": { "type": "string" }
    },
    "required": ["device_name", "command"]
  }
}
```

`device_name` 是 NEXUS 路由机制，**不是工具本身的参数**，Client 收到的 `ExecuteToolRequest` 不含此字段。

### 3.2 路由决策流程

```
LLM 选择 device_name（如 "ubuntu-server"）
  → find_device_by_name(user_id, "ubuntu-server")
  → 验证设备在线
  → 剥离 device_name，发送 ExecuteToolRequest 到目标 Client
  → 若设备不存在/离线 → 返回错误 → LLM 重新选择
```

### 3.3 设备状态与 schema 联动

| 事件 | enum 变化 | LLM 行为 |
|------|-----------|---------|
| 设备注册 | 新增到 enum | 可使用 |
| 设备离线 | 标记 offline | 通常避免选择 |
| 设备注销 | 从 enum 移除 | 不再尝试 |

---

## 4. 协议变更

### 4.1 ExecuteToolRequest（Server → Client，已有字段删除）

```rust
pub struct ExecuteToolRequest {
    pub request_id: String,
    pub tool: String,
    pub params: Value,   // 已剥离 device_name
    // device_id / device_name 字段移除，不传给 Client
}
```

### 4.2 RegisterTools / Heartbeat 新增 device_name

```rust
RegisterTools {
    device_id: DeviceId,
    device_name: String,     // 新增：Client 配置的人类可读名称
    schemas: Vec<ToolSchema>,
}

Heartbeat {
    device_id: DeviceId,
    device_name: String,     // 新增
    tools_hash: String,
    status: DeviceStatus,
}
```

### 4.3 Client 配置

```toml
[device]
name = "macmini-office"   # 必填，人类可读，用户自定义
```

---

## 5. 数据结构变更（state.rs）

```rust
pub struct AppState {
    pub devices: Arc<RwLock<HashMap<DeviceId, DeviceState>>>,

    // 新增：user_id → { device_name → device_id }
    pub devices_by_user: Arc<RwLock<HashMap<UserId, HashMap<DeviceName, DeviceId>>>>,

    pub pending: Arc<RwLock<HashMap<RequestId, oneshot::Sender<ToolExecutionResult>>>>,
    // ...
}
```

索引维护：设备注册时插入，断线/注销时移除。

---

## 6. 关键实现函数

### find_device_by_name（tools_registry.rs）

```rust
pub async fn find_device_by_name(
    state: &AppState,
    user_id: &UserId,
    device_name: &str,
) -> Option<DeviceId> {
    let devices = state.devices_by_user.read().await;
    devices.get(user_id)?.get(device_name).cloned()
}
```

### build_tools_schema（tools_registry.rs）

每次 LLM 请求前，遍历工具 schema，注入 `device_name` enum：

```rust
pub async fn build_tools_schema(
    state: &AppState,
    user_id: &UserId,
    original_schemas: Vec<ToolSchema>,
) -> Vec<Value> {
    let device_enum = get_online_device_names(state, user_id).await;

    original_schemas.into_iter().map(|schema| {
        // 在 properties 中注入 device_name enum
        // 在 required 中添加 "device_name"
        // 保留原有参数不变
    }).collect()
}
```

---

## 7. Agent 上下文（context.rs）

system prompt 注入设备列表：

```
Available devices:
- mac-mini      | status: online
- ubuntu-server | status: online
- gpu-server    | status: offline
```

每次 Agent Loop 开始时刷新（设备状态实时变化）。

---

## 8. 错误处理

| 场景 | LLM 收到的错误信息 |
|------|------------------|
| 设备不存在 | "设备 '{name}' 不存在" |
| 设备离线 | "设备 '{name}' 当前离线" |
| 工具不存在于该设备 | "设备 '{name}' 上没有工具 '{tool}'" |
| 发送失败 | "向设备 '{name}' 发送请求失败" |

所有错误均由 LLM 自然重试（重新选择设备）。

---

## 9. 文件变更清单

| 文件 | 变更内容 |
|------|---------|
| `nexus-common/src/protocol.rs` | `ExecuteToolRequest` 移除 device 路由字段；`Heartbeat`/`RegisterTools` 携带 `device_name` |
| `nexus-server/src/state.rs` | `AppState` 加 `devices_by_user` 二级索引 |
| `nexus-server/src/ws.rs` | 注册/断线时维护 `devices_by_user` 索引 |
| `nexus-server/src/tools_registry.rs` | 实现 `find_device_by_name`、`route_tool`、`build_tools_schema` |
| `nexus-server/src/agent_loop.rs` | 调用 `build_tools_schema`、`route_tool` |
| `nexus-server/src/context.rs` | 生成设备列表上下文 |
| `nexus-client/src/config.rs` | 读取 `device.name` 配置项 |
| `nexus-client/src/session.rs` | RegisterTools/Heartbeat 携带 `device_name` |

---

## 10. 待定事项

- [ ] 设备 busy 状态如何设定与清除（执行工具时 busy，完成后 online）
- [ ] 设备配对审批机制（可选，类似 OpenClaw 的 approve 流程）
- [ ] 同名设备重连时旧 pending 请求的处理策略
