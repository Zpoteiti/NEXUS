/// Responsibility boundary:
/// 1. Handles all PostgreSQL interactions (SQLx operations).
/// 2. CRUD operations for users, sessions, and messages tables.
/// 3. All functions are pure async CRUD -- no business logic.
///    Business logic (consolidation triggers, JWT issuance) is handled by upper-layer modules (memory.rs, auth.rs).

mod users;
mod sessions;
mod messages;
mod devices;
mod discord;
mod cron;
mod skills;
mod checkpoints;

pub use users::*;
pub use sessions::*;
pub use messages::*;
pub use devices::*;
pub use discord::*;
pub use cron::*;
pub use skills::*;
pub use checkpoints::*;

use sqlx::PgPool;

pub async fn init_db(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            user_id TEXT PRIMARY KEY,
            email TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL DEFAULT '',
            is_admin BOOLEAN NOT NULL DEFAULT FALSE,
            soul TEXT,
            preferences JSONB,
            memory_text TEXT NOT NULL DEFAULT '',
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
            mcp_config JSONB NOT NULL DEFAULT '[]',
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
        CREATE TABLE IF NOT EXISTS cron_jobs (
            job_id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL REFERENCES users(user_id),
            name TEXT NOT NULL,
            enabled BOOLEAN DEFAULT TRUE,
            cron_expr TEXT,
            every_seconds INTEGER,
            run_at TIMESTAMPTZ,
            timezone TEXT DEFAULT 'UTC',
            message TEXT NOT NULL,
            channel TEXT NOT NULL,
            chat_id TEXT NOT NULL,
            delete_after_run BOOLEAN DEFAULT FALSE,
            next_run_at TIMESTAMPTZ,
            last_run_at TIMESTAMPTZ,
            run_count INTEGER DEFAULT 0,
            created_at TIMESTAMPTZ DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS agent_checkpoints (
            session_id TEXT PRIMARY KEY REFERENCES sessions(session_id),
            user_id TEXT NOT NULL,
            messages JSONB NOT NULL,
            iteration INTEGER DEFAULT 0,
            channel TEXT NOT NULL,
            chat_id TEXT NOT NULL,
            created_at TIMESTAMPTZ DEFAULT NOW(),
            updated_at TIMESTAMPTZ DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS skills (
            skill_id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL REFERENCES users(user_id),
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            always_on BOOLEAN DEFAULT FALSE,
            skill_path TEXT NOT NULL,
            created_at TIMESTAMPTZ DEFAULT NOW(),
            UNIQUE(user_id, name)
        )
        "#,
    )
    .execute(pool)
    .await?;

    // Safe migrations for existing databases
    sqlx::query("ALTER TABLE users ADD COLUMN IF NOT EXISTS memory_text TEXT NOT NULL DEFAULT ''")
        .execute(pool)
        .await?;

    sqlx::query("ALTER TABLE messages ADD COLUMN IF NOT EXISTS compressed BOOLEAN DEFAULT FALSE")
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
