# Embedding Config Enhancement + Re-embed + Dedup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enhance embedding config with `max_input_length` and `max_concurrency`, constrain memory generation to fit embedding limits, re-embed all memories on config change, truncate oversized memories, and deduplicate at write time.

**Architecture:** `EmbeddingConfig` gains two fields. A shared `Semaphore` throttles concurrent embedding calls. On config update, a background task wipes all embeddings and re-embeds every chunk (truncating if needed, flagging with `truncated=true`). At write time, vector-search before insert — skip if cosine similarity > 0.92.

**Tech Stack:** Rust (tokio::sync::Semaphore, sqlx, reqwest), PostgreSQL (pgvector)

---

## File Structure

| Action | File | Responsibility |
|--------|------|---------------|
| Modify | `nexus-server/src/config.rs` | Add `max_input_length` and `max_concurrency` to `EmbeddingConfig` |
| Modify | `nexus-server/src/db.rs` | Add `truncated` column to `memory_chunks`, add queries for re-embed (get all chunks, clear embeddings, update single embedding, dedup search) |
| Modify | `nexus-server/src/context.rs` | Use `max_input_length * 0.8` as `max_tokens` for consolidation LLM calls; add dedup check before saving; use semaphore for `embed_text` |
| Modify | `nexus-server/src/auth.rs` | Update `EmbeddingConfig` request to require new fields; trigger background re-embed on config change |
| Modify | `nexus-server/src/state.rs` | Add `Arc<Semaphore>` for embedding concurrency |
| Modify | `nexus-server/src/memory.rs` | Pass `max_tokens` derived from embedding config to consolidation LLM call |
| Modify | `nexus-server/src/agent_loop.rs` | Dedup check on `save_memory` tool; pass embedding semaphore |

---

### Task 1: Extend EmbeddingConfig and AppState

**Files:**
- Modify: `nexus-server/src/config.rs`
- Modify: `nexus-server/src/state.rs`

- [ ] **Step 1: Add fields to `EmbeddingConfig`**

