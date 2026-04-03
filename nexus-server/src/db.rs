/// 职责边界：
/// 1. 负责所有与 PostgreSQL 的交互 (SQLx 操作)。
/// 2. 处理 users、sessions、messages、memory_chunks 四张表的增删改查。
/// 3. 所有函数均为纯粹的 async CRUD，不包含任何业务逻辑。
///    业务逻辑（如 consolidation 触发判断、JWT 签发）由上层模块（memory.rs、auth.rs）负责。
///
/// 参考 nanobot：
/// - 这个文件替代了 `nanobot/agent/session.py`（会话管理）和 `nanobot/agent/memory.py`（长期记忆）。
/// - nanobot 基于本地文件（JSONL session 文件、MEMORY.md、HISTORY.md），Nexus 改为 PostgreSQL。

use serde::Serialize;
use sqlx::PgPool;
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub user_id: String,
    pub email: String,
    pub password_hash: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DiscordConfig {
    pub user_id: String,
    pub bot_token: String,
    pub bot_user_id: Option<String>,
    pub enabled: bool,
    pub allowed_users: Vec<String>,
    pub owner_discord_id: Option<String>,
}

pub async fn init_db(pool: &PgPool) -> Result<(), sqlx::Error> {
    // pgvector extension (must be first — memory_chunks uses the vector type)
    sqlx::query("CREATE EXTENSION IF NOT EXISTS vector")
        .execute(pool)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            user_id TEXT PRIMARY KEY,
            email TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL DEFAULT '',
            is_admin BOOLEAN NOT NULL DEFAULT FALSE,
            soul TEXT,
            preferences JSONB,
            created_at TIMESTAMPTZ DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS device_tokens (
            token TEXT PRIMARY KEY,
            user_id TEXT NOT NULL REFERENCES users(user_id),
            device_name TEXT NOT NULL,
            revoked BOOLEAN NOT NULL DEFAULT FALSE,
            fs_policy JSONB NOT NULL DEFAULT '{"mode":"sandbox"}',
            created_at TIMESTAMPTZ DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS idx_device_tokens_user_device ON device_tokens (user_id, device_name) WHERE revoked = FALSE")
        .execute(pool)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS sessions (
            session_id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL REFERENCES users(user_id),
            created_at TIMESTAMPTZ DEFAULT NOW(),
            last_consolidated TEXT
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS messages (
            message_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES sessions(session_id),
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            tool_call_id TEXT,
            tool_name TEXT,
            tool_arguments TEXT,
            is_consolidated BOOLEAN NOT NULL DEFAULT FALSE,
            created_at TIMESTAMPTZ DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS discord_configs (
            user_id TEXT PRIMARY KEY REFERENCES users(user_id),
            bot_token TEXT NOT NULL,
            bot_user_id TEXT,
            owner_discord_id TEXT,
            enabled BOOLEAN NOT NULL DEFAULT TRUE,
            allowed_users TEXT[] NOT NULL DEFAULT '{}',
            created_at TIMESTAMPTZ DEFAULT NOW(),
            updated_at TIMESTAMPTZ DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS system_config (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TIMESTAMPTZ DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS memory_chunks (
            id SERIAL PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES sessions(session_id),
            user_id TEXT NOT NULL REFERENCES users(user_id),
            history_entry TEXT NOT NULL,
            memory_text TEXT NOT NULL,
            embedding vector,
            created_at TIMESTAMPTZ DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn get_system_config(
    db: &PgPool,
    key: &str,
) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query_as::<_, (String,)>(
        "SELECT value FROM system_config WHERE key = $1"
    )
    .bind(key)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.0))
}

pub async fn set_system_config(
    db: &PgPool,
    key: &str,
    value: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO system_config (key, value, updated_at) VALUES ($1, $2, NOW()) ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = NOW()"
    )
    .bind(key)
    .bind(value)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn create_device_token(
    db: &PgPool,
    token: &str,
    user_id: &str,
    device_name: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO device_tokens (token, user_id, device_name) VALUES ($1, $2, $3)",
    )
    .bind(token)
    .bind(user_id)
    .bind(device_name)
    .execute(db)
    .await?;
    Ok(())
}

/// Returns (user_id, device_name) if token is valid and not revoked.
pub async fn verify_device_token(
    pool: &PgPool,
    token: &str,
) -> Result<Option<(String, String)>, sqlx::Error> {
    let row = sqlx::query_as::<_, (String, String)>(
        r#"
        SELECT user_id, COALESCE(device_name, 'unnamed')
        FROM device_tokens
        WHERE token = $1
          AND revoked = FALSE
        "#,
    )
    .bind(token)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn create_user(
    db: &PgPool,
    email: &str,
    password_hash: &str,
    is_admin: bool,
) -> Result<String, sqlx::Error> {
    let user_id = Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO users (user_id, email, password_hash, is_admin)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(&user_id)
    .bind(email)
    .bind(password_hash)
    .bind(is_admin)
    .execute(db)
    .await?;
    Ok(user_id)
}

pub async fn get_user_by_email(
    db: &PgPool,
    email: &str,
) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>(
        r#"
        SELECT user_id, email, password_hash, is_admin
        FROM users
        WHERE email = $1
        "#,
    )
    .bind(email)
    .fetch_optional(db)
    .await
}

pub async fn ensure_session(
    db: &PgPool,
    session_id: &str,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sessions (session_id, user_id) VALUES ($1, $2) ON CONFLICT (session_id) DO NOTHING",
    )
    .bind(session_id)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn save_message(
    db: &PgPool,
    session_id: &str,
    role: &str,
    content: &str,
    tool_call_id: Option<&str>,
    tool_name: Option<&str>,
    tool_arguments: Option<&str>,
) -> Result<String, sqlx::Error> {
    let message_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO messages (message_id, session_id, role, content, tool_call_id, tool_name, tool_arguments) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&message_id)
    .bind(session_id)
    .bind(role)
    .bind(content)
    .bind(tool_call_id)
    .bind(tool_name)
    .bind(tool_arguments)
    .execute(db)
    .await?;
    Ok(message_id)
}

pub async fn get_session_history(
    db: &PgPool,
    session_id: &str,
) -> Result<Vec<serde_json::Value>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT role, content, tool_call_id, tool_name, tool_arguments
        FROM messages
        WHERE session_id = $1
          AND is_consolidated = FALSE
        ORDER BY created_at ASC
        "#,
    )
    .bind(session_id)
    .fetch_all(db)
    .await?;

    let messages = rows
        .iter()
        .map(|row| {
            let role: String = row.get("role");
            let content: String = row.get("content");
            let tool_call_id: Option<String> = row.get("tool_call_id");
            let tool_name: Option<String> = row.get("tool_name");
            let tool_arguments: Option<String> = row.get("tool_arguments");

            if role == "assistant" && tool_name.is_some() {
                serde_json::json!({
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
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": content
                })
            } else {
                serde_json::json!({
                    "role": role,
                    "content": content
                })
            }
        })
        .collect();

    Ok(messages)
}

