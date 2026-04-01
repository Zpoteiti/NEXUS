# M3 Final Three — 消息持久化 + Client 修复 + 上下文压缩

**Date:** 2026-03-31
**Goal:** 修复最后三个硬伤，使 NEXUS server+client 成为完整可用的 AI agent 系统。

---

## TASK-1: 消息持久化修复（assistant tool_calls 未存 DB）

### 问题

当前 `agent_loop.rs` 保存了 user 消息、assistant 文本回复、tool result，但 **assistant 的 tool_calls 消息从未写入 DB**。`messages` 表也缺少 `tool_name` 和 `tool_arguments` 字段。

后果：session 跨轮次时，历史消息重建缺少 tool_calls，LLM 看到孤立的 tool result。OpenAI API 要求 tool result 前必须有对应的 assistant tool_calls 消息。

### 修复方案

**1. 扩展 messages 表 schema（db.rs init_db）**

新增两列：
```sql
ALTER TABLE messages ADD COLUMN IF NOT EXISTS tool_name TEXT;
ALTER TABLE messages ADD COLUMN IF NOT EXISTS tool_arguments TEXT;
```

**2. 扩展 save_message 函数**

```rust
pub async fn save_message(
    db: &PgPool,
    session_id: &str,
    role: &str,
    content: &str,
    tool_call_id: Option<&str>,
    tool_name: Option<&str>,
    tool_arguments: Option<&str>,
) -> Result<String, sqlx::Error>
```

**3. agent_loop.rs 中存 tool_calls 消息**

在 `execute_tool_calls_loop` 中，每次构造 assistant tool_calls 消息加入 `current_messages` 时，同步写 DB：

```rust
for tc in &current_tool_calls {
    // 存 assistant tool_call 到 DB
    let _ = db::save_message(
        &state.db, session_id, "assistant", "",
        Some(&tc.id), Some(&tc.name), Some(&tc.arguments.to_string()),
    ).await;
    // ... 现有的 current_messages.push 逻辑
}
```

tool result 的保存也补上 tool_call_id：
```rust
let _ = db::save_message(
    &state.db, session_id, "tool", &content,
    Some(&tc.id), None, None,
).await;
```

**4. 修复 get_session_history 历史重建**

当前返回简单的 `{role, content, tool_call_id}`，需要完整重建 tool_calls 格式：

```rust
// role == "assistant" && tool_name.is_some() → 重建为 tool_calls 格式
if role == "assistant" && tool_name.is_some() {
    json!({
        "role": "assistant",
        "tool_calls": [{
            "id": tool_call_id,
            "type": "function",
            "function": {
                "name": tool_name,
                "arguments": tool_arguments
            }
        }]
    })
} else if role == "tool" {
    json!({
        "role": "tool",
        "tool_call_id": tool_call_id,
        "content": content
    })
} else {
    json!({ "role": role, "content": content })
}
```

**5. 更新所有 save_message 调用点**

现有的 `save_message` 调用全部增加新参数 `None, None`（user 和 assistant 文本消息不需要这些字段）。

**Files:** `nexus-server/src/db.rs`, `nexus-server/src/agent_loop.rs`

---

## TASK-2: Client 工具执行挂死修复

### 问题

`nexus-client/src/env.rs` 的 `sanitize_path()` 调用 `Path::canonicalize()`——同步阻塞系统调用，在 tokio async 上下文中阻塞 executor 线程。Windows 上尤其容易卡死。且 fs 工具（list_dir、read_file 等）没有 timeout 保护。

调用链：`executor::execute_tool_request` → `ListDirTool::execute` → `resolve_required_path` → `sanitize_path` → `canonicalize()` → **挂死**

### 修复方案

**1. env.rs — 用 spawn_blocking 包装 canonicalize**

```rust
pub async fn sanitize_path_async(raw: &str, restrict: bool) -> PathBuf {
    let raw = raw.to_string();
    tokio::task::spawn_blocking(move || sanitize_path(&raw, restrict))
        .await
        .unwrap_or_else(|_| PathBuf::from(&raw))
}
```

保留原始同步 `sanitize_path` 不变（测试用），新增 async 版本。

**2. fs.rs — 所有工具改用 async 路径解析 + timeout**

```rust
async fn resolve_path_async(path: &str) -> PathBuf {
    env::sanitize_path_async(path, true).await
}
```

