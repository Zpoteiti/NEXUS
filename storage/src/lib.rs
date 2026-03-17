use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use postgres::{Client, NoTls};
use rusqlite::{params, Connection};
use shared_protocol::{
    LoginSession, LoginUser, MemoryRecord, NodeConnection, NodeRegistration, SessionRecord, Tenant,
    UsageRecord, UsageSummary, UserAccount, UserChannelBinding, UserDevice,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("storage lock poisoned")]
    LockPoisoned,
    #[error("sqlite error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("postgres error: {0}")]
    Pg(#[from] postgres::Error),
    #[error("username already exists")]
    UsernameConflict,
}

pub type StorageResult<T> = Result<T, StorageError>;

pub trait GatewayRepository: Send + Sync {
    fn migrate(&self) -> StorageResult<()>;
    fn upsert_tenant(&self, tenant: &Tenant) -> StorageResult<()>;
    fn list_tenants(&self) -> StorageResult<Vec<Tenant>>;
    fn upsert_user(&self, user: &UserAccount) -> StorageResult<()>;
    fn create_login_user(&self, user: &LoginUser) -> StorageResult<()>;
    fn authenticate_login_user(
        &self,
        username: &str,
        password_hash: &str,
    ) -> StorageResult<Option<LoginUser>>;
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

pub trait RepositoryFactory {
    fn sqlite(path: impl AsRef<Path>) -> StorageResult<Box<dyn GatewayRepository>>;
    fn postgres(dsn: &str) -> StorageResult<Box<dyn GatewayRepository>>;
}

pub struct StorageFactory;

impl RepositoryFactory for StorageFactory {
    fn sqlite(path: impl AsRef<Path>) -> StorageResult<Box<dyn GatewayRepository>> {
        Ok(Box::new(SqliteRepository::new(path)?))
    }

    fn postgres(dsn: &str) -> StorageResult<Box<dyn GatewayRepository>> {
        Ok(Box::new(PostgresRepository::new(dsn)?))
    }
}

pub struct SqliteRepository {
    conn: Mutex<Connection>,
}

impl SqliteRepository {
    pub fn new(path: impl AsRef<Path>) -> StorageResult<Self> {
        let conn = Connection::open(path)?;
        let repo = Self { conn: Mutex::new(conn) };
        repo.migrate()?;
        Ok(repo)
    }

    pub fn in_memory() -> StorageResult<Self> {
        let conn = Connection::open_in_memory()?;
        let repo = Self { conn: Mutex::new(conn) };
        repo.migrate()?;
        Ok(repo)
    }

    fn conn(&self) -> StorageResult<MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| StorageError::LockPoisoned)
    }
}

pub struct PostgresRepository {
    client: Mutex<Client>,
}

impl PostgresRepository {
    pub fn new(dsn: &str) -> StorageResult<Self> {
        let client = Client::connect(dsn, NoTls)?;
        let repo = Self { client: Mutex::new(client) };
        repo.migrate()?;
        Ok(repo)
    }

    fn client(&self) -> StorageResult<MutexGuard<'_, Client>> {
        self.client.lock().map_err(|_| StorageError::LockPoisoned)
    }
}

fn migrate_sql(schema: &str, conn: &Connection) -> StorageResult<()> {
    conn.execute_batch(schema)?;
    Ok(())
}

const BASE_SCHEMA_SQLITE: &str = r#"
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
    created_at_ms INTEGER NOT NULL
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
    id INTEGER PRIMARY KEY AUTOINCREMENT,
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
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS node_connections (
    node_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    auth_token TEXT NOT NULL,
    connected_at_ms INTEGER NOT NULL,
    last_seen_ms INTEGER NOT NULL,
    inflight_requests INTEGER NOT NULL
);
"#;

impl GatewayRepository for SqliteRepository {
    fn migrate(&self) -> StorageResult<()> {
        let conn = self.conn()?;
        migrate_sql(BASE_SCHEMA_SQLITE, &conn)
    }