pub async fn get_all_discord_configs(
    db: &PgPool,
) -> Result<Vec<DiscordConfig>, sqlx::Error> {
    sqlx::query_as::<_, DiscordConfig>(
        r#"
        SELECT user_id, bot_token, bot_user_id, enabled, allowed_users, owner_discord_id
        FROM discord_configs
        WHERE enabled = TRUE
        "#,
    )
    .fetch_all(db)
    .await
}

pub async fn get_discord_config_by_user_id(
    db: &PgPool,
    user_id: &str,
) -> Result<Option<DiscordConfig>, sqlx::Error> {
    sqlx::query_as::<_, DiscordConfig>(
        r#"
        SELECT user_id, bot_token, bot_user_id, enabled, allowed_users, owner_discord_id
        FROM discord_configs
        WHERE user_id = $1
        "#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
}

pub async fn update_bot_user_id(
    db: &PgPool,
    user_id: &str,
    bot_user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE discord_configs
        SET bot_user_id = $1, updated_at = NOW()
        WHERE user_id = $2
        "#,
    )
    .bind(bot_user_id)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn upsert_discord_config(
    db: &PgPool,
    user_id: &str,
    bot_token: &str,
    allowed_users: &[String],
    owner_discord_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO discord_configs (user_id, bot_token, allowed_users, owner_discord_id)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (user_id) DO UPDATE
        SET bot_token = $2, allowed_users = $3, owner_discord_id = $4, updated_at = NOW()
        "#,
    )
    .bind(user_id)
    .bind(bot_token)
    .bind(allowed_users)
    .bind(owner_discord_id)
    .execute(db)
    .await?;
    Ok(())
}

// ============================================================================
// Admin API queries
// ============================================================================

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct DeviceTokenInfo {
    pub token: String,
    pub device_name: Option<String>,
    pub revoked: bool,
    pub created_at: Option<chrono::NaiveDateTime>,
}

