# M4: Gateway Enhancement + Web Frontend — Design Spec

## Goal

Build a web-based UI for NEXUS with the gateway as the single public-facing entry point. Users interact through a browser chat interface; all traffic routes through the gateway to the private server.

## Architecture

```
                    PUBLIC NETWORK                      PRIVATE NETWORK
                         │                                    │
[Browser] ──HTTPS──→ [Gateway :9090] ──HTTP/WS──→ [Server :8080]
                      │  serves:                         │
                      │  - static frontend files         ├── PostgreSQL
                      │  - WebSocket /ws/chat             ├── Embedding service
                      │  - REST proxy /api/*              └── [Clients] (outbound WS)
                      │
                      └── also accepts: [Server] inbound WS at /ws/nexus
```

- **Gateway** is the only component exposed to the internet
- **Server** listens on private network only (gateway + clients)
- **Clients** initiate outbound connections to server — no inbound ports
- **Frontend** static files served by gateway in production, Vite dev server in development

## Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Gateway role | Full API gateway (WS + REST proxy) | Server stays private, single public entry point |
| JWT validation | Double — gateway validates AND server validates | Defense in depth |
| Auth flow | Browser → gateway `/api/auth/login` → server → JWT returned | Auth stays on server (has DB), gateway just proxies |
| Frontend location | `nexus-frontend/` sibling in monorepo | Clean separation of Rust and TypeScript toolchains |
| Static serving | Gateway serves `dist/` in production | Single deployment unit, no separate web server |
| Real-time feedback | Progress hints (thinking/tool/composing) | No token streaming — hints are transient status indicators |
| Tech stack | React + TypeScript + Vite + Tailwind + shadcn/ui | Beginner-friendly, largest ecosystem, professional look |

---

## Gateway Changes

### Current State
- `/ws/browser` — browser WebSocket (JWT auth, relay to server)
- `/ws/nexus` — server WebSocket (token auth, relay from server)

### New Endpoints
- `/ws/chat` — enhanced browser WebSocket (replaces `/ws/browser`)
- `/api/*` — REST proxy to server (new)
- `/*` — static frontend files from configured directory (new)

### REST Proxy

Gateway receives any request matching `/api/*`, validates JWT (except `/api/auth/login` and `/api/auth/register` which are public), and forwards to `http://server:8080/api/*`. Response is relayed back to browser unchanged.

```rust
// Pseudocode
async fn api_proxy(req: Request, state: AppState) -> Response {
    // Skip auth for login/register
    if !is_public_endpoint(&req) {
        validate_jwt(&req, &state.jwt_secret)?;
    }
    // Forward to server
    let server_url = format!("{}{}", state.server_api_url, req.uri());
    let response = reqwest_client.request(req.method(), server_url)
        .headers(req.headers())
        .body(req.body())
        .send().await;
    // Relay response
    response
}
```

### Enhanced WebSocket Protocol

Browser → Gateway messages:
```json
{"type": "message", "content": "user text"}
{"type": "new_session"}
{"type": "switch_session", "session_id": "..."}
```

Gateway → Browser messages:
```json
{"type": "message", "content": "agent response", "session_id": "..."}
{"type": "progress", "content": "⏳ Thinking...", "session_id": "..."}
{"type": "progress", "content": "🔧 shell on xiaoshu", "session_id": "..."}
{"type": "error", "reason": "..."}
{"type": "session_created", "session_id": "..."}
```

### Static File Serving

```rust
// In gateway main.rs
let frontend_dir = env::var("NEXUS_FRONTEND_DIR").unwrap_or("../nexus-frontend/dist".into());
app.fallback_service(ServeDir::new(frontend_dir).fallback(ServeFile::new(format!("{}/index.html", frontend_dir))))
```

SPA fallback: any non-API, non-WS route serves `index.html` (React Router handles client-side routing).

### Gateway Config

New env vars:
- `NEXUS_SERVER_API_URL` — server REST API base (default: `http://localhost:8080`)
- `NEXUS_FRONTEND_DIR` — path to built frontend (default: `../nexus-frontend/dist`)

---

## Progress Hints (Agent Loop Enhancement)

Add status messages at key points in `agent_loop.rs`:

| When | Hint | Metadata |
|------|------|----------|
| Before first LLM call | `⏳ Thinking...` | `_progress: true` |
| Tool execution | `🔧 {tool_name} on {device}` | `_progress: true, _tool_hint: true` (existing) |
| Tool error | `⚠️ Tool {name} error: ...` | `_progress: true, _error: true` (existing) |
| After tool results, before next LLM call | `⏳ Analyzing results...` | `_progress: true` |
| Final LLM response arriving | `💬 Composing response...` | `_progress: true` |

Frontend treats `_progress: true` messages as transient — replaces previous hint, clears when final message arrives.

---

## Frontend Design

