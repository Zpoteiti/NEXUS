# nexus-gateway

Browser-to-server WebSocket bridge for NEXUS.

## Why it exists

The gateway is a separate process that sits between browsers and the nexus-server. This lets you deploy it independently -- run it on an edge node close to users, or on a completely different machine from the server. It handles JWT auth for browsers, manages WebSocket lifecycle, and proxies REST calls, so the server never deals with browser connections directly.

It also serves the built frontend as static files, so in production you only need to expose the gateway port.

## Environment Variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `NEXUS_GATEWAY_TOKEN` | Yes | -- | Shared secret the nexus-server uses to authenticate its WebSocket connection to the gateway |
| `JWT_SECRET` | Yes | -- | Secret for validating browser JWT tokens (must match the server's JWT signing key) |
| `GATEWAY_PORT` | Yes | -- | Port the gateway listens on (e.g. `9090`) |
| `NEXUS_SERVER_API_URL` | Yes | -- | Base URL of the nexus-server REST API (e.g. `http://server:8080`) |
| `NEXUS_FRONTEND_DIR` | No | `../nexus-frontend/dist` | Path to the built frontend assets to serve |

## Architecture

Two WebSocket endpoints, one for each side of the bridge:

### `/ws/chat` -- Browser connections

Browsers connect here with a JWT token (via `Authorization: Bearer <token>` header or `?token=` query param -- browsers can't send custom headers on WS upgrade, so the query param exists as a fallback).

On connect, the gateway assigns a `chat_id` (UUID) and a `session_id` (`gateway:{user_id}:{uuid}`), then sends a `session_created` message back. The browser can then send messages, create new sessions, or switch between existing sessions.

### `/ws/nexus` -- Server connection

The nexus-server connects here. Only one server connection is allowed at a time -- a second attempt gets rejected.

Auth flow: the server sends an `{"type":"auth","token":"..."}` message first. The gateway validates it against `NEXUS_GATEWAY_TOKEN` using constant-time comparison, then responds with `auth_ok` or `auth_fail`.

Once authenticated, the server sends `send` messages (agent replies) that the gateway routes to the correct browser by `chat_id`. The gateway forwards browser messages to the server as `message` events with `chat_id`, `sender_id`, `content`, `session_id`, and optional `media`.

## Protocol

Messages are JSON with `{"type": "..."}` discriminators. Quick overview:

**Browser -> Gateway:**
- `message` -- user sends a chat message (with optional `media` attachments)
- `new_session` -- start a fresh conversation session
- `switch_session` -- resume a previous session by ID

**Gateway -> Browser:**
- `message` -- agent reply (with `session_id` and optional `media`)
- `progress` -- intermediate "thinking" update from the agent
- `error` -- something went wrong (e.g. server not connected)
- `session_created` / `session_switched` -- session lifecycle confirmations

**Server -> Gateway:**
- `auth` -- authenticate the server connection
- `send` -- deliver a reply to a browser (with optional `metadata` including `_progress` flag)

**Gateway -> Server:**
- `auth_ok` / `auth_fail` -- auth result
- `message` -- forwarded browser message with routing info

See `src/protocol.rs` for the full type definitions.

## REST Proxy

All `/api/*` requests are proxied to the nexus-server (`NEXUS_SERVER_API_URL`). The gateway validates the JWT on every request (except `/api/auth/login` and `/api/auth/register`, which are public). Hop-by-hop headers are stripped, and request/response bodies are capped at 25 MB. Path traversal attempts (`..`) are rejected.

This means the browser only needs to know the gateway's address -- all API calls and WebSocket connections go through it.

## Build & Run

```bash
# Build
cargo build --package nexus-gateway

# Run (with required env vars)
NEXUS_GATEWAY_TOKEN=your-shared-secret \
JWT_SECRET=your-jwt-secret \
cargo run --package nexus-gateway

# Or with a .env file in the project root
cargo run --package nexus-gateway
```

The gateway will listen on `0.0.0.0:{GATEWAY_PORT}` and serve the frontend from `NEXUS_FRONTEND_DIR`.
