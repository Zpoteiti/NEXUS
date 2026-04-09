//! Database initialization and CRUD modules.
//! All queries via sqlx::query / sqlx::query_as (runtime unchecked).

pub mod devices;
pub mod messages;
pub mod sessions;
pub mod system_config;
pub mod users;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tracing::info;

pub async fn init_db(database_url: &str) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(nexus_common::consts::DB_POOL_MAX_CONNECTIONS)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(database_url)
        .await
        .expect("Failed to connect to database");

    create_tables(&pool).await;
    info!("Database initialized");
    pool
}

async fn create_tables(pool: &PgPool) {
    let statements = [
        "CREATE TABLE IF NOT EXISTS users (
            user_id        TEXT PRIMARY KEY,
            email          TEXT UNIQUE NOT NULL,
            password_hash  TEXT NOT NULL DEFAULT '',
            is_admin       BOOLEAN DEFAULT FALSE,
            soul           TEXT,
            memory_text    TEXT NOT NULL DEFAULT '',
            created_at     TIMESTAMPTZ DEFAULT NOW()
        )",
        "CREATE TABLE IF NOT EXISTS device_tokens (
            token          TEXT PRIMARY KEY,
            user_id        TEXT NOT NULL REFERENCES users(user_id),
            device_name    TEXT NOT NULL,
            fs_policy      JSONB NOT NULL DEFAULT '{\"mode\":\"sandbox\"}',
            mcp_config     JSONB NOT NULL DEFAULT '[]',
            workspace_path TEXT NOT NULL DEFAULT '',
            shell_timeout  BIGINT NOT NULL DEFAULT 60,
            ssrf_whitelist JSONB NOT NULL DEFAULT '[]',
            created_at     TIMESTAMPTZ DEFAULT NOW(),
            UNIQUE(user_id, device_name)
        )",
        "CREATE TABLE IF NOT EXISTS sessions (
            session_id     TEXT PRIMARY KEY,
            user_id        TEXT NOT NULL REFERENCES users(user_id),
            created_at     TIMESTAMPTZ DEFAULT NOW()
        )",
        "CREATE TABLE IF NOT EXISTS messages (
            message_id     TEXT PRIMARY KEY,
            session_id     TEXT NOT NULL REFERENCES sessions(session_id),
            role           TEXT NOT NULL,
            content        TEXT NOT NULL,
            tool_call_id   TEXT,
            tool_name      TEXT,
            tool_arguments TEXT,
            compressed     BOOLEAN DEFAULT FALSE,
            created_at     TIMESTAMPTZ DEFAULT NOW()
        )",
        "CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, created_at)",
        "CREATE TABLE IF NOT EXISTS discord_configs (
            user_id           TEXT PRIMARY KEY REFERENCES users(user_id),
            bot_token         TEXT NOT NULL,
            bot_user_id       TEXT,
            owner_discord_id  TEXT,
            enabled           BOOLEAN DEFAULT TRUE,
            allowed_users     TEXT[] DEFAULT '{}',
            created_at        TIMESTAMPTZ DEFAULT NOW(),
            updated_at        TIMESTAMPTZ DEFAULT NOW()
        )",
        "CREATE TABLE IF NOT EXISTS system_config (
            key            TEXT PRIMARY KEY,
            value          TEXT NOT NULL,
            updated_at     TIMESTAMPTZ DEFAULT NOW()
        )",
        "CREATE TABLE IF NOT EXISTS cron_jobs (
            job_id          TEXT PRIMARY KEY,
            user_id         TEXT NOT NULL REFERENCES users(user_id),
            name            TEXT NOT NULL,
            enabled         BOOLEAN DEFAULT TRUE,
            cron_expr       TEXT,
            every_seconds   INTEGER,
            timezone        TEXT DEFAULT 'UTC',
            message         TEXT NOT NULL,
            channel         TEXT NOT NULL,
            chat_id         TEXT NOT NULL,
            delete_after_run BOOLEAN DEFAULT FALSE,
            deliver         BOOLEAN DEFAULT TRUE,
            next_run_at     TIMESTAMPTZ,
            last_run_at     TIMESTAMPTZ,
            run_count       INTEGER DEFAULT 0,
            created_at      TIMESTAMPTZ DEFAULT NOW()
        )",
        "CREATE TABLE IF NOT EXISTS skills (
            skill_id       TEXT PRIMARY KEY,
            user_id        TEXT NOT NULL REFERENCES users(user_id),
            name           TEXT NOT NULL,
            description    TEXT NOT NULL DEFAULT '',
            always_on      BOOLEAN DEFAULT FALSE,
            skill_path     TEXT NOT NULL,
            created_at     TIMESTAMPTZ DEFAULT NOW(),
            UNIQUE(user_id, name)
        )",
    ];

    for sql in &statements {
        sqlx::query(sql)
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("Failed to execute SQL: {e}\n{sql}"));
    }
}