### Tech Stack
- **React 18** + **TypeScript 5**
- **Vite** — build tool + dev server
- **Tailwind CSS 3** — utility-first styling
- **shadcn/ui** — pre-built components (Button, Dialog, Input, Table, Sidebar, etc.)
- **React Router v6** — client-side routing
- **Zustand** — lightweight state management (simpler than Redux)

### Pages

#### 1. Login / Register (`/login`)
- Email + password form
- Toggle between login and register
- On success: store JWT in localStorage, redirect to `/chat`

#### 2. Main Chat (`/chat`)
- **Left sidebar**: session list (past conversations), "New Chat" button
- **Center**: message thread (user messages right-aligned, agent messages left-aligned)
- **Top-right**: online devices indicator (green dots with device names)
- **Bottom**: message input with send button
- **Progress hints**: shown as transient status below the last message
- **Tool hints**: shown inline with subtle styling (collapsible)
- WebSocket connection established on page load

#### 3. User Settings (`/settings`)
Tabbed layout:
- **Profile**: username, email (read-only from API)
- **Devices**: list devices, FsPolicy per device, MCP config per device, device tokens
- **Skills**: list/create/delete skills
- **Soul**: edit soul text (personality/instructions)
- **Preferences**: user preferences JSON
- **Cron Jobs**: list/create/delete/toggle scheduled tasks

#### 4. Admin Panel (`/admin`)
Only visible to `is_admin` users. Tabbed:
- **LLM Config**: model, api_base, api_key, context_window, max_output_tokens
- **Embedding Config**: model, api_base, max_input_length, max_concurrency
- **Server MCP**: configure shared MCP servers
- **Default Soul**: set default soul for new users
- **Users**: list all users (future)

### Component Hierarchy
```
App
├── LoginPage
├── ChatPage
│   ├── SessionSidebar
│   │   ├── SessionList
│   │   └── NewChatButton
│   ├── ChatArea
│   │   ├── DeviceStatus (top-right)
│   │   ├── MessageList
│   │   │   ├── UserMessage
│   │   │   ├── AgentMessage (with markdown rendering)
│   │   │   └── ProgressHint (transient)
│   │   └── MessageInput
│   └── WebSocketProvider (context)
├── SettingsPage
│   ├── ProfileTab
│   ├── DevicesTab
│   ├── SkillsTab
│   ├── SoulTab
│   ├── PreferencesTab
│   └── CronTab
└── AdminPage
    ├── LlmConfigTab
    ├── EmbeddingConfigTab
    ├── ServerMcpTab
    └── DefaultSoulTab
```

### API Client

Centralized API client using `fetch`:
```typescript
// api.ts
const API_BASE = ''; // same origin (gateway)

async function apiRequest(path: string, options?: RequestInit) {
    const token = localStorage.getItem('jwt');
    const res = await fetch(`${API_BASE}${path}`, {
        ...options,
        headers: {
            'Content-Type': 'application/json',
            ...(token ? { Authorization: `Bearer ${token}` } : {}),
            ...options?.headers,
        },
    });
    if (res.status === 401) {
        localStorage.removeItem('jwt');
        window.location.href = '/login';
    }
    return res;
}
```

### WebSocket Client

```typescript
// useWebSocket.ts (React hook)
function useWebSocket() {
    const [messages, setMessages] = useState<Message[]>([]);
    const [progress, setProgress] = useState<string | null>(null);
    const ws = useRef<WebSocket | null>(null);

    useEffect(() => {
        const token = localStorage.getItem('jwt');
        ws.current = new WebSocket(`wss://${location.host}/ws/chat?token=${token}`);
        
        ws.current.onmessage = (event) => {
            const data = JSON.parse(event.data);
            if (data.type === 'progress') {
                setProgress(data.content);
            } else if (data.type === 'message') {
                setProgress(null);
                setMessages(prev => [...prev, data]);
            }
        };
        
        return () => ws.current?.close();
    }, []);

    const send = (content: string) => {
        ws.current?.send(JSON.stringify({ type: 'message', content }));
    };

    return { messages, progress, send };
}
```

---

## Session Management

### Current Model
- Session ID format: `gateway:{chat_id}` where `chat_id` is a random UUID per browser connection
- Each browser tab gets a new session
- No session persistence across page reloads

### Enhanced Model
- Session ID format: `gateway:{user_id}:{session_name}` (deterministic, user-scoped)
- Sessions persist in DB (already exists)
- Browser can list past sessions via `GET /api/sessions`
- Browser can switch sessions via WebSocket message `{"type": "switch_session", "session_id": "..."}`
- "New Chat" creates a new session
- Page reload reconnects to the last active session

### Gateway Changes for Session Management
- Gateway tracks `user_id → active_session_id` mapping
- On browser connect: resume last session or create new
- On `switch_session`: update mapping, load history from server API
- On `new_session`: create new session, update mapping

---

## Development Workflow

```bash
# Terminal 1: Server (private network)
cd nexus-server && cargo run

