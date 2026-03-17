# NEXUS

NEXUS is a multitenant Rust gateway with PostgreSQL persistence, WebSocket node RPC, and a decoupled WebUI frontend.

## Components

- `shared-protocol`: shared config and RPC contract types
- `storage`: PostgreSQL repository implementation
- `server`: gateway service + auth + API + static webui hosting
- `client-node`: local tool executor connected over WebSocket
- `webui`: Vue 3 + Vite + TypeScript + Pinia + Vue Router frontend

## Configuration

Server default config is in `server/config/default.toml`:

- `bind_addr`: HTTP/WebSocket bind address
- `postgres_dsn`: PostgreSQL DSN (required)
- `vlm_endpoint`: provider endpoint metadata
- `auth.node_auth_token`: token required for node registration
- `auth.admin_username` and `auth.admin_password`: admin basic-auth credentials
- `limits.max_connections`: WebSocket connection cap
- `limits.max_inflight_requests`: RPC inflight cap
- `limits.request_timeout_ms`: per-request timeout

## Local development

### 1) Start backend

```bash
cargo run -p server
```

### 2) Start frontend (dev mode)

```bash
cd webui
npm install
npm run dev
```

By default Vite proxies `/api`, `/auth`, `/user`, `/rpc` to `http://127.0.0.1:7878`.

### 3) Build frontend for production

```bash
cd webui
npm run build
```

Build output is `webui/dist`. Server serves static assets from this directory.

## UI routes

- `/admin/*` -> administrator app
- `/app/*` -> user app
- `/login` -> unified login page

## API overview

- `GET /health`
- `GET /openapi.yaml`
- `POST /auth/register`
- `POST /auth/login` (sets `nexus_session` HttpOnly cookie + `nexus_csrf` cookie)
- `POST /auth/logout`
- `GET /api/admin/dashboard` (Basic auth required)
- `GET /api/user/dashboard` (cookie session required)
- `GET /api/user/sessions`
- `GET /api/user/sessions/{session_id}/memory`
- `GET /api/user/usage`
- `GET /user/devices`
- `POST /user/dispatch` (requires `x-csrf-token`)
- `POST /rpc/tool` (Basic auth required)
- `GET /ws`

## Auth model

- Admin: Basic auth header (`authorization: Basic <username>:<password>`)
- User: cookie session (`nexus_session`) + CSRF token (`nexus_csrf` + `x-csrf-token` for mutating user actions)

## OpenAPI

OpenAPI spec is maintained at `server/openapi.yaml` and exposed at runtime via `/openapi.yaml`.