    fn upsert_tenant(&self, tenant: &Tenant) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute("INSERT INTO tenants (tenant_id, display_name) VALUES (?1, ?2) ON CONFLICT(tenant_id) DO UPDATE SET display_name = excluded.display_name", params![tenant.tenant_id, tenant.display_name])?;
        Ok(())
    }

    fn list_tenants(&self) -> StorageResult<Vec<Tenant>> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT tenant_id, display_name FROM tenants ORDER BY tenant_id")?;
        let rows =
            stmt.query_map([], |r| Ok(Tenant { tenant_id: r.get(0)?, display_name: r.get(1)? }))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(StorageError::from)
    }

    fn upsert_user(&self, user: &UserAccount) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute("INSERT INTO users (tenant_id, user_id, display_name) VALUES (?1, ?2, ?3) ON CONFLICT(tenant_id, user_id) DO UPDATE SET display_name = excluded.display_name", params![user.tenant_id, user.user_id, user.display_name])?;
        Ok(())
    }

    fn create_login_user(&self, user: &LoginUser) -> StorageResult<()> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "INSERT OR IGNORE INTO login_users (username, password_hash, tenant_id, user_id) VALUES (?1, ?2, ?3, ?4)",
            params![user.username, user.password_hash, user.tenant_id, user.user_id],
        )?;
        if changed == 0 {
            return Err(StorageError::UsernameConflict);
        }
        Ok(())
    }

    fn authenticate_login_user(
        &self,
        username: &str,
        password_hash: &str,
    ) -> StorageResult<Option<LoginUser>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT username, password_hash, tenant_id, user_id FROM login_users WHERE username = ?1 AND password_hash = ?2 LIMIT 1")?;
        let mut rows = stmt.query(params![username, password_hash])?;
        if let Some(r) = rows.next()? {
            return Ok(Some(LoginUser {
                username: r.get(0)?,
                password_hash: r.get(1)?,
                tenant_id: r.get(2)?,
                user_id: r.get(3)?,
            }));
        }
        Ok(None)
    }

    fn save_login_session(&self, session: &LoginSession) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute("INSERT OR REPLACE INTO login_sessions (session_id, username, tenant_id, user_id, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)", params![session.session_id, session.username, session.tenant_id, session.user_id, session.created_at_ms as i64])?;
        Ok(())
    }

    fn get_login_session(&self, session_id: &str) -> StorageResult<Option<LoginSession>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT session_id, username, tenant_id, user_id, created_at_ms FROM login_sessions WHERE session_id = ?1 LIMIT 1")?;
        let mut rows = stmt.query(params![session_id])?;
        if let Some(r) = rows.next()? {
            return Ok(Some(LoginSession {
                session_id: r.get(0)?,
                username: r.get(1)?,
                tenant_id: r.get(2)?,
                user_id: r.get(3)?,
                created_at_ms: r.get::<_, i64>(4)? as u64,
            }));
        }
        Ok(None)
    }

    fn upsert_user_device(&self, device: &UserDevice) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute("INSERT INTO user_devices (tenant_id, user_id, node_id, alias) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(tenant_id, user_id, alias) DO UPDATE SET node_id = excluded.node_id", params![device.tenant_id, device.user_id, device.node_id, device.alias])?;
        Ok(())
    }

    fn list_user_devices(&self, tenant_id: &str, user_id: &str) -> StorageResult<Vec<UserDevice>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT tenant_id, user_id, node_id, alias FROM user_devices WHERE tenant_id = ?1 AND user_id = ?2 ORDER BY alias")?;
        let rows = stmt.query_map(params![tenant_id, user_id], |r| {
            Ok(UserDevice {
                tenant_id: r.get(0)?,
                user_id: r.get(1)?,
                node_id: r.get(2)?,
                alias: r.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(StorageError::from)
    }

    fn resolve_device_node(
        &self,
        tenant_id: &str,
        user_id: &str,
        alias: &str,
    ) -> StorageResult<Option<String>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT node_id FROM user_devices WHERE tenant_id = ?1 AND user_id = ?2 AND alias = ?3 LIMIT 1")?;
        let mut rows = stmt.query(params![tenant_id, user_id, alias])?;
        if let Some(r) = rows.next()? {
            return Ok(Some(r.get(0)?));
        }
        Ok(None)
    }

    fn upsert_channel_binding(&self, binding: &UserChannelBinding) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute("INSERT INTO user_channels (tenant_id, user_id, channel_name, external_user, credentials_json) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(tenant_id, channel_name, external_user) DO UPDATE SET user_id = excluded.user_id, credentials_json = excluded.credentials_json", params![binding.tenant_id, binding.user_id, binding.channel_name, binding.external_user, binding.credentials_json])?;
        Ok(())
    }

    fn resolve_channel_user(
        &self,
        tenant_id: &str,
        channel_name: &str,
        external_user: &str,
    ) -> StorageResult<Option<UserAccount>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT u.tenant_id, u.user_id, u.display_name FROM users u JOIN user_channels c ON c.tenant_id = u.tenant_id AND c.user_id = u.user_id WHERE c.tenant_id = ?1 AND c.channel_name = ?2 AND c.external_user = ?3 LIMIT 1")?;
        let mut rows = stmt.query(params![tenant_id, channel_name, external_user])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(UserAccount {
                tenant_id: row.get(0)?,
                user_id: row.get(1)?,
                display_name: row.get(2)?,
            }));
        }
        Ok(None)
    }

    fn save_session(&self, session: &SessionRecord) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute("INSERT INTO sessions (tenant_id, user_id, session_id, title) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(tenant_id, session_id) DO UPDATE SET user_id = excluded.user_id, title = excluded.title", params![session.tenant_id, session.user_id, session.session_id, session.title])?;
        Ok(())
    }

    fn append_memory(&self, memory: &MemoryRecord) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO memory (tenant_id, user_id, session_id, content) VALUES (?1, ?2, ?3, ?4)",
            params![memory.tenant_id, memory.user_id, memory.session_id, memory.content],
        )?;
        Ok(())
    }

    fn list_memory(
        &self,
        tenant_id: &str,
        user_id: &str,
        session_id: &str,
    ) -> StorageResult<Vec<MemoryRecord>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT tenant_id, user_id, session_id, content FROM memory WHERE tenant_id = ?1 AND user_id = ?2 AND session_id = ?3 ORDER BY id ASC")?;
        let rows = stmt.query_map(params![tenant_id, user_id, session_id], |r| {
            Ok(MemoryRecord {
                tenant_id: r.get(0)?,
                user_id: r.get(1)?,
                session_id: r.get(2)?,
                content: r.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(StorageError::from)
    }

    fn record_usage(&self, usage: &UsageRecord) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute("INSERT OR REPLACE INTO usage_records (request_id, tenant_id, user_id, model, input_tokens, output_tokens) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![usage.request_id, usage.tenant_id, usage.user_id, usage.model, usage.input_tokens as i64, usage.output_tokens as i64])?;
        Ok(())
    }

    fn usage_summary(&self) -> StorageResult<Vec<UsageSummary>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT tenant_id, COUNT(*), COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0) FROM usage_records GROUP BY tenant_id ORDER BY tenant_id")?;
        let rows = stmt.query_map([], |r| {
            Ok(UsageSummary {
                tenant_id: r.get(0)?,
                requests: r.get::<_, i64>(1)? as u64,
                total_input_tokens: r.get::<_, i64>(2)? as u64,
                total_output_tokens: r.get::<_, i64>(3)? as u64,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(StorageError::from)
    }

    fn upsert_node(
        &self,
        registration: &NodeRegistration,
        connected_at_ms: u64,
    ) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute("INSERT OR REPLACE INTO node_connections (node_id, tenant_id, user_id, auth_token, connected_at_ms, last_seen_ms, inflight_requests) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)", params![registration.node_id, registration.tenant_id, registration.user_id, registration.auth_token, connected_at_ms as i64, connected_at_ms as i64, 0_i64])?;
        Ok(())
    }

    fn touch_node(
        &self,
        node_id: &str,
        seen_ms: u64,
        inflight_requests: usize,
    ) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute("UPDATE node_connections SET last_seen_ms = ?2, inflight_requests = ?3 WHERE node_id = ?1", params![node_id, seen_ms as i64, inflight_requests as i64])?;
        Ok(())
    }

    fn remove_node(&self, node_id: &str) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM node_connections WHERE node_id = ?1", params![node_id])?;
        Ok(())
    }

    fn list_nodes(&self) -> StorageResult<Vec<NodeConnection>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT tenant_id, user_id, node_id, connected_at_ms, last_seen_ms, inflight_requests FROM node_connections ORDER BY tenant_id, user_id, node_id")?;
        let rows = stmt.query_map([], |r| {
            Ok(NodeConnection {
                tenant_id: r.get(0)?,
                user_id: r.get(1)?,
                node_id: r.get(2)?,
                connected_at_ms: r.get::<_, i64>(3)? as u64,
                last_seen_ms: r.get::<_, i64>(4)? as u64,
                inflight_requests: r.get::<_, i64>(5)? as usize,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(StorageError::from)
    }
}

impl GatewayRepository for PostgresRepository {
    fn migrate(&self) -> StorageResult<()> {
        let mut c = self.client()?;
        c.batch_execute(
            r#"
            CREATE TABLE IF NOT EXISTS tenants (tenant_id TEXT PRIMARY KEY, display_name TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS users (tenant_id TEXT NOT NULL, user_id TEXT NOT NULL, display_name TEXT NOT NULL, PRIMARY KEY (tenant_id, user_id));
            CREATE TABLE IF NOT EXISTS login_users (username TEXT PRIMARY KEY, password_hash TEXT NOT NULL, tenant_id TEXT NOT NULL, user_id TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS login_sessions (session_id TEXT PRIMARY KEY, username TEXT NOT NULL, tenant_id TEXT NOT NULL, user_id TEXT NOT NULL, created_at_ms BIGINT NOT NULL);
            CREATE TABLE IF NOT EXISTS user_devices (tenant_id TEXT NOT NULL, user_id TEXT NOT NULL, node_id TEXT NOT NULL, alias TEXT NOT NULL, PRIMARY KEY (tenant_id, user_id, alias));
            CREATE TABLE IF NOT EXISTS user_channels (tenant_id TEXT NOT NULL, user_id TEXT NOT NULL, channel_name TEXT NOT NULL, external_user TEXT NOT NULL, credentials_json TEXT NOT NULL, PRIMARY KEY (tenant_id, channel_name, external_user));
            CREATE TABLE IF NOT EXISTS sessions (tenant_id TEXT NOT NULL, user_id TEXT NOT NULL, session_id TEXT NOT NULL, title TEXT NOT NULL, PRIMARY KEY (tenant_id, session_id));
            CREATE TABLE IF NOT EXISTS memory (id BIGSERIAL PRIMARY KEY, tenant_id TEXT NOT NULL, user_id TEXT NOT NULL, session_id TEXT NOT NULL, content TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS usage_records (request_id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, user_id TEXT NOT NULL, model TEXT NOT NULL, input_tokens BIGINT NOT NULL, output_tokens BIGINT NOT NULL);
            CREATE TABLE IF NOT EXISTS node_connections (node_id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, user_id TEXT NOT NULL, auth_token TEXT NOT NULL, connected_at_ms BIGINT NOT NULL, last_seen_ms BIGINT NOT NULL, inflight_requests BIGINT NOT NULL);
            "#,
        )?;
        Ok(())
    }
    fn upsert_tenant(&self, tenant: &Tenant) -> StorageResult<()> {
        let mut c = self.client()?;
        c.execute("INSERT INTO tenants (tenant_id, display_name) VALUES ($1,$2) ON CONFLICT(tenant_id) DO UPDATE SET display_name=EXCLUDED.display_name", &[&tenant.tenant_id,&tenant.display_name])?;
        Ok(())
    }
    fn list_tenants(&self) -> StorageResult<Vec<Tenant>> {
        let mut c = self.client()?;
        Ok(c.query("SELECT tenant_id, display_name FROM tenants ORDER BY tenant_id", &[])?
            .into_iter()
            .map(|r| Tenant { tenant_id: r.get(0), display_name: r.get(1) })
            .collect())
    }
    fn upsert_user(&self, user: &UserAccount) -> StorageResult<()> {
        let mut c = self.client()?;
        c.execute("INSERT INTO users (tenant_id,user_id,display_name) VALUES ($1,$2,$3) ON CONFLICT(tenant_id,user_id) DO UPDATE SET display_name=EXCLUDED.display_name", &[&user.tenant_id,&user.user_id,&user.display_name])?;
        Ok(())
    }
    fn create_login_user(&self, user: &LoginUser) -> StorageResult<()> {
        let mut c = self.client()?;
        let changed=c.execute("INSERT INTO login_users (username,password_hash,tenant_id,user_id) VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING", &[&user.username,&user.password_hash,&user.tenant_id,&user.user_id])?;
        if changed == 0 {
            return Err(StorageError::UsernameConflict);
        }
        Ok(())
    }
    fn authenticate_login_user(
        &self,
        username: &str,
        password_hash: &str,
    ) -> StorageResult<Option<LoginUser>> {
        let mut c = self.client()?;
        Ok(c.query_opt("SELECT username,password_hash,tenant_id,user_id FROM login_users WHERE username=$1 AND password_hash=$2", &[&username,&password_hash])?.map(|r| LoginUser{username:r.get(0),password_hash:r.get(1),tenant_id:r.get(2),user_id:r.get(3)}))
    }
    fn save_login_session(&self, session: &LoginSession) -> StorageResult<()> {
        let mut c = self.client()?;
        c.execute("INSERT INTO login_sessions (session_id,username,tenant_id,user_id,created_at_ms) VALUES ($1,$2,$3,$4,$5) ON CONFLICT(session_id) DO UPDATE SET username=EXCLUDED.username,tenant_id=EXCLUDED.tenant_id,user_id=EXCLUDED.user_id,created_at_ms=EXCLUDED.created_at_ms", &[&session.session_id,&session.username,&session.tenant_id,&session.user_id,&(session.created_at_ms as i64)])?;
        Ok(())
    }
    fn get_login_session(&self, session_id: &str) -> StorageResult<Option<LoginSession>> {
        let mut c = self.client()?;
        Ok(c.query_opt("SELECT session_id,username,tenant_id,user_id,created_at_ms FROM login_sessions WHERE session_id=$1", &[&session_id])?.map(|r| LoginSession{session_id:r.get(0),username:r.get(1),tenant_id:r.get(2),user_id:r.get(3),created_at_ms:r.get::<_,i64>(4) as u64}))
    }
    fn upsert_user_device(&self, device: &UserDevice) -> StorageResult<()> {
        let mut c = self.client()?;
        c.execute("INSERT INTO user_devices (tenant_id,user_id,node_id,alias) VALUES ($1,$2,$3,$4) ON CONFLICT(tenant_id,user_id,alias) DO UPDATE SET node_id=EXCLUDED.node_id", &[&device.tenant_id,&device.user_id,&device.node_id,&device.alias])?;
        Ok(())
    }
    fn list_user_devices(&self, tenant_id: &str, user_id: &str) -> StorageResult<Vec<UserDevice>> {
        let mut c = self.client()?;
        Ok(c.query("SELECT tenant_id,user_id,node_id,alias FROM user_devices WHERE tenant_id=$1 AND user_id=$2 ORDER BY alias", &[&tenant_id,&user_id])?.into_iter().map(|r| UserDevice{tenant_id:r.get(0),user_id:r.get(1),node_id:r.get(2),alias:r.get(3)}).collect())
    }
    fn resolve_device_node(
        &self,
        tenant_id: &str,
        user_id: &str,
        alias: &str,
    ) -> StorageResult<Option<String>> {
        let mut c = self.client()?;
        Ok(c.query_opt(
            "SELECT node_id FROM user_devices WHERE tenant_id=$1 AND user_id=$2 AND alias=$3",
            &[&tenant_id, &user_id, &alias],
        )?
        .map(|r| r.get(0)))
    }
    fn upsert_channel_binding(&self, binding: &UserChannelBinding) -> StorageResult<()> {
        let mut c = self.client()?;
        c.execute("INSERT INTO user_channels (tenant_id,user_id,channel_name,external_user,credentials_json) VALUES ($1,$2,$3,$4,$5) ON CONFLICT(tenant_id,channel_name,external_user) DO UPDATE SET user_id=EXCLUDED.user_id, credentials_json=EXCLUDED.credentials_json", &[&binding.tenant_id,&binding.user_id,&binding.channel_name,&binding.external_user,&binding.credentials_json])?;
        Ok(())
    }
    fn resolve_channel_user(
        &self,
        tenant_id: &str,
        channel_name: &str,
        external_user: &str,
    ) -> StorageResult<Option<UserAccount>> {
        let mut c = self.client()?;
        Ok(c.query_opt("SELECT u.tenant_id,u.user_id,u.display_name FROM users u JOIN user_channels c ON c.tenant_id=u.tenant_id AND c.user_id=u.user_id WHERE c.tenant_id=$1 AND c.channel_name=$2 AND c.external_user=$3 LIMIT 1", &[&tenant_id,&channel_name,&external_user])?.map(|r| UserAccount{tenant_id:r.get(0),user_id:r.get(1),display_name:r.get(2)}))
    }
    fn save_session(&self, session: &SessionRecord) -> StorageResult<()> {
        let mut c = self.client()?;
        c.execute("INSERT INTO sessions (tenant_id,user_id,session_id,title) VALUES ($1,$2,$3,$4) ON CONFLICT(tenant_id,session_id) DO UPDATE SET user_id=EXCLUDED.user_id,title=EXCLUDED.title", &[&session.tenant_id,&session.user_id,&session.session_id,&session.title])?;
        Ok(())
    }
    fn append_memory(&self, memory: &MemoryRecord) -> StorageResult<()> {
        let mut c = self.client()?;
        c.execute(
            "INSERT INTO memory (tenant_id,user_id,session_id,content) VALUES ($1,$2,$3,$4)",
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
        let mut c = self.client()?;
        Ok(c.query("SELECT tenant_id,user_id,session_id,content FROM memory WHERE tenant_id=$1 AND user_id=$2 AND session_id=$3 ORDER BY id", &[&tenant_id,&user_id,&session_id])?.into_iter().map(|r| MemoryRecord{tenant_id:r.get(0),user_id:r.get(1),session_id:r.get(2),content:r.get(3)}).collect())
    }
    fn record_usage(&self, usage: &UsageRecord) -> StorageResult<()> {
        let mut c = self.client()?;
        c.execute("INSERT INTO usage_records (request_id,tenant_id,user_id,model,input_tokens,output_tokens) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT(request_id) DO UPDATE SET tenant_id=EXCLUDED.tenant_id,user_id=EXCLUDED.user_id,model=EXCLUDED.model,input_tokens=EXCLUDED.input_tokens,output_tokens=EXCLUDED.output_tokens", &[&usage.request_id,&usage.tenant_id,&usage.user_id,&usage.model,&(usage.input_tokens as i64),&(usage.output_tokens as i64)])?;
        Ok(())
    }
    fn usage_summary(&self) -> StorageResult<Vec<UsageSummary>> {
        let mut c = self.client()?;
        Ok(c.query("SELECT tenant_id, COUNT(*), COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0) FROM usage_records GROUP BY tenant_id ORDER BY tenant_id", &[])?.into_iter().map(|r| UsageSummary{tenant_id:r.get(0),requests:r.get::<_,i64>(1) as u64,total_input_tokens:r.get::<_,i64>(2) as u64,total_output_tokens:r.get::<_,i64>(3) as u64}).collect())
    }
    fn upsert_node(
        &self,
        registration: &NodeRegistration,
        connected_at_ms: u64,
    ) -> StorageResult<()> {
        let mut c = self.client()?;
        c.execute("INSERT INTO node_connections (node_id,tenant_id,user_id,auth_token,connected_at_ms,last_seen_ms,inflight_requests) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(node_id) DO UPDATE SET tenant_id=EXCLUDED.tenant_id,user_id=EXCLUDED.user_id,auth_token=EXCLUDED.auth_token,connected_at_ms=EXCLUDED.connected_at_ms,last_seen_ms=EXCLUDED.last_seen_ms,inflight_requests=EXCLUDED.inflight_requests", &[&registration.node_id,&registration.tenant_id,&registration.user_id,&registration.auth_token,&(connected_at_ms as i64),&(connected_at_ms as i64),&0_i64])?;
        Ok(())
    }
    fn touch_node(
        &self,
        node_id: &str,
        seen_ms: u64,
        inflight_requests: usize,
    ) -> StorageResult<()> {
        let mut c = self.client()?;
        c.execute(
            "UPDATE node_connections SET last_seen_ms=$2, inflight_requests=$3 WHERE node_id=$1",
            &[&node_id, &(seen_ms as i64), &(inflight_requests as i64)],
        )?;
        Ok(())
    }
    fn remove_node(&self, node_id: &str) -> StorageResult<()> {
        let mut c = self.client()?;
        c.execute("DELETE FROM node_connections WHERE node_id=$1", &[&node_id])?;
        Ok(())
    }
    fn list_nodes(&self) -> StorageResult<Vec<NodeConnection>> {
        let mut c = self.client()?;
        Ok(c.query("SELECT tenant_id,user_id,node_id,connected_at_ms,last_seen_ms,inflight_requests FROM node_connections ORDER BY tenant_id,user_id,node_id", &[])?.into_iter().map(|r| NodeConnection{tenant_id:r.get(0),user_id:r.get(1),node_id:r.get(2),connected_at_ms:r.get::<_,i64>(3) as u64,last_seen_ms:r.get::<_,i64>(4) as u64,inflight_requests:r.get::<_,i64>(5) as usize}).collect())
    }
}
