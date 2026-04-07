# NEXUS
**Networked Execution Exchange for Unified Services**

NEXUS is a distributed AI agent system that separates orchestration from execution. A central server runs the ReAct agent loop and manages conversations, while lightweight clients deployed on remote machines expose local tools and execute them on demand. A gateway serves the web frontend, proxies REST APIs, and manages browser WebSocket sessions.

---

## Architecture

```
                              +------------------+
                              |   LiteLLM Proxy  |
                              | (multi-provider) |
                              +--------+---------+
                                       |
User --> Browser --> nexus-gateway --> nexus-server --> nexus-client(s)
                      (REST proxy,        |
                       /ws/chat,          |
                       frontend)    Discord Channel
```

- **nexus-server** -- the orchestration hub. Runs the ReAct agent loop, calls the LLM via LiteLLM, routes tool requests to devices, and manages sessions, memory, skills, and cron jobs.
- **nexus-client** -- the execution node. Connects to the server via WebSocket, registers local tool capabilities, and executes tool calls on behalf of the agent.
- **nexus-common** -- the shared protocol and error layer. Defines WebSocket message types, error codes, MCP utilities, and constants.
- **nexus-gateway** -- the browser-facing layer. Serves the frontend, proxies REST API requests to the server, and manages WebSocket chat sessions with progress forwarding.
- **nexus-frontend** -- the web UI. React + TypeScript + Vite + Tailwind. Provides login, chat, settings, and admin pages.

### Communication topology

| Link | Transport |
|------|-----------|
| Browser <-> Gateway | WebSocket (`/ws/chat`), REST (`/api/*`), static files |
| Gateway <-> Server | REST proxy (`/api/*` -> server) |
| Client <-> Server | WebSocket (`/ws`) |
| Server <-> LiteLLM | HTTP (local proxy, auto-managed venv) |
| Discord <-> Server | Discord bot (direct connection) |

### Tool types

The system supports tools from multiple sources:

| Category | Naming convention | Source |
|----------|-------------------|--------|
| Server-native | original name | compiled into the server (e.g. `save_memory`, `send_file`, `download_to_device`, `message`, `cron_create`, `cron_list`, `cron_remove`, `read_skill`, `read_skill_file`) |
| Built-in (client) | original name (e.g. `shell`, `read_file`, `write_file`, `edit_file`, `list_dir`, `stat`) | compiled into the client |
| MCP (server) | server-side admin-shared tools | rmcp SDK, registered on server |
| MCP (client) | `mcp_{server_name}_*` | external MCP servers via rmcp SDK, with hot-reload |
| Skill | `skill_*` | scripts in the skills directory |

---

## Crates

### `nexus-common`
Protocol and error layer shared across crates.

- **`protocol.rs`** -- `ServerToClient` / `ClientToServer` message enums (`#[serde(tag="type", content="data")]`), `FsPolicy` (Sandbox/Whitelist/Unrestricted), `McpServerEntry`, file upload/download request/response types
- **`error.rs`** -- `ErrorCode` enum (23 variants covering auth, tools, MCP, protocol), `NexusError` (internal), `ApiError` (HTTP JSON responses with auto-derived status codes), axum `IntoResponse` impl
- **`mcp_utils.rs`** -- MCP schema normalization utilities for converting MCP tool schemas to OpenAI-compatible format
- **`consts.rs`** -- shared constants (protocol version, heartbeat interval, agent iteration limit, tool output truncation thresholds, exit codes, token format)

### `nexus-server`
- ReAct agent loop with max 200 iterations, concurrent tool execution, and progress hints
- Server-native tools: `save_memory`, `send_file`, `download_to_device`, `message`, `cron_create`/`cron_list`/`cron_remove`, `read_skill`, `read_skill_file`
- Server-side MCP via rmcp SDK with admin-shared tool configuration
- Memory: pgvector RAG, consolidation, dedup
- Skills: server-side skill system with progressive disclosure and REST API management
- Cron scheduler for recurring tasks
- LiteLLM integration (auto-provisions Python venv, multi-provider LLM support)
- Centralized error handling via `ApiError`/`NexusError`
- Graceful shutdown with checkpointing
- Channels: Discord (multi-bot support), Gateway
- Auth: JWT for browser users, device tokens for clients, admin APIs
- Full REST API for sessions, memory, devices, skills, settings, admin operations

