# NEXUS

NEXUS is a multitenant Rust gateway with SQLite-first persistence, WebSocket node RPC, tenant-scoped channel routing, and an admin interface.

## Components

- `shared-protocol`: shared config and RPC contract types
- `storage`: repository interfaces, SQLite backend, PostgreSQL migration scaffold
- `server`: multitenant gateway, admin endpoints, usage metering, safeguards
- `client-node`: local tool executor connected over WebSocket

## Configuration

Server default config is in `server/config/default.toml`:

- `bind_addr`: HTTP/WebSocket bind address
- `sqlite_path`: SQLite database path
- `vlm_endpoint`: provider health-check endpoint
- `auth.node_auth_token`: token required for node registration
- `auth.admin_username` and `auth.admin_password`: admin guard credentials
- `limits.max_connections`: WebSocket connection cap
- `limits.max_inflight_requests`: RPC inflight cap
- `limits.request_timeout_ms`: per-request timeout

Client default config is in `client-node/config/default.toml`:

- `node_id`, `tenant_id`, `user_id`
- `server_endpoint`
- `auth_token`

## Run

From `nexus` (NEXUS workspace):

```bash
cargo run -p server
```

In another terminal:

```bash
cargo run -p client-node
```

Admin and service endpoints:

- `GET /health`
- `GET /admin`
- `GET /admin/tenants`
- `GET /admin/usage`
- `GET /admin/nodes`
- `GET /admin/channel-route?tenant_id=...&channel_name=...&external_user=...`
- `GET /admin/provider-health`
- `POST /rpc/tool`
- `GET /ws`

Use header `authorization: Basic <username>:<password>` for admin and RPC routes.

## SQLite and Migration Prep

- SQLite schema is created by `SqliteRepository::migrate()`.
- Repository access is done through `GatewayRepository`.
- PostgreSQL migration path is scaffolded by `PostgresScaffoldRepository` and `RepositoryFactory`.

## Safeguards

- Connection semaphore for max connected nodes
- Inflight semaphore for RPC backpressure
- Bounded per-node outbound queue
- Heartbeat ping/pong lifecycle updates
- Tenant/user checks before dispatching remote tools

## Test Coverage

- Tenant isolation and per-user channel binding tests in `storage/tests`
- Tool RPC roundtrip test in `server/tests/tool_roundtrip.rs`
- Provider health-check and load baseline tests in `server/src/lib.rs` tests
