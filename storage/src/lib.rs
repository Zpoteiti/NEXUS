use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{params, Connection};
use shared_protocol::{
    MemoryRecord, NodeConnection, NodeRegistration, SessionRecord, Tenant, UsageRecord,
    UsageSummary, UserAccount, UserChannelBinding,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("storage lock poisoned")]
    LockPoisoned,
    #[error("sqlite error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("repository not implemented for backend: {0}")]
    NotImplemented(&'static str),
}

pub type StorageResult<T> = Result<T, StorageError>;

pub trait GatewayRepository: Send + Sync {
    fn migrate(&self) -> StorageResult<()>;
    fn upsert_tenant(&self, tenant: &Tenant) -> StorageResult<()>;
    fn list_tenants(&self) -> StorageResult<Vec<Tenant>>;
    fn upsert_user(&self, user: &UserAccount) -> StorageResult<()>;
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
    fn save_node_registration(&self, registration: &NodeRegistration) -> StorageResult<()> {
        self.upsert_node(registration, 0)
    }
    fn get_node_registration(&self, node_id: &str) -> StorageResult<Option<NodeRegistration>> {
        Ok(self.list_nodes()?.into_iter().find(|node| node.node_id == node_id).map(|node| {
            NodeRegistration {
                node_id: node.node_id,
                tenant_id: node.tenant_id,
                user_id: node.user_id,
                auth_token: String::new(),
            }
        }))
    }
}

pub trait RepositoryFactory {
    fn sqlite(path: impl AsRef<Path>) -> StorageResult<Box<dyn GatewayRepository>>;
    fn postgres(_dsn: &str) -> StorageResult<Box<dyn GatewayRepository>>;
}

pub struct StorageFactory;

impl RepositoryFactory for StorageFactory {
    fn sqlite(path: impl AsRef<Path>) -> StorageResult<Box<dyn GatewayRepository>> {
        Ok(Box::new(SqliteRepository::new(path)?))
    }

    fn postgres(_dsn: &str) -> StorageResult<Box<dyn GatewayRepository>> {
        Ok(Box::new(PostgresScaffoldRepository))
    }
}

pub struct PostgresScaffoldRepository;

impl GatewayRepository for PostgresScaffoldRepository {
    fn migrate(&self) -> StorageResult<()> {
        Err(StorageError::NotImplemented("postgres"))
    }

    fn upsert_tenant(&self, _tenant: &Tenant) -> StorageResult<()> {
        Err(StorageError::NotImplemented("postgres"))
    }

    fn list_tenants(&self) -> StorageResult<Vec<Tenant>> {
        Err(StorageError::NotImplemented("postgres"))
    }

    fn upsert_user(&self, _user: &UserAccount) -> StorageResult<()> {
        Err(StorageError::NotImplemented("postgres"))
    }

    fn upsert_channel_binding(&self, _binding: &UserChannelBinding) -> StorageResult<()> {
        Err(StorageError::NotImplemented("postgres"))
    }

    fn resolve_channel_user(
        &self,
        _tenant_id: &str,
        _channel_name: &str,
        _external_user: &str,
    ) -> StorageResult<Option<UserAccount>> {
        Err(StorageError::NotImplemented("postgres"))
    }

    fn save_session(&self, _session: &SessionRecord) -> StorageResult<()> {
        Err(StorageError::NotImplemented("postgres"))
    }

    fn append_memory(&self, _memory: &MemoryRecord) -> StorageResult<()> {
        Err(StorageError::NotImplemented("postgres"))
    }

    fn list_memory(
        &self,
        _tenant_id: &str,
        _user_id: &str,
        _session_id: &str,
    ) -> StorageResult<Vec<MemoryRecord>> {
        Err(StorageError::NotImplemented("postgres"))
    }

    fn record_usage(&self, _usage: &UsageRecord) -> StorageResult<()> {
        Err(StorageError::NotImplemented("postgres"))
    }

    fn usage_summary(&self) -> StorageResult<Vec<UsageSummary>> {
        Err(StorageError::NotImplemented("postgres"))
    }

    fn upsert_node(
        &self,
        _registration: &NodeRegistration,
        _connected_at_ms: u64,
    ) -> StorageResult<()> {
        Err(StorageError::NotImplemented("postgres"))
    }

    fn touch_node(
        &self,
        _node_id: &str,
        _seen_ms: u64,
        _inflight_requests: usize,
    ) -> StorageResult<()> {
        Err(StorageError::NotImplemented("postgres"))
    }

    fn remove_node(&self, _node_id: &str) -> StorageResult<()> {
        Err(StorageError::NotImplemented("postgres"))
    }

    fn list_nodes(&self) -> StorageResult<Vec<NodeConnection>> {
        Err(StorageError::NotImplemented("postgres"))
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

impl GatewayRepository for SqliteRepository {
    fn migrate(&self) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS tenants (
                tenant_id TEXT PRIMARY KEY,
                display_name TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS users (
                tenant_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                display_name TEXT NOT NULL,
                PRIMARY KEY (tenant_id, user_id)
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
            "#,
        )?;
        Ok(())
    }

    fn upsert_tenant(&self, tenant: &Tenant) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO tenants (tenant_id, display_name) VALUES (?1, ?2)
            ON CONFLICT(tenant_id) DO UPDATE SET display_name = excluded.display_name",
            params![tenant.tenant_id, tenant.display_name],
        )?;
        Ok(())
    }