In `nexus-server/src/config.rs`, add two fields to `EmbeddingConfig`:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EmbeddingConfig {
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    pub dimensions: usize,
    pub max_input_length: usize,
    pub max_concurrency: usize,
}
```

- [ ] **Step 2: Add embedding semaphore to AppState**

In `nexus-server/src/state.rs`, add:

```rust
use tokio::sync::Semaphore;
```

Add to `AppState`:

```rust
pub embedding_semaphore: Arc<Semaphore>,
```

Initialize with a default (e.g., 10) in `AppState::new()`. When embedding config is updated, the semaphore will be recreated with the new `max_concurrency`.

- [ ] **Step 3: Update AppState::new to initialize the semaphore**

In the `AppState::new` function (or wherever AppState is constructed in `main.rs`), add:

```rust
embedding_semaphore: Arc::new(Semaphore::new(10)),
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build --package nexus-server 2>&1 | grep "^error"`

- [ ] **Step 5: Commit**

```bash
git add nexus-server/src/config.rs nexus-server/src/state.rs
git commit -m "feat(config): add max_input_length, max_concurrency to EmbeddingConfig, add semaphore to AppState"
```

---

### Task 2: Database Schema — Add `truncated` Column

**Files:**
- Modify: `nexus-server/src/db.rs`

- [ ] **Step 1: Add `truncated` column to `memory_chunks` CREATE TABLE**

```sql
CREATE TABLE IF NOT EXISTS memory_chunks (
    id SERIAL PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    user_id TEXT NOT NULL REFERENCES users(user_id),
    history_entry TEXT NOT NULL,
    memory_text TEXT NOT NULL,
    embedding vector,
    truncated BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ DEFAULT NOW()
)
```

- [ ] **Step 2: Add query to get all memory chunks for re-embedding**

```rust
pub async fn get_all_memory_chunks_for_reembed(
    db: &PgPool,
) -> Result<Vec<(i32, String)>, sqlx::Error> {
    sqlx::query_as::<_, (i32, String)>(
        "SELECT id, memory_text FROM memory_chunks ORDER BY id"
    )
    .fetch_all(db)
    .await
}
```

- [ ] **Step 3: Add query to clear all embeddings**

```rust
pub async fn clear_all_embeddings(db: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE memory_chunks SET embedding = NULL, truncated = FALSE")
        .execute(db)
        .await?;
    Ok(())
}
```

- [ ] **Step 4: Add query to update a single chunk's embedding**

```rust
pub async fn update_memory_embedding(
    db: &PgPool,
    id: i32,
    embedding: &[f32],
    truncated: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE memory_chunks SET embedding = $1::vector, truncated = $2 WHERE id = $3"
    )
    .bind(embedding)
    .bind(truncated)
    .bind(id)
    .execute(db)
    .await?;
    Ok(())
}
```

- [ ] **Step 5: Add dedup query — find similar existing memory**

```rust
pub async fn find_similar_memory(
    db: &PgPool,
    user_id: &str,
    embedding: &[f32],
    threshold: f64,
) -> Result<bool, sqlx::Error> {
    let row: Option<(i32,)> = sqlx::query_as(
        "SELECT id FROM memory_chunks WHERE user_id = $1 AND embedding IS NOT NULL AND 1 - (embedding <=> $2::vector) > $3 LIMIT 1"
    )
    .bind(user_id)
    .bind(embedding)
    .bind(threshold)
    .fetch_optional(db)
    .await?;
    Ok(row.is_some())
}
```

- [ ] **Step 6: Verify it compiles**

Run: `cargo build --package nexus-server 2>&1 | grep "^error"`

- [ ] **Step 7: Commit**

```bash
git add nexus-server/src/db.rs
git commit -m "feat(db): add truncated column, re-embed queries, dedup similarity search"
```

---

### Task 3: Semaphore-Guarded embed_text

**Files:**
- Modify: `nexus-server/src/context.rs`

- [ ] **Step 1: Add a semaphore-aware version of `embed_text`**

Add a new function that wraps `embed_text` with semaphore acquisition:

```rust
pub async fn embed_text_throttled(
    config: &crate::config::EmbeddingConfig,
    text: &str,
    semaphore: &Arc<Semaphore>,
) -> Vec<f32> {
    let _permit = semaphore.acquire().await.expect("semaphore closed");
    embed_text(config, text).await
}
```

Update the existing `embed_text` call in `build_system_prompt` (RAG injection section) to use `embed_text_throttled` with the semaphore from AppState.

- [ ] **Step 2: Verify it compiles**

Run: `cargo build --package nexus-server 2>&1 | grep "^error"`

- [ ] **Step 3: Commit**

```bash
git add nexus-server/src/context.rs
git commit -m "feat(context): add embed_text_throttled with semaphore concurrency control"
```

---

### Task 4: Constrain Memory Generation Length

**Files:**
- Modify: `nexus-server/src/memory.rs`

- [ ] **Step 1: Use `max_input_length * 0.8` as `max_tokens` for consolidation LLM calls**

In the consolidation function where the LLM is called to generate memory summaries, read the embedding config and calculate:

```rust
let max_tokens = match state.config.embedding.read().await.as_ref() {
    Some(emb) => ((emb.max_input_length as f64) * 0.8) as usize,
    None => 1024, // fallback if no embedding config
};
```

Pass this `max_tokens` value to the LLM request body as the `max_tokens` parameter when calling the consolidation prompt.

- [ ] **Step 2: Verify it compiles**

Run: `cargo build --package nexus-server 2>&1 | grep "^error"`

- [ ] **Step 3: Commit**

```bash
git add nexus-server/src/memory.rs
git commit -m "feat(memory): constrain consolidation output to embedding model max_input_length * 0.8"
```

---

### Task 5: Dedup at Write Time

**Files:**
- Modify: `nexus-server/src/agent_loop.rs`

- [ ] **Step 1: Add dedup check before `save_memory_chunk`**

In the `save_memory` tool handler in `agent_loop.rs`, after generating the embedding but before saving to DB:

```rust
// Dedup: skip if a very similar memory already exists
if !embedding.is_empty() {
    match db::find_similar_memory(&state.db, &user_id, &embedding, 0.92).await {
        Ok(true) => {
            // Similar memory exists, skip saving
            tracing::info!("save_memory: skipping duplicate (cosine > 0.92)");
            // Return success to the agent but don't insert
            // ... return tool result indicating dedup ...
        }
        Ok(false) => { /* proceed with save */ }
        Err(e) => {
            tracing::warn!("save_memory: dedup check failed: {}, proceeding with save", e);
        }
    }
}
```

Also apply the same dedup check in the consolidation path in `memory.rs` where `save_memory_chunk` is called after generating embeddings.

- [ ] **Step 2: Verify it compiles**

Run: `cargo build --package nexus-server 2>&1 | grep "^error"`

- [ ] **Step 3: Commit**

```bash
git add nexus-server/src/agent_loop.rs nexus-server/src/memory.rs
git commit -m "feat(memory): deduplicate memories at write time (cosine > 0.92 = skip)"
```

---

### Task 6: Background Re-embed on Config Change

**Files:**
- Modify: `nexus-server/src/auth.rs`

- [ ] **Step 1: Update `UpdateEmbeddingConfigRequest` to require all fields**

```rust
#[derive(Debug, Deserialize)]
pub struct UpdateEmbeddingConfigRequest {
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    pub dimensions: usize,
    pub max_input_length: usize,
    pub max_concurrency: usize,
}
```

- [ ] **Step 2: Rebuild the config fully (not partial update) and recreate semaphore**

In `update_embedding_config`, replace the partial-update logic with full replacement:

```rust
let new_config = crate::config::EmbeddingConfig {
    api_base: payload.api_base,
    api_key: payload.api_key,
    model: payload.model,
    dimensions: payload.dimensions,
    max_input_length: payload.max_input_length,
    max_concurrency: payload.max_concurrency,
};