# Terminal 2: Gateway
cd nexus-gateway && cargo run

# Terminal 3: Frontend dev server (hot reload)
cd nexus-frontend && npm run dev
# Vite proxies /api/* and /ws/* to gateway:9090

# Terminal 4: Client (optional)
cd nexus-client && cargo run
```

Vite config:
```typescript
// vite.config.ts
export default defineConfig({
    server: {
        proxy: {
            '/api': 'http://localhost:9090',
            '/ws': { target: 'ws://localhost:9090', ws: true },
        },
    },
});
```

---

## File Upload / Download

### Design Principles
- Files stored on server filesystem at `/tmp/nexus-uploads/{user_id}/` (temp, not DB)
- Every channel downloads attachments to server temp, passes local path via `InboundEvent.media`
- Agent gets consistent file access regardless of source channel (Discord, Web UI, future Telegram)
- Server never executes uploaded files — just stores and serves bytes

### Web UI Upload Flow
1. User attaches file in chat → browser sends `POST /api/files` (multipart, via gateway proxy)
2. Server saves to `/tmp/nexus-uploads/{user_id}/{uuid}_{filename}`
3. Server returns `{ "file_id": "...", "file_name": "...", "file_path": "/tmp/nexus-uploads/..." }`
4. Browser sends chat message with attachment reference via WebSocket
5. Gateway forwards to server with file path in `InboundEvent.media`
6. Agent sees the file path, reads it directly from server filesystem

### Agent → User File Download
1. Agent calls `send_file` tool → file saved to `/tmp/nexus-media/`
2. OutboundEvent includes `media: ["__FILE__:/tmp/nexus-media/..."]`
3. Gateway channel sends file download URL to browser
4. Browser renders download link or inline preview (images)
5. Download via `GET /api/files/{file_id}` (proxied through gateway)

### Mitigations
- **Size limit**: 25MB per file (same as Discord)
- **TTL cleanup**: server deletes uploaded files after 24h
- **No execution**: server only stores/serves bytes, never runs uploaded files

### Standardized Channel File Handling
All channels follow the same pattern:
```
Channel receives attachment → downloads to /tmp/nexus-uploads/ → path in InboundEvent.media
```
This ensures the agent loop has a consistent interface regardless of which channel the file came from.

---

## Cross-Channel Session Visibility

### Behavior
- `GET /api/sessions` returns ALL user sessions across ALL channels (Discord, Web UI, future Telegram)
- Web UI sidebar shows all sessions with channel indicator (e.g., "💬 Web Chat", "🎮 Discord #general")
- **Cross-channel sessions are read-only**: user can view message history but cannot send messages in a session from a different channel
- Only sessions matching the current channel (gateway/web UI) are interactive
- This prevents confusion where a reply goes to both Discord and web UI simultaneously

### Session Display
```
Session Sidebar:
├── 💬 New Chat                    ← interactive (web UI session)
├── 💬 Chat about deployment       ← interactive (web UI session)
├── 🎮 Discord #general           ← read-only (view history only)
├── 🎮 Discord DM                 ← read-only (view history only)
└── ⏰ Cron: daily-weather-check  ← read-only (cron session)
```

---

## Implementation Order

1. **Gateway REST proxy** — add `/api/*` forwarding to server
2. **Gateway static serving** — serve frontend `dist/` with SPA fallback
3. **Gateway WebSocket enhancement** — session management, progress forwarding
4. **Server file upload API** — `POST/GET /api/files` endpoints
5. **Agent loop progress hints** — add thinking/analyzing/composing status messages
6. **Frontend scaffold** — Vite + React + TypeScript + Tailwind + shadcn/ui
7. **Login/Register page** — auth flow through gateway
8. **Chat page** — WebSocket connection, message display, session sidebar, file upload/download
9. **Settings page** — profile, devices, skills, soul, cron tabs
10. **Admin page** — LLM config, embedding, server MCP

---

## Config Summary

### Gateway env vars (new/changed)
| Var | Default | Purpose |
|-----|---------|---------|
| `GATEWAY_PORT` | 9090 | Gateway listen port (existing) |
| `NEXUS_GATEWAY_TOKEN` | required | Server auth token (existing) |
| `JWT_SECRET` | required | JWT validation secret (existing) |
| `NEXUS_SERVER_API_URL` | `http://localhost:8080` | Server REST API base (new) |
| `NEXUS_FRONTEND_DIR` | `../nexus-frontend/dist` | Frontend static files path (new) |

### Frontend env vars (build-time)
| Var | Default | Purpose |
|-----|---------|---------|
| `VITE_API_BASE` | `` (empty = same origin) | API base URL override for development |