pub async fn list_device_tokens(
    db: &PgPool,
    user_id: &str,
) -> Result<Vec<DeviceTokenInfo>, sqlx::Error> {
    sqlx::query_as::<_, DeviceTokenInfo>(
        "SELECT token, device_name, revoked, created_at FROM device_tokens WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(db)
    .await
}

pub async fn revoke_device_token(
    db: &PgPool,
    token: &str,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE device_tokens SET revoked = TRUE WHERE token = $1 AND user_id = $2 AND revoked = FALSE",
    )
    .bind(token)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_discord_config(
    db: &PgPool,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM discord_configs WHERE user_id = $1")
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct MessageInfo {
    pub message_id: String,
    pub role: String,
    pub content: String,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_arguments: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn get_session_messages(
    db: &PgPool,
    session_id: &str,
    user_id: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<MessageInfo>, sqlx::Error> {
    // Verify ownership first
    let session = sqlx::query_as::<_, (String,)>(
        "SELECT user_id FROM sessions WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_optional(db)
    .await?;

    match session {
        Some((owner,)) if owner == user_id => {}
        Some(_) => return Ok(vec![]), // Not owner, return empty
        None => return Ok(vec![]),
    }

    sqlx::query_as::<_, MessageInfo>(
        "SELECT message_id, role, content, tool_call_id, tool_name, tool_arguments, created_at FROM messages WHERE session_id = $1 ORDER BY created_at ASC LIMIT $2 OFFSET $3",
    )
    .bind(session_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(db)
    .await
}

pub async fn list_sessions_by_user(
    db: &PgPool,
    user_id: &str,
) -> Result<Vec<SessionInfo>, sqlx::Error> {
    sqlx::query_as::<_, SessionInfo>(
        "SELECT session_id, created_at FROM sessions WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(db)
    .await
}

pub async fn delete_session(
    db: &PgPool,
    session_id: &str,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query("DELETE FROM messages WHERE session_id = $1")
        .bind(session_id)
        .execute(db)
        .await?;
    let result = sqlx::query(
        "DELETE FROM sessions WHERE session_id = $1 AND user_id = $2",
    )
    .bind(session_id)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() > 0)
}

// ============================================================================
// StoredMessage (for consolidation)
// ============================================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StoredMessage {
    pub message_id: String,
    pub role: String,
    pub content: String,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_arguments: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

// ============================================================================
// Soul & Preferences
// ============================================================================

pub async fn get_user_soul(db: &PgPool, user_id: &str) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query_as::<_, (Option<String>,)>(
        "SELECT soul FROM users WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    Ok(row.and_then(|r| r.0))
}

pub async fn update_user_soul(db: &PgPool, user_id: &str, soul: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET soul = $1 WHERE user_id = $2")
        .bind(soul)
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn get_user_preferences(
    db: &PgPool,
    user_id: &str,
) -> Result<Option<serde_json::Value>, sqlx::Error> {
    let row = sqlx::query_as::<_, (Option<serde_json::Value>,)>(
        "SELECT preferences FROM users WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    Ok(row.and_then(|r| r.0))
}

pub async fn update_user_preferences(
    db: &PgPool,
    user_id: &str,
    prefs: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET preferences = $1 WHERE user_id = $2")
        .bind(prefs)
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(())
}

// ============================================================================
// Memory Chunks
// ============================================================================

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

pub async fn get_unconsolidated_messages(
    db: &PgPool,
    session_id: &str,
) -> Result<Vec<StoredMessage>, sqlx::Error> {
    sqlx::query_as::<_, StoredMessage>(
        "SELECT message_id, role, content, tool_call_id, tool_name, tool_arguments, created_at FROM messages WHERE session_id = $1 AND is_consolidated = FALSE ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(db)
    .await
}

pub async fn mark_messages_consolidated(
    db: &PgPool,
    message_ids: &[String],
) -> Result<(), sqlx::Error> {
    if message_ids.is_empty() {
        return Ok(());
    }
    sqlx::query("UPDATE messages SET is_consolidated = TRUE WHERE message_id = ANY($1)")
        .bind(message_ids)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn update_session_last_consolidated(
    db: &PgPool,
    session_id: &str,
    last_message_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE sessions SET last_consolidated = $1 WHERE session_id = $2")
        .bind(last_message_id)
        .bind(session_id)
        .execute(db)
        .await?;
    Ok(())
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
// Device Policy
// ============================================================================

pub async fn get_device_policy(
    db: &PgPool,
    user_id: &str,
    device_name: &str,
) -> Result<nexus_common::protocol::FsPolicy, sqlx::Error> {
    let row: (serde_json::Value,) = sqlx::query_as(
        "SELECT COALESCE(fs_policy, '{\"mode\":\"sandbox\"}'::jsonb) FROM device_tokens WHERE user_id = $1 AND device_name = $2 AND revoked = FALSE"
    )
    .bind(user_id)
    .bind(device_name)
    .fetch_one(db)
    .await?;

    serde_json::from_value(row.0)
        .map_err(|e| sqlx::Error::Protocol(format!("invalid fs_policy JSON: {e}")))
}

pub async fn update_device_policy(
    db: &PgPool,
    user_id: &str,
    device_name: &str,
    policy: &nexus_common::protocol::FsPolicy,
) -> Result<bool, sqlx::Error> {
    let json = serde_json::to_value(policy)
        .map_err(|e| sqlx::Error::Protocol(format!("failed to serialize policy: {e}")))?;

    let result = sqlx::query(
        "UPDATE device_tokens SET fs_policy = $1 WHERE user_id = $2 AND device_name = $3 AND revoked = FALSE"
    )
    .bind(json)
    .bind(user_id)
    .bind(device_name)
    .execute(db)
    .await?;

    Ok(result.rows_affected() > 0)
}
