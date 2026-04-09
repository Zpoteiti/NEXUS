# nexus-server

Orchestration hub -- runs the ReAct agent loop, manages users/sessions/devices, and routes tool calls to connected clients.

## Module Map

| Module | What it does |
|--------|-------------|
| `agent_loop` | Per-session ReAct loop: calls the LLM, dispatches tool calls, loops until done |
| `api` | REST endpoints for the WebUI (sessions, memory, devices, files, user profile) |
| `auth` | Registration, login, JWT sign/verify, Axum middleware, plus sub-modules for admin/device/discord/cron/skills APIs |
| `bus` | Internal message bus -- `InboundEvent` (user messages from any channel) and `OutboundEvent` routing via broadcast/mpsc |
| `channels` | Channel abstraction (gateway, Discord); `ChannelManager` spawns each channel and runs the outbound dispatch loop |
| `config` | Loads env vars into `ServerConfig`, defines `LlmConfig` struct |
| `context` | Assembles the full prompt (system prompt + history + soul + memory + tool schemas) before each LLM call |
| `cron` | Cron scheduler -- polls DB for due jobs every 10s and injects prompts into agent loop via bus |
| `db` | All PostgreSQL interactions (SQLx). Pure async CRUD, no business logic. Sub-modules: users, sessions, messages, devices, discord, cron, skills, checkpoints |
| `file_store` | Centralized file storage for uploads/media/temp files (25MB max, hourly cleanup of files >24h old) |
| `memory` | Context compression -- when remaining context window drops below 16K tokens, compresses history into a summary |
| `providers` | OpenAI-compatible LLM provider with retry logic |
| `server_mcp` | Server-side MCP client manager -- admins configure shared MCP servers whose tools appear as `device_name="server"` |
| `server_tools` | Server-native tools (memory, message, file_transfer, cron, skills, web_fetch) that execute on the server, not on clients |
| `session` | Session handle management -- per-session locks and inbox queues |
| `state` | Global `AppState`: online device routing table, device name index, tool schemas, DB pool, bus, config |
| `tools_registry` | Resolves `device_name` from tool calls, injects `device_name` enum into schemas, routes `ExecuteToolRequest` to the right client |
| `ws` | Client WebSocket handler: login handshake, message loop (heartbeat, tool registration, tool results), heartbeat reaper |

## Server-Native Tools

These tools run on the server, not on client devices. No `device_name` routing needed.

| Tool | Description |
|------|-------------|
| `save_memory` | Save text to the user's persistent memory |
| `edit_memory` | Edit existing memory text |
| `message` | Send a message to any channel, optionally with file attachments. Files are pulled from the specified `from_device` and delivered to the target channel. This is the only way to send files to users. |
| `file_transfer` | Move files between devices. Server acts as relay: pulls from `from_device`, pushes to `to_device`. Supports server → client and client → client (via server relay). |
| `cron` | Manage scheduled jobs: create, list, remove. Supports `cron_expr`, `every_seconds`, and one-shot `at` scheduling. |
| `read_skill` | Read a skill's full SKILL.md instructions (per-user isolated, on-demand for progressive disclosure) |
| `install_skill` | Install a skill from a GitHub repo for the current user. Per-user isolated — each user has their own skill set. |
| `web_fetch` | Fetch a URL and extract readable content. SSRF-protected. Output flagged as untrusted. |

## Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DATABASE_URL` | Yes | -- | PostgreSQL connection string (e.g. `postgres://user:pass@localhost/nexus`) |
| `ADMIN_TOKEN` | Yes | -- | Secret token for admin operations |
| `JWT_SECRET` | Yes | -- | JWT signing key (recommend 32+ characters) |
| `SERVER_PORT` | Yes | -- | HTTP listen port (e.g. `8080`) |
| `NEXUS_GATEWAY_WS_URL` | Yes | -- | WebSocket URL for the gateway connection (e.g. `ws://localhost:9090/ws/nexus`) |
| `NEXUS_GATEWAY_TOKEN` | Yes | -- | Shared secret for gateway authentication |
| `NEXUS_SKILLS_DIR` | No | `~/.nexus/skills` | Directory where skill scripts are stored |