每个工具的 `execute` 方法包装 timeout：
```rust
async fn execute(&self, arguments: Value) -> ToolResult {
    match tokio::time::timeout(Duration::from_secs(30), self.execute_inner(arguments)).await {
        Ok(result) => result,
        Err(_) => ToolResult { exit_code: 1, output: "tool execution timed out".to_string() },
    }
}
```

**3. executor.rs — 顶层 timeout 兜底**

```rust
pub async fn execute_tool_request(req: ExecuteToolRequest) -> ToolExecutionResult {
    let result = match tokio::time::timeout(
        Duration::from_secs(120),
        execute_inner(&req),
    ).await {
        Ok(r) => r,
        Err(_) => ToolResult { exit_code: 1, output: "execution timed out after 120s".to_string() },
    };
    // ...
}
```

**Files:** `nexus-client/src/env.rs`, `nexus-client/src/tools/fs.rs`, `nexus-client/src/executor.rs`

---

## TASK-3: 上下文压缩（完整实现）

参考 nanobot/agent/memory.py 的设计，在 NEXUS 中实现完整的上下文压缩系统。

### 3.1 配置扩展

**LlmConfig 新增字段：**
```rust
pub struct LlmConfig {
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    pub context_window: usize,       // 204800
    pub max_output_tokens: usize,    // 131072
}
```

**新增 EmbeddingConfig（与 LlmConfig 同级，同样 Arc<RwLock<>>，admin API 热更新）：**
```rust
pub struct EmbeddingConfig {
    pub api_base: String,     // "http://localhost:xxxx/v1"
    pub api_key: String,      // local 可为空
    pub model: String,        // "Qwen3-Embedding-8B"
    pub dimensions: usize,    // 1024
}
```

**ServerConfig：**
```rust
pub struct ServerConfig {
    // ... existing fields ...
    pub llm: Arc<RwLock<LlmConfig>>,
    pub embedding: Arc<RwLock<EmbeddingConfig>>,
}
```

**Admin API 新增：**
- `GET /api/embedding-config` — 获取 embedding 配置（admin only）
- `PUT /api/embedding-config` — 更新 embedding 配置（admin only）

### 3.2 DB 扩展

**新建 memory_chunks 表（init_db 中添加）：**

需要先确保 pgvector 扩展已启用。

```sql
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS memory_chunks (
    id SERIAL PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    user_id TEXT NOT NULL REFERENCES users(user_id),
    history_entry TEXT NOT NULL,
    memory_text TEXT NOT NULL,
    embedding vector,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
```

注意：`vector` 类型不指定维度，由插入时的实际向量长度决定。这样维度可通过 admin API 随时调整。

**新增 DB 函数：**

```rust
pub async fn save_memory_chunk(
    db: &PgPool,
    session_id: &str,
    user_id: &str,
    history_entry: &str,
    memory_text: &str,
    embedding: Option<&[f32]>,
) -> Result<(), sqlx::Error>

pub async fn get_latest_memory_text(
    db: &PgPool,
    session_id: &str,
) -> Result<Option<String>, sqlx::Error>
// 获取该 session 最新的 memory_text（用于 consolidation 提示词）

pub async fn vector_search_memory(
    db: &PgPool,
    user_id: &str,
    query_embedding: &[f32],
    top_k: usize,
) -> Result<Vec<MemoryChunk>, sqlx::Error>
// RAG 检索：按用户维度跨 session 搜索最相关的记忆

pub async fn mark_messages_consolidated(
    db: &PgPool,
    message_ids: &[String],
) -> Result<(), sqlx::Error>

pub async fn update_session_last_consolidated(
    db: &PgPool,
    session_id: &str,
    last_message_id: &str,
) -> Result<(), sqlx::Error>

pub async fn get_unconsolidated_messages(
    db: &PgPool,
    session_id: &str,
) -> Result<Vec<StoredMessage>, sqlx::Error>
// 返回完整的 StoredMessage（含 message_id、created_at、tool 字段）
```

### 3.3 Embedding 调用（context.rs）

替换原来的 `embed_text` stub：

```rust
pub async fn embed_text(
    config: &EmbeddingConfig,
    text: &str,
) -> Vec<f32> {
    // POST {config.api_base}/embeddings
    // Body: {"model": config.model, "input": text, "dimensions": config.dimensions}
    // Response: {"data": [{"embedding": [...]}]}
    // 失败返回空 Vec（跳过 embedding，不阻断流程）
}
```

### 3.4 memory.rs 完整实现

**Token 估算：**
```rust
fn estimate_tokens(messages: &[Value]) -> usize {
    messages.iter()
        .map(|m| {
            let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let tool_args = m.get("tool_calls")
                .map(|v| v.to_string().len())
                .unwrap_or(0);
            (content.len() + tool_args) / 3
        })
        .sum()
}
```

