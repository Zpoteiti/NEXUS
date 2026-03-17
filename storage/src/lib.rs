use postgres::NoTls;
use r2d2::{Pool, PooledConnection};
use r2d2_postgres::PostgresConnectionManager;
use shared_protocol::{
    LoginSession, LoginUser, MemoryRecord, NodeConnection, NodeRegistration, SessionRecord, Tenant,
    UsageRecord, UsageSummary, UserAccount, UserChannelBinding, UserDevice,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("postgres error: {0}")]
    Pg(#[from] postgres::Error),
    #[error("postgres pool error: {0}")]
    PgPool(#[from] r2d2::Error),
    #[error("username already exists")]
    UsernameConflict,
}

pub type StorageResult<T> = Result<T, StorageError>;

#[derive(Debug, Clone)]
pub struct UserUsageBreakdown {
    pub tenant_id: String,
    pub user_id: String,
    pub requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct AdminDashboardStats {
    pub total_users: u64,
    pub daily_active_users: u64,
    pub usage_by_user: Vec<UserUsageBreakdown>,
}

#[derive(Debug, Clone)]
pub struct UserDashboardStats {
    pub sessions: u64,
    pub memories: u64,
    pub requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct UsageTrendPoint {
    pub day: String,
    pub requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct SessionListItem {
    pub session_id: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct UsageDetailItem {
    pub request_id: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub trait GatewayRepository: Send + Sync {
    fn migrate(&self) -> StorageResult<()>;
    fn upsert_tenant(&self, tenant: &Tenant) -> StorageResult<()>;
    fn list_tenants(&self) -> StorageResult<Vec<Tenant>>;
    fn upsert_user(&self, user: &UserAccount) -> StorageResult<()>;
    fn register_user_with_login(&self, user: &UserAccount, login: &LoginUser) -> StorageResult<()>;
    fn get_login_user_by_username(&self, username: &str) -> StorageResult<Option<LoginUser>>;
    fn save_login_session(&self, session: &LoginSession) -> StorageResult<()>;
    fn get_login_session(&self, session_id: &str) -> StorageResult<Option<LoginSession>>;
    fn upsert_user_device(&self, device: &UserDevice) -> StorageResult<()>;
    fn list_user_devices(&self, tenant_id: &str, user_id: &str) -> StorageResult<Vec<UserDevice>>;
    fn resolve_device_node(
        &self,
        tenant_id: &str,
        user_id: &str,
        alias: &str,
    ) -> StorageResult<Option<String>>;
    fn upsert_channel_binding(&self, binding: &UserChannelBinding) -> StorageResult<()>;
    fn resolve_channel_user(
        &self,
        tenant_id: &str,
        channel_name: &str,
        external_user: &str,
    ) -> StorageResult<Option<UserAccount>>;
    fn save_session(&self, session: &SessionRecord) -> StorageResult<()>;
    fn append_memory(&self, memory: &MemoryRecord) -> StorageResult<()>;
    fn list_memory(
        &self,
        tenant_id: &str,
        user_id: &str,
        session_id: &str,
    ) -> StorageResult<Vec<MemoryRecord>>;
    fn record_usage(&self, usage: &UsageRecord) -> StorageResult<()>;
    fn usage_summary(&self) -> StorageResult<Vec<UsageSummary>>;
    fn admin_dashboard_stats(&self, now_ms: u64) -> StorageResult<AdminDashboardStats>;
    fn user_dashboard_stats(
        &self,
        tenant_id: &str,
        user_id: &str,
    ) -> StorageResult<UserDashboardStats>;
    fn usage_trend(
        &self,
        tenant_id: Option<&str>,
        days: u32,
    ) -> StorageResult<Vec<UsageTrendPoint>>;
    fn list_user_sessions(
        &self,
        tenant_id: &str,
        user_id: &str,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<SessionListItem>>;
    fn list_session_memory(
        &self,
        tenant_id: &str,
        user_id: &str,
        session_id: &str,
    ) -> StorageResult<Vec<MemoryRecord>>;
    fn list_usage_details(
        &self,
        tenant_id: &str,
        user_id: &str,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<UsageDetailItem>>;
    fn upsert_node(
        &self,
        registration: &NodeRegistration,
        connected_at_ms: u64,
    ) -> StorageResult<()>;
    fn touch_node(
        &self,
        node_id: &str,
        seen_ms: u64,
        inflight_requests: usize,
    ) -> StorageResult<()>;
    fn remove_node(&self, node_id: &str) -> StorageResult<()>;
    fn list_nodes(&self) -> StorageResult<Vec<NodeConnection>>;
}

pub struct PostgresRepository {
    pool: Pool<PostgresConnectionManager<NoTls>>,
}

impl PostgresRepository {
    pub fn new(dsn: &str) -> StorageResult<Self> {
        let config = dsn.parse::<postgres::Config>()?;
        let manager = PostgresConnectionManager::new(config, NoTls);
        let pool = Pool::builder().max_size(16).build(manager)?;
        let repo = Self { pool };
        repo.migrate()?;
        Ok(repo)
    }

    fn conn(&self) -> StorageResult<PooledConnection<PostgresConnectionManager<NoTls>>> {
        Ok(self.pool.get()?)
    }
}

impl GatewayRepository for PostgresRepository {
    fn migrate(&self) -> StorageResult<()> {
        let mut conn = self.conn()?;
        conn.batch_execute(
            r#"
            CREATE TABLE IF NOT EXISTS tenants (tenant_id TEXT PRIMARY KEY, display_name TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS users (
                tenant_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                display_name TEXT NOT NULL,
                PRIMARY KEY (tenant_id, user_id)
            );
            CREATE TABLE IF NOT EXISTS login_users (
                username TEXT PRIMARY KEY,
                password_hash TEXT NOT NULL,
                tenant_id TEXT NOT NULL,
                user_id TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS login_sessions (
                session_id TEXT PRIMARY KEY,
                username TEXT NOT NULL,
                tenant_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL
            );
            CREATE TABLE IF NOT EXISTS user_devices (
                tenant_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                node_id TEXT NOT NULL,
                alias TEXT NOT NULL,
                PRIMARY KEY (tenant_id, user_id, alias)
            );
            CREATE TABLE IF NOT EXISTS user_channels (
                tenant_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                channel_name TEXT NOT NULL,
                external_user TEXT NOT NULL,
                credentials_json TEXT NOT NULL,
                PRIMARY KEY (tenant_id, channel_name, external_user)
            );
            CREATE TABLE IF NOT EXISTS sessions (
                tenant_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                title TEXT NOT NULL,
                PRIMARY KEY (tenant_id, session_id)
            );
            CREATE TABLE IF NOT EXISTS memory (
                id BIGSERIAL PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                content TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS usage_records (
                request_id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                model TEXT NOT NULL,
                input_tokens BIGINT NOT NULL,
                output_tokens BIGINT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );
            CREATE TABLE IF NOT EXISTS node_connections (
                node_id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                auth_token TEXT NOT NULL,
                connected_at TIMESTAMPTZ NOT NULL,
                last_seen_at TIMESTAMPTZ NOT NULL,
                inflight_requests BIGINT NOT NULL
            );
            ALTER TABLE usage_records ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ NOT NULL DEFAULT now();
            "#,
        )?;
        Ok(())
    }

    fn upsert_tenant(&self, tenant: &Tenant) -> StorageResult<()> {
        let mut conn = self.conn()?;
        conn.execute(
            "INSERT INTO tenants (tenant_id, display_name) VALUES ($1, $2)
             ON CONFLICT(tenant_id) DO UPDATE SET display_name = EXCLUDED.display_name",
            &[&tenant.tenant_id, &tenant.display_name],
        )?;
        Ok(())
    }

    fn list_tenants(&self) -> StorageResult<Vec<Tenant>> {
        let mut conn = self.conn()?;
        let rows =
            conn.query("SELECT tenant_id, display_name FROM tenants ORDER BY tenant_id", &[])?;
        Ok(rows
            .into_iter()
            .map(|row| Tenant { tenant_id: row.get(0), display_name: row.get(1) })
            .collect())
    }

    fn upsert_user(&self, user: &UserAccount) -> StorageResult<()> {
        let mut conn = self.conn()?;
        conn.execute(
            "INSERT INTO users (tenant_id, user_id, display_name) VALUES ($1, $2, $3)
             ON CONFLICT(tenant_id, user_id) DO UPDATE SET display_name = EXCLUDED.display_name",
            &[&user.tenant_id, &user.user_id, &user.display_name],
        )?;
        Ok(())
    }

    fn register_user_with_login(&self, user: &UserAccount, login: &LoginUser) -> StorageResult<()> {
        let mut conn = self.conn()?;
        let mut tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO users (tenant_id, user_id, display_name) VALUES ($1, $2, $3)
             ON CONFLICT(tenant_id, user_id) DO UPDATE SET display_name = EXCLUDED.display_name",
            &[&user.tenant_id, &user.user_id, &user.display_name],
        )?;
        let changed = tx.execute(
            "INSERT INTO login_users (username, password_hash, tenant_id, user_id)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT DO NOTHING",
            &[&login.username, &login.password_hash, &login.tenant_id, &login.user_id],
        )?;
        if changed == 0 {
            return Err(StorageError::UsernameConflict);
        }
        tx.commit()?;
        Ok(())
    }

    fn get_login_user_by_username(&self, username: &str) -> StorageResult<Option<LoginUser>> {
        let mut conn = self.conn()?;
        let row = conn.query_opt(
            "SELECT username, password_hash, tenant_id, user_id FROM login_users WHERE username = $1",
            &[&username],
        )?;
        Ok(row.map(|row| LoginUser {
            username: row.get(0),
            password_hash: row.get(1),
            tenant_id: row.get(2),
            user_id: row.get(3),
        }))
    }

    fn save_login_session(&self, session: &LoginSession) -> StorageResult<()> {
        let mut conn = self.conn()?;
        conn.execute(
            "INSERT INTO login_sessions (session_id, username, tenant_id, user_id, created_at)
             VALUES ($1, $2, $3, $4, to_timestamp($5::double precision / 1000.0))
             ON CONFLICT(session_id) DO UPDATE SET
               username = EXCLUDED.username,
               tenant_id = EXCLUDED.tenant_id,
               user_id = EXCLUDED.user_id,
               created_at = EXCLUDED.created_at",
            &[
                &session.session_id,
                &session.username,
                &session.tenant_id,
                &session.user_id,
                &(session.created_at_ms as i64),
            ],
        )?;
        Ok(())
    }

    fn get_login_session(&self, session_id: &str) -> StorageResult<Option<LoginSession>> {
        let mut conn = self.conn()?;
        let row = conn.query_opt(
            "SELECT session_id, username, tenant_id, user_id,
                    CAST(EXTRACT(EPOCH FROM created_at) * 1000 AS BIGINT)
             FROM login_sessions
             WHERE session_id = $1",
            &[&session_id],
        )?;
        Ok(row.map(|row| LoginSession {
            session_id: row.get(0),
            username: row.get(1),
            tenant_id: row.get(2),
            user_id: row.get(3),
            created_at_ms: row.get::<_, i64>(4) as u64,
        }))
    }

    fn upsert_user_device(&self, device: &UserDevice) -> StorageResult<()> {
        let mut conn = self.conn()?;
        conn.execute(
            "INSERT INTO user_devices (tenant_id, user_id, node_id, alias) VALUES ($1, $2, $3, $4)
             ON CONFLICT(tenant_id, user_id, alias) DO UPDATE SET node_id = EXCLUDED.node_id",
            &[&device.tenant_id, &device.user_id, &device.node_id, &device.alias],
        )?;
        Ok(())
    }

    fn list_user_devices(&self, tenant_id: &str, user_id: &str) -> StorageResult<Vec<UserDevice>> {
        let mut conn = self.conn()?;
        let rows = conn.query(
            "SELECT tenant_id, user_id, node_id, alias FROM user_devices
             WHERE tenant_id = $1 AND user_id = $2
             ORDER BY alias",
            &[&tenant_id, &user_id],
        )?;
        Ok(rows
            .into_iter()
            .map(|row| UserDevice {
                tenant_id: row.get(0),
                user_id: row.get(1),
                node_id: row.get(2),
                alias: row.get(3),
            })
            .collect())
    }

    fn resolve_device_node(
        &self,
        tenant_id: &str,
        user_id: &str,
        alias: &str,
    ) -> StorageResult<Option<String>> {
        let mut conn = self.conn()?;
        let row = conn.query_opt(
            "SELECT node_id FROM user_devices WHERE tenant_id = $1 AND user_id = $2 AND alias = $3",
            &[&tenant_id, &user_id, &alias],
        )?;
        Ok(row.map(|row| row.get(0)))
    }

    fn upsert_channel_binding(&self, binding: &UserChannelBinding) -> StorageResult<()> {
        let mut conn = self.conn()?;
        conn.execute(
            "INSERT INTO user_channels (tenant_id, user_id, channel_name, external_user, credentials_json)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT(tenant_id, channel_name, external_user) DO UPDATE
             SET user_id = EXCLUDED.user_id, credentials_json = EXCLUDED.credentials_json",
            &[
                &binding.tenant_id,
                &binding.user_id,
                &binding.channel_name,
                &binding.external_user,
                &binding.credentials_json,
            ],
        )?;
        Ok(())
    }

    fn resolve_channel_user(
        &self,
        tenant_id: &str,
        channel_name: &str,
        external_user: &str,
    ) -> StorageResult<Option<UserAccount>> {
        let mut conn = self.conn()?;
        let row = conn.query_opt(
            "SELECT u.tenant_id, u.user_id, u.display_name
             FROM users u
             JOIN user_channels c ON c.tenant_id = u.tenant_id AND c.user_id = u.user_id
             WHERE c.tenant_id = $1 AND c.channel_name = $2 AND c.external_user = $3
             LIMIT 1",
            &[&tenant_id, &channel_name, &external_user],
        )?;
        Ok(row.map(|row| UserAccount {
            tenant_id: row.get(0),
            user_id: row.get(1),
            display_name: row.get(2),
        }))
    }

    fn save_session(&self, session: &SessionRecord) -> StorageResult<()> {
        let mut conn = self.conn()?;
        conn.execute(
            "INSERT INTO sessions (tenant_id, user_id, session_id, title) VALUES ($1, $2, $3, $4)
             ON CONFLICT(tenant_id, session_id) DO UPDATE
             SET user_id = EXCLUDED.user_id, title = EXCLUDED.title",
            &[&session.tenant_id, &session.user_id, &session.session_id, &session.title],
        )?;
        Ok(())
    }

    fn append_memory(&self, memory: &MemoryRecord) -> StorageResult<()> {
        let mut conn = self.conn()?;
        conn.execute(
            "INSERT INTO memory (tenant_id, user_id, session_id, content) VALUES ($1, $2, $3, $4)",
            &[&memory.tenant_id, &memory.user_id, &memory.session_id, &memory.content],
        )?;
        Ok(())
    }

    fn list_memory(
        &self,
        tenant_id: &str,
        user_id: &str,
        session_id: &str,
    ) -> StorageResult<Vec<MemoryRecord>> {
        let mut conn = self.conn()?;
        let rows = conn.query(
            "SELECT tenant_id, user_id, session_id, content
             FROM memory
             WHERE tenant_id = $1 AND user_id = $2 AND session_id = $3
             ORDER BY id ASC",
            &[&tenant_id, &user_id, &session_id],
        )?;
        Ok(rows
            .into_iter()
            .map(|row| MemoryRecord {
                tenant_id: row.get(0),
                user_id: row.get(1),
                session_id: row.get(2),
                content: row.get(3),
            })
            .collect())
    }

    fn record_usage(&self, usage: &UsageRecord) -> StorageResult<()> {
        let mut conn = self.conn()?;
        conn.execute(
            "INSERT INTO usage_records (request_id, tenant_id, user_id, model, input_tokens, output_tokens, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, now())
             ON CONFLICT(request_id) DO UPDATE SET
                tenant_id = EXCLUDED.tenant_id,
                user_id = EXCLUDED.user_id,
                model = EXCLUDED.model,
                input_tokens = EXCLUDED.input_tokens,
                output_tokens = EXCLUDED.output_tokens,
                created_at = now()",
            &[
                &usage.request_id,
                &usage.tenant_id,
                &usage.user_id,
                &usage.model,
                &(usage.input_tokens as i64),
                &(usage.output_tokens as i64),
            ],
        )?;
        Ok(())
    }

    fn usage_summary(&self) -> StorageResult<Vec<UsageSummary>> {
        let mut conn = self.conn()?;
        let rows = conn.query(
            "SELECT tenant_id, COUNT(*), COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0)
             FROM usage_records
             GROUP BY tenant_id
             ORDER BY tenant_id",
            &[],
        )?;
        Ok(rows
            .into_iter()
            .map(|row| UsageSummary {
                tenant_id: row.get(0),
                requests: row.get::<_, i64>(1) as u64,
                total_input_tokens: row.get::<_, i64>(2) as u64,
                total_output_tokens: row.get::<_, i64>(3) as u64,
            })
            .collect())
    }

    fn admin_dashboard_stats(&self, now_ms: u64) -> StorageResult<AdminDashboardStats> {
        let mut conn = self.conn()?;
        let total_users =
            conn.query_one("SELECT COUNT(*) FROM users", &[])?.get::<_, i64>(0) as u64;
        let day_ago_ms = now_ms.saturating_sub(86_400_000) as i64;
        let daily_active_users = conn
            .query_one(
                "SELECT COUNT(*) FROM (
                    SELECT DISTINCT tenant_id, user_id
                    FROM login_sessions
                    WHERE created_at >= to_timestamp($1::double precision / 1000.0)
                ) AS active_users",
                &[&day_ago_ms],
            )?
            .get::<_, i64>(0) as u64;

        let rows = conn.query(
            "SELECT tenant_id, user_id, COUNT(*), COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0)
             FROM usage_records
             GROUP BY tenant_id, user_id
             ORDER BY tenant_id, user_id",
            &[],
        )?;

        Ok(AdminDashboardStats {
            total_users,
            daily_active_users,
            usage_by_user: rows
                .into_iter()
                .map(|row| UserUsageBreakdown {
                    tenant_id: row.get(0),
                    user_id: row.get(1),
                    requests: row.get::<_, i64>(2) as u64,
                    total_input_tokens: row.get::<_, i64>(3) as u64,
                    total_output_tokens: row.get::<_, i64>(4) as u64,
                })
                .collect(),
        })
    }

    fn user_dashboard_stats(
        &self,
        tenant_id: &str,
        user_id: &str,
    ) -> StorageResult<UserDashboardStats> {
        let mut conn = self.conn()?;
        let sessions = conn
            .query_one(
                "SELECT COUNT(*) FROM sessions WHERE tenant_id = $1 AND user_id = $2",
                &[&tenant_id, &user_id],
            )?
            .get::<_, i64>(0) as u64;
        let memories = conn
            .query_one(
                "SELECT COUNT(*) FROM memory WHERE tenant_id = $1 AND user_id = $2",
                &[&tenant_id, &user_id],
            )?
            .get::<_, i64>(0) as u64;
        let usage_row = conn.query_one(
            "SELECT COUNT(*), COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0)
             FROM usage_records
             WHERE tenant_id = $1 AND user_id = $2",
            &[&tenant_id, &user_id],
        )?;
        Ok(UserDashboardStats {
            sessions,
            memories,
            requests: usage_row.get::<_, i64>(0) as u64,
            total_input_tokens: usage_row.get::<_, i64>(1) as u64,
            total_output_tokens: usage_row.get::<_, i64>(2) as u64,
        })
    }

    fn usage_trend(
        &self,
        tenant_id: Option<&str>,
        days: u32,
    ) -> StorageResult<Vec<UsageTrendPoint>> {
        let mut conn = self.conn()?;
        let days = days.max(1) as i64;
        let rows = if let Some(tenant_id) = tenant_id {
            conn.query(
                "SELECT to_char(date_trunc('day', created_at), 'YYYY-MM-DD') AS day,
                        COUNT(*),
                        COALESCE(SUM(input_tokens), 0),
                        COALESCE(SUM(output_tokens), 0)
                 FROM usage_records
                 WHERE tenant_id = $1
                   AND created_at >= now() - ($2 * interval '1 day')
                 GROUP BY day
                 ORDER BY day",
                &[&tenant_id, &days],
            )?
        } else {
            conn.query(
                "SELECT to_char(date_trunc('day', created_at), 'YYYY-MM-DD') AS day,
                        COUNT(*),
                        COALESCE(SUM(input_tokens), 0),
                        COALESCE(SUM(output_tokens), 0)
                 FROM usage_records
                 WHERE created_at >= now() - ($1 * interval '1 day')
                 GROUP BY day
                 ORDER BY day",
                &[&days],
            )?
        };
        Ok(rows
            .into_iter()
            .map(|row| UsageTrendPoint {
                day: row.get(0),
                requests: row.get::<_, i64>(1) as u64,
                total_input_tokens: row.get::<_, i64>(2) as u64,
                total_output_tokens: row.get::<_, i64>(3) as u64,
            })
            .collect())
    }

    fn list_user_sessions(
        &self,
        tenant_id: &str,
        user_id: &str,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<SessionListItem>> {
        let mut conn = self.conn()?;
        let rows = conn.query(
            "SELECT session_id, title
             FROM sessions
             WHERE tenant_id = $1 AND user_id = $2
             ORDER BY session_id DESC
             LIMIT $3 OFFSET $4",
            &[&tenant_id, &user_id, &(limit as i64), &(offset as i64)],
        )?;
        Ok(rows
            .into_iter()
            .map(|row| SessionListItem { session_id: row.get(0), title: row.get(1) })
            .collect())
    }

    fn list_session_memory(
        &self,
        tenant_id: &str,
        user_id: &str,
        session_id: &str,
    ) -> StorageResult<Vec<MemoryRecord>> {
        self.list_memory(tenant_id, user_id, session_id)
    }

    fn list_usage_details(
        &self,
        tenant_id: &str,
        user_id: &str,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<UsageDetailItem>> {
        let mut conn = self.conn()?;
        let rows = conn.query(
            "SELECT request_id, model, input_tokens, output_tokens
             FROM usage_records
             WHERE tenant_id = $1 AND user_id = $2
             ORDER BY created_at DESC
             LIMIT $3 OFFSET $4",
            &[&tenant_id, &user_id, &(limit as i64), &(offset as i64)],
        )?;
        Ok(rows
            .into_iter()
            .map(|row| UsageDetailItem {
                request_id: row.get(0),
                model: row.get(1),
                input_tokens: row.get::<_, i64>(2) as u64,
                output_tokens: row.get::<_, i64>(3) as u64,
            })
            .collect())
    }

    fn upsert_node(
        &self,
        registration: &NodeRegistration,
        connected_at_ms: u64,
    ) -> StorageResult<()> {
        let mut conn = self.conn()?;
        conn.execute(
            "INSERT INTO node_connections
             (node_id, tenant_id, user_id, auth_token, connected_at, last_seen_at, inflight_requests)
             VALUES ($1, $2, $3, $4, to_timestamp($5::double precision / 1000.0), to_timestamp($5::double precision / 1000.0), $6)
             ON CONFLICT(node_id) DO UPDATE SET
                 tenant_id = EXCLUDED.tenant_id,
                 user_id = EXCLUDED.user_id,
                 auth_token = EXCLUDED.auth_token,
                 connected_at = EXCLUDED.connected_at,
                 last_seen_at = EXCLUDED.last_seen_at,
                 inflight_requests = EXCLUDED.inflight_requests",
            &[
                &registration.node_id,
                &registration.tenant_id,
                &registration.user_id,
                &registration.auth_token,
                &(connected_at_ms as i64),
                &0_i64,
            ],
        )?;
        Ok(())
    }

    fn touch_node(
        &self,
        node_id: &str,
        seen_ms: u64,
        inflight_requests: usize,
    ) -> StorageResult<()> {
        let mut conn = self.conn()?;
        conn.execute(
            "UPDATE node_connections
             SET last_seen_at = to_timestamp($2::double precision / 1000.0), inflight_requests = $3
             WHERE node_id = $1",
            &[&node_id, &(seen_ms as i64), &(inflight_requests as i64)],
        )?;
        Ok(())
    }

    fn remove_node(&self, node_id: &str) -> StorageResult<()> {
        let mut conn = self.conn()?;
        conn.execute("DELETE FROM node_connections WHERE node_id = $1", &[&node_id])?;
        Ok(())
    }

    fn list_nodes(&self) -> StorageResult<Vec<NodeConnection>> {
        let mut conn = self.conn()?;
        let rows = conn.query(
            "SELECT tenant_id, user_id, node_id,
                    CAST(EXTRACT(EPOCH FROM connected_at) * 1000 AS BIGINT),
                    CAST(EXTRACT(EPOCH FROM last_seen_at) * 1000 AS BIGINT),
                    inflight_requests
             FROM node_connections
             ORDER BY tenant_id, user_id, node_id",
            &[],
        )?;
        Ok(rows
            .into_iter()
            .map(|row| NodeConnection {
                tenant_id: row.get(0),
                user_id: row.get(1),
                node_id: row.get(2),
                connected_at_ms: row.get::<_, i64>(3) as u64,
                last_seen_ms: row.get::<_, i64>(4) as u64,
                inflight_requests: row.get::<_, i64>(5) as usize,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::{GatewayRepository, PostgresRepository};
    use shared_protocol::{LoginUser, UserAccount};

    #[test]
    fn postgres_registration_roundtrip_when_env_present() {
        let Some(dsn) = env::var("NEXUS_TEST_POSTGRES_DSN").ok() else {
            return;
        };
        let repo = PostgresRepository::new(&dsn).expect("pg repo");
        let user = UserAccount {
            tenant_id: "tenant-pg".to_owned(),
            user_id: "user-pg".to_owned(),
            display_name: "PG User".to_owned(),
        };
        let login = LoginUser {
            username: format!("pg-{}", std::process::id()),
            password_hash: "hash".to_owned(),
            tenant_id: "tenant-pg".to_owned(),
            user_id: "user-pg".to_owned(),
        };
        repo.register_user_with_login(&user, &login).expect("register");
        let stored =
            repo.get_login_user_by_username(&login.username).expect("lookup").expect("present");
        assert_eq!(stored.user_id, "user-pg");
    }
}