LLM configuration (`api_base`, `api_key`, `model`, `context_window`) is managed via the `/api/llm-config` API and persisted in the `system_config` DB table -- not env vars.

## API Endpoints

### Auth (public)

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/auth/register` | Register a new user |
| POST | `/api/auth/login` | Login, returns JWT |

### WebSocket

| Method | Path | Description |
|--------|------|-------------|
| GET | `/ws` | Client WebSocket connection (login handshake, tool execution, heartbeat) |

### Devices (JWT required)

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/device-tokens` | Create a device token |
| GET | `/api/device-tokens` | List device tokens |
| DELETE | `/api/device-tokens/{token}` | Delete a device token |
| GET | `/api/devices` | List connected devices |
| GET | `/api/devices/{device_name}/policy` | Get device filesystem policy |
| PATCH | `/api/devices/{device_name}/policy` | Update device filesystem policy |
| GET | `/api/devices/{device_name}/mcp` | Get device MCP server config |
| PUT | `/api/devices/{device_name}/mcp` | Update device MCP server config |

### Sessions (JWT required)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/sessions` | List sessions |
| DELETE | `/api/sessions/{session_id}` | Delete a session |
| GET | `/api/sessions/{session_id}/messages` | Get session message history |

### User (JWT required)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/user/profile` | Get user profile |
| GET | `/api/user/soul` | Get user's soul (system prompt) |
| PATCH | `/api/user/soul` | Update user's soul |
| GET | `/api/user/memory` | Get user's persistent memory text |
| PATCH | `/api/user/memory` | Update user's memory text |

### Discord (JWT required)

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/discord-config` | Create/update Discord bot config |
| GET | `/api/discord-config` | Get Discord bot config |
| DELETE | `/api/discord-config` | Delete Discord bot config |

### Skills (JWT required)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/skills` | List user's skills |
| POST | `/api/skills` | Create a skill |
| POST | `/api/skills/install` | Install a skill from URL |
| DELETE | `/api/skills/{name}` | Delete a skill |

### Cron Jobs (JWT required)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/cron-jobs` | List cron jobs |
| POST | `/api/cron-jobs` | Create a cron job |
| PATCH | `/api/cron-jobs/{job_id}` | Update a cron job |
| DELETE | `/api/cron-jobs/{job_id}` | Delete a cron job |

### Files (JWT required)

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/files` | Upload a file |
| GET | `/api/files/{file_id}` | Download a file |

### Admin (JWT required, admin only)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/admin/default-soul` | Get default soul (system prompt) |
| PUT | `/api/admin/default-soul` | Set default soul |
| GET | `/api/admin/skills` | List all users' skills |
| GET | `/api/llm-config` | Get LLM configuration |
| PUT | `/api/llm-config` | Update LLM configuration |
| GET | `/api/server-mcp` | Get server MCP config |
| PUT | `/api/server-mcp` | Update server MCP config |
| GET | `/api/admin/rate-limit` | Get rate limit config |
| PUT | `/api/admin/rate-limit` | Set rate limit config |

## Database Tables

| Table | Purpose |
|-------|---------|
| `users` | User accounts (email, password hash, admin flag, soul, memory text) |
| `device_tokens` | Per-device auth tokens with filesystem policy and MCP config (JSONB) |
| `sessions` | Chat sessions, one per user per channel context |
| `messages` | Message history (role, content, tool call metadata, compressed flag) |
| `discord_configs` | Per-user Discord bot configuration (token, allowed users) |
| `system_config` | Key-value store for server-wide settings (LLM config, MCP config) |
| `cron_jobs` | Scheduled jobs with cron expressions or interval-based triggers |
| `agent_checkpoints` | In-flight agent loop state for crash recovery (messages, iteration count) |
| `skills` | User-installed skill scripts (name, description, file path, always-on flag) |

## Build & Run

```bash
# Build
cargo build --package nexus-server

# Run (requires PostgreSQL)
DATABASE_URL=postgres://user:pass@localhost/nexus \
ADMIN_TOKEN=your-admin-token \
JWT_SECRET=your-jwt-secret-at-least-32-chars \
cargo run --package nexus-server

# Lint
cargo clippy --package nexus-server

# Format check
cargo fmt --package nexus-server --check
```
