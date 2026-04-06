# nexus-gateway

Browser-facing entry point for NEXUS. Serves the web frontend, proxies REST API requests to the server, and manages WebSocket chat sessions.

---

## Responsibilities

- **WebSocket chat** (`/ws/chat`): Accepts browser connections with JWT authentication (via query parameter). Manages session creation, switching, and progress forwarding.
- **REST API proxy** (`/api/*`): Forwards all API requests to nexus-server, passing through auth headers.
- **Static frontend serving**: Serves the built frontend from `NEXUS_FRONTEND_DIR` with SPA fallback (all unmatched routes serve `index.html`).
- **Server connection** (`/ws/nexus`): Accepts nexus-server WebSocket connections with gateway token authentication.
- **CORS**: Permissive CORS policy (allow all origins, methods, headers).

## Does NOT do

- Execute tools or run the agent loop.
- Persist sessions or messages (stateless proxy).
- Connect directly to nexus-client devices.

---

## Routes

| Route | Method | Purpose |
|-------|--------|---------|
| `/ws/chat` | GET (WebSocket) | Browser chat sessions (JWT from query param) |
| `/ws/nexus` | GET (WebSocket) | Server connection (gateway token auth) |
| `/api/*` | Any | REST proxy to nexus-server |
| `/*` | GET | Static frontend files with SPA fallback |

---

## WebSocket protocol

### Browser -> Gateway (`/ws/chat`)

| Type | Fields | Purpose |
|------|--------|---------|
| `message` | `content`, `media?` | Send a chat message |
| `new_session` | -- | Create a new session |
| `switch_session` | `session_id` | Switch to an existing session |

### Gateway -> Browser

| Type | Fields | Purpose |
|------|--------|---------|
| `message` | `content`, `session_id`, `media?` | Agent response |
| `progress` | `content`, `session_id` | Progress hint during processing |
| `error` | `reason` | Error notification |
| `session_created` | `session_id` | New session confirmation |
| `session_switched` | `session_id` | Session switch confirmation |

---

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `GATEWAY_PORT` | `9090` | Listening port |
| `NEXUS_GATEWAY_TOKEN` | (required) | Shared secret for nexus-server authentication |
| `JWT_SECRET` | (required) | Secret for JWT validation |
| `NEXUS_SERVER_API_URL` | `http://localhost:8080` | Server API URL for REST proxying |
| `NEXUS_FRONTEND_DIR` | `../nexus-frontend/dist` | Path to built frontend assets |

---

## Running

```bash
cd NEXUS
NEXUS_GATEWAY_TOKEN=your-token JWT_SECRET=your-secret cargo run --package nexus-gateway
```

---

## Architecture decisions

**Independent binary**: Deployed separately from nexus-server. Can sit at the network edge while the server runs in an internal network.

**JWT double validation**: Gateway validates JWT locally for fast rejection, then the server re-validates on API requests for authoritative access control.

**Stateless proxying**: Gateway holds no persistent state. Browser session state is managed server-side; gateway only routes messages by `chat_id`.