**触发条件（参考 nanobot）：**
```rust
let budget = llm_config.context_window - llm_config.max_output_tokens - 1024; // safety buffer
let target = budget / 2;
let estimated = estimate_tokens(&all_messages);
if estimated >= budget {
    // 循环压缩直到 estimated < target，最多 5 轮
}
```

**pick_consolidation_boundary：**
从消息列表开头往后扫，找到最远的 `role=user` 边界，使得该边界之前的消息 token 数 >= 需要移除的量。避免在 assistant/tool 中间截断。

**save_memory tool 定义（内置，不注册到 client）：**
```json
{
  "type": "function",
  "function": {
    "name": "save_memory",
    "description": "Save the memory consolidation result.",
    "parameters": {
      "type": "object",
      "properties": {
        "history_entry": {
          "type": "string",
          "description": "Timestamped summary: [YYYY-MM-DD HH:MM] key events and decisions."
        },
        "memory_update": {
          "type": "string",
          "description": "Full updated long-term memory as markdown. Merge new info with existing."
        }
      },
      "required": ["history_entry", "memory_update"]
    }
  }
}
```

**consolidate 流程：**
1. `get_unconsolidated_messages` 获取待压缩消息
2. `pick_consolidation_boundary` 确定边界
3. 格式化消息为文本：`[timestamp] ROLE: content`
4. 获取当前 memory_text（`get_latest_memory_text`）
5. 构造 consolidation prompt + save_memory tool，调 LLM（tool_choice=forced）
6. 解析 LLM 返回的 save_memory 调用
7. 调 `embed_text` 生成 embedding
8. 存入 `memory_chunks` 表
9. `mark_messages_consolidated` + `update_session_last_consolidated`
10. 失败时：计数 +1，3 次后 raw archive fallback

**3-strike fallback：**
```rust
// per-session 失败计数，存在 DashMap<String, usize> 中
static FAILURE_COUNTS: LazyLock<DashMap<String, usize>> = LazyLock::new(DashMap::new);
const MAX_FAILURES: usize = 3;
```

### 3.5 RAG 注入（context.rs）

在 `build_system_prompt` 的段 4 实现 RAG 注入：

```rust
// 段 4 — RAG 长期记忆注入
let embedding_config = state.config.embedding.read().await.clone();
if !embedding_config.api_base.is_empty() {
    let query_emb = embed_text(&embedding_config, user_input).await;
    if !query_emb.is_empty() {
        let chunks = db::vector_search_memory(&state.db, user_id, &query_emb, 5).await
            .unwrap_or_default();
        if !chunks.is_empty() {
            let memory_text = chunks.iter()
                .map(|c| c.history_entry.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!("## Relevant Memory\n{}", memory_text));
        }
    }
}
```

### 3.6 agent_loop.rs 集成

在 `run_single_turn` 中，LLM 调用前触发压缩：

```rust
async fn run_single_turn(...) -> Result<String, String> {
    let llm_config = state.config.llm.read().await.clone();

    // 压缩检查（在构建历史之前）
    crate::memory::maybe_consolidate(
        session_id, &event.sender_id, &state.db, &llm_config, &state.config.embedding,
    ).await;

    let system_prompt = context::build_system_prompt(...).await;
    // ... 现有逻辑
}
```

### 3.7 Cargo.toml 依赖

```toml
pgvector = "0.4"  # pgvector Rust 客户端，提供 Vector 类型
```

---

## 执行顺序

1. **TASK-1** 消息持久化 — 扩展 schema + save tool_calls + 修复历史重建
2. **TASK-2** Client 修复 — spawn_blocking + timeout
3. **TASK-3a** 配置扩展 — LlmConfig 加 context_window/max_output_tokens、新增 EmbeddingConfig + admin API
4. **TASK-3b** DB 扩展 — memory_chunks 表 + 新增 DB 函数
5. **TASK-3c** Embedding 调用 — context.rs embed_text 实现
6. **TASK-3d** memory.rs 完整实现 — 压缩逻辑 + 3-strike fallback
7. **TASK-3e** RAG 注入 — context.rs 段 4
8. **TASK-3f** agent_loop 集成 — 触发点
9. **编译 + 测试**

---

## 不在范围

- Session RESUME（Discord Gateway）— M4
- 用户 soul/preferences 注入 — M4+
- 多模态消息（图片/附件）— M4+