**Key environment variables:**

| Variable | Description |
|----------|-------------|
| `DATABASE_URL` | PostgreSQL connection string |
| `ADMIN_TOKEN` | Token for admin endpoints |
| `SERVER_PORT` | Listening port (default: `8080`) |
| `JWT_SECRET` | Secret for JWT signing/validation |
| `NEXUS_GATEWAY_TOKEN` | Shared secret for gateway authentication |
| `HEARTBEAT_TIMEOUT_SEC` | Seconds before an unresponsive device is evicted (default: `60`) |
| `NEXUS_SKILLS_DIR` | Path to server-side skills directory |
| `LITELLM_PORT` | Port for LiteLLM proxy (default: auto) |

### `nexus-client`
- WebSocket connection with auto-reconnect and exponential backoff
- Built-in tools: `shell`, `read_file`, `write_file`, `edit_file`, `list_dir`, `stat`
- MCP tool discovery via rmcp SDK with hot-reload on config changes
- FsPolicy enforcement (Sandbox/Whitelist/Unrestricted), pushed from server
- Guardrails: shell command validation, path bounds checking, SSRF protection
- File download handling (`download_to_device` support)
- Tool hash-based re-registration on heartbeat

**Key environment variables:**

| Variable | Description |
|----------|-------------|
| `NEXUS_SERVER_WS_URL` | Server WebSocket address (default: `ws://127.0.0.1:8080/ws`) |
| `NEXUS_AUTH_TOKEN` | Device token (`nexus_dev_` + 32 random chars) |

### `nexus-gateway`
- REST API proxy: `/api/*` requests forwarded to nexus-server
- WebSocket chat: `/ws/chat` with session management (create, switch) and progress forwarding
- Static frontend serving with SPA fallback (index.html)
- JWT validation from query parameters (double validation with server)
- CORS support (permissive)

**Key environment variables:**

| Variable | Description |
|----------|-------------|
| `GATEWAY_PORT` | Listening port (default: `9090`) |
| `NEXUS_GATEWAY_TOKEN` | Shared secret for server authentication |
| `JWT_SECRET` | Secret for JWT validation |
| `NEXUS_SERVER_API_URL` | Server API URL for proxying (default: `http://localhost:8080`) |
| `NEXUS_FRONTEND_DIR` | Path to built frontend assets (default: `../nexus-frontend/dist`) |

### `nexus-frontend`
- React 19 + TypeScript + Vite + Tailwind CSS 4
- State management: Zustand
- Pages: Login, Chat (sessions sidebar, device status, progress hints), Settings (6 tabs), Admin (4 tabs)
- File upload/download, markdown rendering (react-markdown)
- SPA with react-router-dom

---

## Quick start

```bash
# 1. Start PostgreSQL with pgvector
docker run -d --name nexus-postgres \
  -e POSTGRES_USER=nexus -e POSTGRES_PASSWORD=nexus -e POSTGRES_DB=nexus \
  -p 5432:5432 pgvector/pgvector:pg16

# 2. Start server (auto-installs LiteLLM on first run)
cd nexus-server && cargo run

# 3. Start gateway
cd nexus-gateway && cargo run

# 4. Start frontend dev server
cd nexus-frontend && npm run dev

# 5. Start client on a device
cd nexus-client && cargo run
```

---

## Requirements

- Rust 1.85+ (edition 2024)
- PostgreSQL 16+ with pgvector
- Node.js (for frontend development)
- Python 3 (auto-managed by LiteLLM integration)

---

## Acknowledgements

NEXUS is architecturally inspired by [nanobot](https://github.com/HKUDS/nanobot), an ultra-lightweight open-source AI agent by HKUDS. Several core design patterns in NEXUS -- including the ReAct agent loop, memory consolidation strategy, tool execution guardrails, and MCP integration approach -- are adapted from nanobot's implementation.