*state.config.embedding.write().await = Some(new_config.clone());

// Recreate semaphore with new concurrency
// Note: old semaphore permits will drain naturally
let new_sem = Arc::new(Semaphore::new(new_config.max_concurrency));
// Store new semaphore in AppState (need a way to swap it)
```

- [ ] **Step 3: Spawn background re-embed task**

After saving the config, spawn a background task:

```rust
let db = state.db.clone();
let emb_config = new_config.clone();
let semaphore = state.embedding_semaphore.clone();

tokio::spawn(async move {
    tracing::info!("re-embed: starting background re-embedding of all memory chunks");

    // 1. Clear all existing embeddings
    if let Err(e) = db::clear_all_embeddings(&db).await {
        tracing::error!("re-embed: failed to clear embeddings: {}", e);
        return;
    }

    // 2. Fetch all chunks
    let chunks = match db::get_all_memory_chunks_for_reembed(&db).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("re-embed: failed to fetch chunks: {}", e);
            return;
        }
    };

    tracing::info!("re-embed: processing {} chunks", chunks.len());
    let mut success = 0u32;
    let mut truncated_count = 0u32;
    let mut failed = 0u32;

    for (id, memory_text) in &chunks {
        // Truncate if needed (rough estimate: 1 token ≈ 3 chars)
        let max_chars = emb_config.max_input_length * 3;
        let (text_to_embed, is_truncated) = if memory_text.len() > max_chars {
            (&memory_text[..max_chars], true)
        } else {
            (memory_text.as_str(), false)
        };

        let embedding = crate::context::embed_text_throttled(&emb_config, text_to_embed, &semaphore).await;
        if embedding.is_empty() {
            tracing::warn!("re-embed: failed to embed chunk id={}", id);
            failed += 1;
            continue;
        }

        if let Err(e) = db::update_memory_embedding(&db, *id, &embedding, is_truncated).await {
            tracing::warn!("re-embed: failed to update chunk id={}: {}", id, e);
            failed += 1;
        } else {
            success += 1;
            if is_truncated { truncated_count += 1; }
        }
    }

    tracing::info!("re-embed: done. {} success ({} truncated), {} failed", success, truncated_count, failed);
});
```

- [ ] **Step 4: Persist config and respond**

```rust
if let Ok(json_str) = serde_json::to_string(&new_config) {
    if let Err(e) = db::set_system_config(&state.db, "embedding_config", &json_str).await {
        tracing::error!("Failed to persist embedding config to DB: {e}");
    }
}

(StatusCode::OK, "Embedding config updated. Re-embedding started in background.").into_response()
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo build --package nexus-server 2>&1 | grep "^error"`

- [ ] **Step 6: Commit**

```bash
git add nexus-server/src/auth.rs
git commit -m "feat(auth): full embedding config replacement, background re-embed on change"
```

---

### Task 7: Build Verification

- [ ] **Step 1: Full workspace build**

```bash
cargo build 2>&1 | grep "^error"
```
Expected: No errors.

- [ ] **Step 2: Commit any fixes**

```bash
git add -u
git commit -m "fix: address build issues from embedding config changes"
```