    fn list_tenants(&self) -> StorageResult<Vec<Tenant>> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT tenant_id, display_name FROM tenants ORDER BY tenant_id")?;
        let rows = stmt.query_map([], |row| {
            Ok(Tenant { tenant_id: row.get(0)?, display_name: row.get(1)? })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(StorageError::from)
    }

    fn upsert_user(&self, user: &UserAccount) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO users (tenant_id, user_id, display_name) VALUES (?1, ?2, ?3)
            ON CONFLICT(tenant_id, user_id) DO UPDATE SET display_name = excluded.display_name",
            params![user.tenant_id, user.user_id, user.display_name],
        )?;
        Ok(())
    }

    fn upsert_channel_binding(&self, binding: &UserChannelBinding) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO user_channels (tenant_id, user_id, channel_name, external_user, credentials_json)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(tenant_id, channel_name, external_user) DO UPDATE
            SET user_id = excluded.user_id, credentials_json = excluded.credentials_json",
            params![
                binding.tenant_id,
                binding.user_id,
                binding.channel_name,
                binding.external_user,
                binding.credentials_json
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
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT u.tenant_id, u.user_id, u.display_name
             FROM users u
             JOIN user_channels c ON c.tenant_id = u.tenant_id AND c.user_id = u.user_id
             WHERE c.tenant_id = ?1 AND c.channel_name = ?2 AND c.external_user = ?3
             LIMIT 1",
        )?;
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
        conn.execute(
            "INSERT INTO sessions (tenant_id, user_id, session_id, title) VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(tenant_id, session_id) DO UPDATE
            SET user_id = excluded.user_id, title = excluded.title",
            params![session.tenant_id, session.user_id, session.session_id, session.title],
        )?;
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
        let mut stmt = conn.prepare(
            "SELECT tenant_id, user_id, session_id, content
             FROM memory
             WHERE tenant_id = ?1 AND user_id = ?2 AND session_id = ?3
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![tenant_id, user_id, session_id], |row| {
            Ok(MemoryRecord {
                tenant_id: row.get(0)?,
                user_id: row.get(1)?,
                session_id: row.get(2)?,
                content: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(StorageError::from)
    }

    fn record_usage(&self, usage: &UsageRecord) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT OR REPLACE INTO usage_records
            (request_id, tenant_id, user_id, model, input_tokens, output_tokens)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                usage.request_id,
                usage.tenant_id,
                usage.user_id,
                usage.model,
                usage.input_tokens as i64,
                usage.output_tokens as i64
            ],
        )?;
        Ok(())
    }

    fn usage_summary(&self) -> StorageResult<Vec<UsageSummary>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT tenant_id, COUNT(*), COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0)
             FROM usage_records
             GROUP BY tenant_id
             ORDER BY tenant_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(UsageSummary {
                tenant_id: row.get(0)?,
                requests: row.get::<_, i64>(1)? as u64,
                total_input_tokens: row.get::<_, i64>(2)? as u64,
                total_output_tokens: row.get::<_, i64>(3)? as u64,
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
        conn.execute(
            "INSERT OR REPLACE INTO node_connections
            (node_id, tenant_id, user_id, auth_token, connected_at_ms, last_seen_ms, inflight_requests)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                registration.node_id,
                registration.tenant_id,
                registration.user_id,
                registration.auth_token,
                connected_at_ms as i64,
                connected_at_ms as i64,
                0_i64
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
        let conn = self.conn()?;
        conn.execute(
            "UPDATE node_connections SET last_seen_ms = ?2, inflight_requests = ?3 WHERE node_id = ?1",
            params![node_id, seen_ms as i64, inflight_requests as i64],
        )?;
        Ok(())
    }

    fn remove_node(&self, node_id: &str) -> StorageResult<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM node_connections WHERE node_id = ?1", params![node_id])?;
        Ok(())
    }

    fn list_nodes(&self) -> StorageResult<Vec<NodeConnection>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT tenant_id, user_id, node_id, connected_at_ms, last_seen_ms, inflight_requests
             FROM node_connections
             ORDER BY tenant_id, user_id, node_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(NodeConnection {
                tenant_id: row.get(0)?,
                user_id: row.get(1)?,
                node_id: row.get(2)?,
                connected_at_ms: row.get::<_, i64>(3)? as u64,
                last_seen_ms: row.get::<_, i64>(4)? as u64,
                inflight_requests: row.get::<_, i64>(5)? as usize,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(StorageError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::{GatewayRepository, SqliteRepository};
    use shared_protocol::{MemoryRecord, UserAccount, UserChannelBinding};

    #[test]
    fn enforces_tenant_isolation_for_memory_reads() {
        let repo = SqliteRepository::in_memory().expect("init sqlite");
        repo.append_memory(&MemoryRecord {
            tenant_id: "t1".to_owned(),
            user_id: "u1".to_owned(),
            session_id: "s1".to_owned(),
            content: "private".to_owned(),
        })
        .expect("memory");

        let isolated = repo.list_memory("t2", "u1", "s1").expect("list");
        assert!(isolated.is_empty());
    }

    #[test]
    fn resolves_channel_binding_per_user() {
        let repo = SqliteRepository::in_memory().expect("init sqlite");
        repo.upsert_user(&UserAccount {
            tenant_id: "t1".to_owned(),
            user_id: "alice".to_owned(),
            display_name: "Alice".to_owned(),
        })
        .expect("user");
        repo.upsert_channel_binding(&UserChannelBinding {
            tenant_id: "t1".to_owned(),
            user_id: "alice".to_owned(),
            channel_name: "telegram".to_owned(),
            external_user: "tg-alice".to_owned(),
            credentials_json: "{}".to_owned(),
        })
        .expect("channel");

        let resolved = repo.resolve_channel_user("t1", "telegram", "tg-alice").expect("resolve");
        assert!(resolved.is_some());
        assert_eq!(resolved.expect("present").user_id, "alice");
    }
}
