use serde::Serialize;
use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct MemoryChunk {
    pub id: i32,
    pub session_id: String,
    pub user_id: String,
    pub history_entry: String,
    pub memory_text: String,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn save_memory_chunk(
    db: &PgPool,
    session_id: &str,
    user_id: &str,
    history_entry: &str,
    memory_text: &str,
    embedding: Option<&[f32]>,
) -> Result<(), sqlx::Error> {
    if let Some(emb) = embedding {
        let emb_str = format!(
            "[{}]",
            emb.iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        sqlx::query(
            "INSERT INTO memory_chunks (session_id, user_id, history_entry, memory_text, embedding) VALUES ($1, $2, $3, $4, $5::vector)",
        )
        .bind(session_id)
        .bind(user_id)
        .bind(history_entry)
        .bind(memory_text)
        .bind(&emb_str)
        .execute(db)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO memory_chunks (session_id, user_id, history_entry, memory_text) VALUES ($1, $2, $3, $4)",
        )
        .bind(session_id)
        .bind(user_id)
        .bind(history_entry)
        .bind(memory_text)
        .execute(db)
        .await?;
    }
    Ok(())
}

pub async fn get_latest_memory_text(
    db: &PgPool,
    session_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query_as::<_, (String,)>(
        "SELECT memory_text FROM memory_chunks WHERE session_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(session_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.0))
}

pub async fn vector_search_memory(
    db: &PgPool,
    user_id: &str,
    query_embedding: &[f32],
    top_k: usize,
) -> Result<Vec<MemoryChunk>, sqlx::Error> {
    let emb_str = format!(
        "[{}]",
        query_embedding
            .iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    sqlx::query_as::<_, MemoryChunk>(
        "SELECT id, session_id, user_id, history_entry, memory_text, created_at FROM memory_chunks WHERE user_id = $1 AND embedding IS NOT NULL ORDER BY embedding <=> $2::vector LIMIT $3",
    )
    .bind(user_id)
    .bind(&emb_str)
    .bind(top_k as i64)
    .fetch_all(db)
    .await
}

pub async fn list_memory_chunks(
    db: &PgPool,
    user_id: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<MemoryChunk>, sqlx::Error> {
    sqlx::query_as::<_, MemoryChunk>(
        "SELECT id, session_id, user_id, history_entry, memory_text, created_at FROM memory_chunks WHERE user_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
    )
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(db)
    .await
}

// ============================================================================
// Re-embed & Dedup
// ============================================================================

#[derive(Debug, sqlx::FromRow)]
pub struct MemoryChunkForReembed {
    pub id: i32,
    pub memory_text: String,
}

pub async fn get_all_memory_chunks_for_reembed(
    db: &PgPool,
) -> Result<Vec<MemoryChunkForReembed>, sqlx::Error> {
    sqlx::query_as::<_, MemoryChunkForReembed>(
        "SELECT id, memory_text FROM memory_chunks ORDER BY id"
    )
    .fetch_all(db)
    .await
}

pub async fn clear_all_embeddings(db: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE memory_chunks SET embedding = NULL, truncated = FALSE")
        .execute(db)
        .await?;
    Ok(())
}

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
