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
}

pub async fn init_db(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            user_id TEXT PRIMARY KEY,
            email TEXT UNIQUE NOT NULL,
            created_at TIMESTAMP DEFAULT NOW()
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
            device_name TEXT,
            revoked BOOLEAN NOT NULL DEFAULT FALSE,
            created_at TIMESTAMP DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("ALTER TABLE users ADD COLUMN IF NOT EXISTS password_hash TEXT NOT NULL DEFAULT ''")
        .execute(pool)
        .await?;

    sqlx::query("ALTER TABLE users ADD COLUMN IF NOT EXISTS is_admin BOOLEAN NOT NULL DEFAULT FALSE")
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
            enabled BOOLEAN NOT NULL DEFAULT TRUE,
            allowed_users TEXT[] NOT NULL DEFAULT '{}',
            created_at TIMESTAMPTZ DEFAULT NOW(),
            updated_at TIMESTAMPTZ DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
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
) -> Result<String, sqlx::Error> {
    let message_id = Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO messages (message_id, session_id, role, content, tool_call_id)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(&message_id)
    .bind(session_id)
    .bind(role)
    .bind(content)
    .bind(tool_call_id)
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
        SELECT role, content, tool_call_id
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

            let mut obj = serde_json::json!({
                "role": role,
                "content": content,
            });

            if let Some(id) = tool_call_id {
                obj["tool_call_id"] = serde_json::Value::String(id);
            }

            obj
        })
        .collect();

    Ok(messages)
}

pub async fn get_all_discord_configs(
    db: &PgPool,
) -> Result<Vec<DiscordConfig>, sqlx::Error> {
    sqlx::query_as::<_, DiscordConfig>(
        r#"
        SELECT user_id, bot_token, bot_user_id, enabled, allowed_users
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
        SELECT user_id, bot_token, bot_user_id, enabled, allowed_users
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
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO discord_configs (user_id, bot_token, allowed_users)
        VALUES ($1, $2, $3)
        ON CONFLICT (user_id) DO UPDATE
        SET bot_token = $2, allowed_users = $3, updated_at = NOW()
        "#,
    )
    .bind(user_id)
    .bind(bot_token)
    .bind(allowed_users)
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
