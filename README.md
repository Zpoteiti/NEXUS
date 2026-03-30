# NEXUS
**Networked Execution Exchange for Unified Services**

NEXUS is a distributed AI agent system that separates orchestration from execution. A central server runs the agent loop and manages conversations, while lightweight clients deployed on remote machines expose local tools (shell, MCP servers, Skills) and execute them on demand.

---

## Architecture

```
User ↔ Browser ↔ nexus-gateway ↔ nexus-server ↔ nexus-client(s)
                                       ↑
                   Discord/Telegram Channel（直连）
```

- **nexus-server** — the orchestration hub. Runs the ReAct agent loop, calls the LLM, routes tool requests to the right device, and manages sessions, memory, and authentication.
- **nexus-client** — the execution node. Connects to the server via WebSocket, registers its local tool capabilities, and executes tool calls on behalf of the agent.
- **nexus-common** — the frozen shared protocol layer. Defines the WebSocket message types and constants used by both server and client.
- **nexus-gateway** — the browser WebSocket gateway. Accepts browser connections on `/ws/browser` and nexus-server connections on `/ws/nexus`, bridging messages between them.

### Communication topology

| Link | Transport |
|------|-----------|
| Browser ↔ Gateway | WebSocket (`/ws/browser`) |
| Gateway ↔ Server | WebSocket (`/ws/nexus`, server connects as client) |
| Client ↔ Server | WebSocket (`/ws`) |

### Tool types

Clients aggregate three tool categories and register them with the server:

| Category | Naming convention | Source |
|----------|-------------------|--------|
| Built-in | original name (e.g. `shell`) | compiled into the client |
| MCP | `mcp_{server_name}_*` | external MCP servers |
| Skill | `skill_*` | scripts in the skills directory |

---

## Crates

### `nexus-common`
Protocol and constants shared between server and client. Defines `ServerToClient` / `ClientToServer` message enums, exit code conventions, and token format constants. **Frozen — no functional changes.**

### `nexus-server`
- Authenticates devices via Device Tokens (database-backed, revocable)
- Maintains a live device registry and per-device tool snapshot
- Runs the ReAct agent loop: LLM call → tool dispatch → result collection → reply
- Suspends tool calls via `tokio::sync::oneshot` channels; cleans up on device disconnect
- Persists sessions and memory in PostgreSQL (with pgvector)

**Requirements:** Linux, Docker Compose, PostgreSQL 16+ with pgvector, Rust 1.85+

**Key environment variables:**

| Variable | Description |
|----------|-------------|
| `DATABASE_URL` | PostgreSQL connection string |
| `ADMIN_TOKEN` | Token for the `/admin/register` endpoint |
| `SERVER_PORT` | Listening port (default: `8080`) |
| `HEARTBEAT_TIMEOUT_SEC` | Seconds before an unresponsive device is evicted (default: `60`) |

### `nexus-client`
- Connects to the server, authenticates, and registers tools in three phases: connect → discover & register → heartbeat loop
- Detects tool-set changes via hash on every heartbeat and re-registers when changed
- Applies guardrails (command pattern checks, path bounds, network targets) before executing any tool
- Reconnects with exponential backoff on disconnect; replays the full handshake sequence on reconnect

**Requirements:** Linux or Windows, Rust 1.85+, network access to the server

**Key environment variables:**

| Variable | Description |
|----------|-------------|
| `NEXUS_SERVER_WS_URL` | Server WebSocket address (default: `ws://127.0.0.1:8080/ws`) |
| `NEXUS_AUTH_TOKEN` | Device token (`nexus_dev_` + 32 random chars) |
| `NEXUS_DEVICE_ID` | Device identifier (default: hostname) |
| `NEXUS_DEVICE_NAME` | Human-readable device name (default: hostname) |
| `NEXUS_MCP_SERVERS_JSON` | JSON array or object of MCP server configs |
| `NEXUS_SKILLS_DIR` | Path to the skills directory (default: `~/.nexus/skills`) |

### `nexus-gateway`
Standalone Rust WebSocket gateway. Accepts browser connections on `/ws/browser` (assigns `chat_id`) and nexus-server connections on `/ws/nexus` (gateway token auth), bridging messages bidirectionally. Deployed independently from the server.

**Key environment variables:**

| Variable | Description |
|----------|-------------|
| `GATEWAY_PORT` | Listening port (default: `9090`) |
| `NEXUS_GATEWAY_TOKEN` | Shared secret for nexus-server authentication |

---

## Acknowledgements

NEXUS is architecturally inspired by [nanobot](https://github.com/HKUDS/nanobot), an ultra-lightweight open-source AI agent by HKUDS. Several core design patterns in NEXUS — including the ReAct agent loop, memory consolidation strategy, tool execution guardrails, and MCP integration approach — are adapted from nanobot's implementation.
