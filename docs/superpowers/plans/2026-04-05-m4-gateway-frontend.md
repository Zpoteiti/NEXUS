# M4: Gateway + Frontend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a web frontend for NEXUS with the gateway as the single public-facing API gateway (REST proxy + WebSocket + static serving).

**Architecture:** Gateway proxies all REST API calls to the private server, enhances WebSocket with session management and progress forwarding, and serves the React frontend. Frontend is a React+TypeScript SPA with chat, settings, and admin pages.

**Tech Stack:** Rust (axum, reqwest, tower-http) for gateway; React 18, TypeScript 5, Vite, Tailwind CSS 3, shadcn/ui, React Router v6, Zustand for frontend.

---

## Phase 1: Gateway Enhancement (Rust)

### Task 1: Gateway REST Proxy

**Files:**
- Create: `nexus-gateway/src/proxy.rs`
- Modify: `nexus-gateway/src/main.rs`
- Modify: `nexus-gateway/src/state.rs`
- Modify: `nexus-gateway/Cargo.toml`

**Purpose:** Forward all `/api/*` requests to the nexus-server. Public endpoints (login/register) skip JWT validation. All others require valid JWT.

- [ ] **Step 1: Add dependencies**

In `nexus-gateway/Cargo.toml`, add:
```toml
reqwest = { version = "0.12", features = ["json", "multipart", "stream"] }
tower-http = { version = "0.6", features = ["fs", "cors"] }
hyper = { version = "1", features = ["full"] }
```

- [ ] **Step 2: Add server_api_url to state**

In `nexus-gateway/src/state.rs`, add `server_api_url: String` field. In `main.rs`, read from `NEXUS_SERVER_API_URL` env var (default `http://localhost:8080`).

- [ ] **Step 3: Create proxy module**

Create `nexus-gateway/src/proxy.rs`:

```rust
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use tracing::debug;

use crate::state::AppState;

const PUBLIC_PATHS: &[&str] = &["/api/auth/login", "/api/auth/register"];

fn is_public(path: &str) -> bool {
    PUBLIC_PATHS.iter().any(|p| path == *p)
}

pub async fn api_proxy(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();

    // Validate JWT for non-public endpoints
    if !is_public(&path) {
        let auth = req.headers().get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        match auth {
            Some(token) => {
                if crate::browser::verify_jwt(token, &state.jwt_secret).is_err() {
                    return (StatusCode::UNAUTHORIZED, "Invalid or expired token").into_response();
                }
            }
            None => {
                return (StatusCode::UNAUTHORIZED, "Missing Authorization header").into_response();
            }
        }
    }

    // Forward to server
    let target_url = format!("{}{}", state.server_api_url, req.uri());
    debug!("proxy: {} {} -> {}", method, path, target_url);

    let mut headers = req.headers().clone();
    headers.remove("host");

    let client = reqwest::Client::new();
    let body_bytes = axum::body::to_bytes(req.into_body(), 26_214_400) // 25MB limit
        .await
        .unwrap_or_default();

    let resp = match client
        .request(method, &target_url)
        .headers(reqwest_headers(&headers))
        .body(body_bytes)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (StatusCode::BAD_GATEWAY, format!("Server unreachable: {}", e)).into_response();
        }
    };

    // Relay response
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let resp_headers = resp.headers().clone();
    let body = resp.bytes().await.unwrap_or_default();

    let mut response = (status, body).into_response();
    for (k, v) in resp_headers.iter() {
        if k != "transfer-encoding" && k != "connection" {
            response.headers_mut().insert(k.clone(), v.clone());
        }
    }
    response
}

fn reqwest_headers(axum_headers: &HeaderMap) -> reqwest::header::HeaderMap {
    let mut h = reqwest::header::HeaderMap::new();
    for (k, v) in axum_headers.iter() {
        if let Ok(name) = reqwest::header::HeaderName::from_bytes(k.as_str().as_bytes()) {
            if let Ok(val) = reqwest::header::HeaderValue::from_bytes(v.as_bytes()) {
                h.insert(name, val);
            }
        }
    }
    h
}
```

- [ ] **Step 4: Register proxy route in main.rs**

```rust
mod proxy;

// In router:
let app = Router::new()
    .route("/ws/nexus", get(gateway::nexus_ws_handler))
    .route("/ws/chat", get(browser::browser_ws_handler))
    .route("/api/{*path}", any(proxy::api_proxy))
    // static serving added in Task 2
    .with_state(state);
```

- [ ] **Step 5: Build and test**

```bash
cargo build --package nexus-gateway
# Manual test: start server + gateway, curl through gateway
curl -X POST http://localhost:9090/api/auth/login -H "Content-Type: application/json" -d '{"email":"test@test.com","password":"test"}'
```

- [ ] **Step 6: Commit**

---

### Task 2: Gateway Static File Serving

**Files:**
- Modify: `nexus-gateway/src/main.rs`

**Purpose:** Serve the frontend `dist/` directory. SPA fallback serves `index.html` for all non-API, non-WS routes.

- [ ] **Step 1: Add static serving**

In `main.rs`, read `NEXUS_FRONTEND_DIR` env var. Add `tower_http::services::ServeDir` as fallback:

```rust
use tower_http::services::{ServeDir, ServeFile};

let frontend_dir = std::env::var("NEXUS_FRONTEND_DIR")
    .unwrap_or_else(|_| "../nexus-frontend/dist".to_string());

let app = Router::new()
    .route("/ws/nexus", get(gateway::nexus_ws_handler))
    .route("/ws/chat", get(browser::browser_ws_handler))
    .route("/api/{*path}", any(proxy::api_proxy))
    .fallback_service(
        ServeDir::new(&frontend_dir)
            .fallback(ServeFile::new(format!("{}/index.html", frontend_dir)))
    )
    .with_state(state);
```

- [ ] **Step 2: Add CORS for development**

```rust
use tower_http::cors::{CorsLayer, Any};

let cors = CorsLayer::new()
    .allow_origin(Any)
    .allow_methods(Any)
    .allow_headers(Any);

let app = Router::new()
    // ... routes ...
    .layer(cors)
    .with_state(state);
```

- [ ] **Step 3: Build and test**

```bash
cargo build --package nexus-gateway
```

- [ ] **Step 4: Commit**

---

### Task 3: Gateway WebSocket Enhancement

**Files:**
- Modify: `nexus-gateway/src/browser.rs`
- Modify: `nexus-gateway/src/protocol.rs`
- Modify: `nexus-gateway/src/gateway.rs`
- Modify: `nexus-gateway/src/state.rs`

**Purpose:** Enhance browser WebSocket with session management (new_session, switch_session), progress forwarding, and user-scoped session IDs.

- [ ] **Step 1: Update protocol types**

In `protocol.rs`, update:

```rust
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum BrowserInbound {
    #[serde(rename = "message")]
    Message { content: String, media: Option<Vec<String>> },
    #[serde(rename = "new_session")]
    NewSession,
    #[serde(rename = "switch_session")]
    SwitchSession { session_id: String },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum BrowserOutbound {
    #[serde(rename = "message")]
    Message { content: String, session_id: String, media: Option<Vec<String>> },
    #[serde(rename = "progress")]
    Progress { content: String, session_id: String },
    #[serde(rename = "error")]
    Error { reason: String },
    #[serde(rename = "session_created")]
    SessionCreated { session_id: String },
    #[serde(rename = "session_switched")]
    SessionSwitched { session_id: String },
}
```

- [ ] **Step 2: Update state for session tracking**

In `state.rs`, add user → session mapping:

```rust
pub struct BrowserConnection {
    pub tx: mpsc::Sender<String>,
    pub user_id: String,
    pub session_id: String,
}

// Change browser_conns to store BrowserConnection:
pub browser_conns: Arc<DashMap<String, BrowserConnection>>,
```

- [ ] **Step 3: Update browser.rs for session management**

Enhance the browser WebSocket handler:
- On connect: extract user_id from JWT, create initial session_id as `gateway:{user_id}:{uuid}`
- On `NewSession`: generate new session_id, update mapping, send `SessionCreated`
- On `SwitchSession`: validate session_id, update mapping, send `SessionSwitched`
- On `Message`: forward with current session_id
- Include `session_id` in all outbound messages so frontend can correlate

- [ ] **Step 4: Update gateway.rs for progress forwarding**

When nexus-server sends messages back, include progress metadata. Update `NexusInbound` to support progress:

```rust
#[serde(rename = "send")]
Send { chat_id: String, content: String, media: Option<Vec<String>>, metadata: Option<serde_json::Value> },
```

When routing to browser, check metadata for `_progress` flag and send as `BrowserOutbound::Progress` instead of `Message`.

- [ ] **Step 5: Update server-side GatewayChannel**

In `nexus-server/src/channels/gateway.rs`, update `send()` and `send_progress()` to include metadata in the outbound message so gateway can distinguish progress from final messages.

- [ ] **Step 6: Build and test**

```bash
cargo build --package nexus-gateway
cargo build --package nexus-server
```

- [ ] **Step 7: Commit**

---

### Task 4: Server File Upload API

**Files:**
- Modify: `nexus-server/src/api.rs`
- Modify: `nexus-server/src/main.rs`

**Purpose:** Add `POST /api/files` (multipart upload) and `GET /api/files/{file_id}` (download) endpoints.

- [ ] **Step 1: Add upload endpoint**

In `api.rs`:

```rust
use axum::extract::Multipart;

/// POST /api/files — upload a file
pub async fn upload_file(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    mut multipart: Multipart,
) -> Response {
    let user_id = &claims.sub;
    let upload_dir = format!("/tmp/nexus-uploads/{}", user_id);
    tokio::fs::create_dir_all(&upload_dir).await.ok();

    while let Ok(Some(field)) = multipart.next_field().await {
        let file_name = field.file_name().unwrap_or("upload").to_string();
        let data = field.bytes().await.map_err(|_| /* error */)?;

        if data.len() > 25 * 1024 * 1024 {
            return ApiError::new(ErrorCode::ValidationFailed, "File too large (max 25MB)").into_response();
        }

        let file_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let safe_name = format!("{}_{}", file_id, file_name);
        let file_path = format!("{}/{}", upload_dir, safe_name);

        tokio::fs::write(&file_path, &data).await
            .map_err(|e| /* error */)?;

        return Json(json!({
            "file_id": file_id,
            "file_name": file_name,
            "file_path": file_path,
        })).into_response();
    }

    ApiError::new(ErrorCode::ValidationFailed, "No file provided").into_response()
}
```

- [ ] **Step 2: Add download endpoint**

```rust
/// GET /api/files/{file_id}
pub async fn download_file(
    State(state): State<AppState>,
    claims: axum::Extension<Claims>,
    Path(file_id): Path<String>,
) -> Response {
    // Search in both upload and media directories
    let user_dir = format!("/tmp/nexus-uploads/{}", claims.sub);
    let media_dir = "/tmp/nexus-media";

    // Find file matching file_id prefix
    // Serve with appropriate content-type
    // ...
}
```

- [ ] **Step 3: Register routes**

In `main.rs`:
```rust
.route("/api/files", axum::routing::post(api::upload_file))
.route("/api/files/{file_id}", axum::routing::get(api::download_file))
```

- [ ] **Step 4: Build and test**
- [ ] **Step 5: Commit**

---

### Task 5: Agent Loop Progress Hints

**Files:**
- Modify: `nexus-server/src/agent_loop.rs`

**Purpose:** Emit `⏳ Thinking...`, `⏳ Analyzing results...`, and `💬 Composing response...` at key points.

- [ ] **Step 1: Add progress helper**

```rust
async fn emit_progress(state: &AppState, channel: &str, chat_id: &str, hint: &str) {
    let mut metadata = HashMap::new();
    metadata.insert("_progress".to_string(), json!(true));
    let _ = state.bus.publish_outbound(OutboundEvent {
        channel: channel.to_string(),
        chat_id: chat_id.to_string(),
        content: hint.to_string(),
        media: Vec::new(),
        metadata,
    }).await;
}
```

- [ ] **Step 2: Add hints in run_single_turn**

Before LLM call:
```rust
emit_progress(state, &event.channel, &event.chat_id, "⏳ Thinking...").await;
```

- [ ] **Step 3: Add hints in execute_tool_calls_loop**

After tool results collected, before next LLM call:
```rust
emit_progress(state, event_channel, event_chat_id, "⏳ Analyzing results...").await;
```

When final response (finish_reason = "stop"):
```rust
emit_progress(state, event_channel, event_chat_id, "💬 Composing response...").await;
```

- [ ] **Step 4: Build and test**
- [ ] **Step 5: Commit**

---

## Phase 2: Frontend (React + TypeScript)

### Task 6: Frontend Scaffold

**Files:**
- Create: `nexus-frontend/` (entire directory)

**Purpose:** Initialize React + TypeScript + Vite + Tailwind + shadcn/ui project.

- [ ] **Step 1: Create Vite project**

```bash
cd /home/yucheng/Documents/GitHub/NEXUS
npm create vite@latest nexus-frontend -- --template react-ts
cd nexus-frontend
npm install
```

- [ ] **Step 2: Install dependencies**

```bash
npm install react-router-dom zustand react-markdown
npm install -D tailwindcss @tailwindcss/vite
```

- [ ] **Step 3: Configure Tailwind**

In `src/index.css`:
```css
@import "tailwindcss";
```

In `vite.config.ts`:
```typescript
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    proxy: {
      '/api': 'http://localhost:9090',
      '/ws': { target: 'ws://localhost:9090', ws: true },
    },
  },
})
```

- [ ] **Step 4: Set up shadcn/ui**

```bash
npx shadcn@latest init
npx shadcn@latest add button input card tabs dialog table textarea badge
```

- [ ] **Step 5: Create base files**

Create:
- `src/lib/api.ts` — centralized fetch wrapper with JWT
- `src/lib/useWebSocket.ts` — WebSocket hook
- `src/lib/store.ts` — Zustand store (auth state, user info)
- `src/App.tsx` — React Router setup
- `src/pages/LoginPage.tsx` — placeholder
- `src/pages/ChatPage.tsx` — placeholder
- `src/pages/SettingsPage.tsx` — placeholder
- `src/pages/AdminPage.tsx` — placeholder

- [ ] **Step 6: Set up routing**

```typescript
// App.tsx
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'

function App() {
  const token = localStorage.getItem('jwt')
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/login" element={<LoginPage />} />
        <Route path="/chat" element={token ? <ChatPage /> : <Navigate to="/login" />} />
        <Route path="/settings" element={token ? <SettingsPage /> : <Navigate to="/login" />} />
        <Route path="/admin" element={token ? <AdminPage /> : <Navigate to="/login" />} />
        <Route path="*" element={<Navigate to="/chat" />} />
      </Routes>
    </BrowserRouter>
  )
}
```

- [ ] **Step 7: Verify dev server**

```bash
npm run dev
# Should open http://localhost:5173 with placeholder pages
```

- [ ] **Step 8: Commit**

---

### Task 7: Login/Register Page

**Files:**
- Modify: `nexus-frontend/src/pages/LoginPage.tsx`
- Modify: `nexus-frontend/src/lib/api.ts`
- Modify: `nexus-frontend/src/lib/store.ts`

**Purpose:** Auth flow: email + password form, toggle login/register, store JWT, redirect to /chat.

- [ ] **Step 1: Create API client**

```typescript
// src/lib/api.ts
export async function apiRequest(path: string, options?: RequestInit) {
  const token = localStorage.getItem('jwt')
  const res = await fetch(path, {
    ...options,
    headers: {
      'Content-Type': 'application/json',
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...options?.headers,
    },
  })
  if (res.status === 401) {
    localStorage.removeItem('jwt')
    window.location.href = '/login'
  }
  return res
}

export async function login(email: string, password: string) {
  const res = await apiRequest('/api/auth/login', {
    method: 'POST',
    body: JSON.stringify({ email, password }),
  })
  if (!res.ok) throw new Error(await res.text())
  return res.json()
}

export async function register(email: string, password: string) {
  const res = await apiRequest('/api/auth/register', {
    method: 'POST',
    body: JSON.stringify({ email, password }),
  })
  if (!res.ok) throw new Error(await res.text())
  return res.json()
}
```

- [ ] **Step 2: Create auth store**

```typescript
// src/lib/store.ts
import { create } from 'zustand'

interface AuthState {
  token: string | null
  isAdmin: boolean
  setAuth: (token: string, isAdmin: boolean) => void
  logout: () => void
}

export const useAuthStore = create<AuthState>((set) => ({
  token: localStorage.getItem('jwt'),
  isAdmin: false,
  setAuth: (token, isAdmin) => {
    localStorage.setItem('jwt', token)
    set({ token, isAdmin })
  },
  logout: () => {
    localStorage.removeItem('jwt')
    set({ token: null, isAdmin: false })
  },
}))
```

- [ ] **Step 3: Build LoginPage**

Full login/register form with shadcn/ui Card, Input, Button. Toggle between modes. Error display. Redirect on success.

- [ ] **Step 4: Test and commit**

---

### Task 8: Chat Page

**Files:**
- Create: `nexus-frontend/src/pages/ChatPage.tsx`
- Create: `nexus-frontend/src/components/SessionSidebar.tsx`
- Create: `nexus-frontend/src/components/MessageList.tsx`
- Create: `nexus-frontend/src/components/MessageInput.tsx`
- Create: `nexus-frontend/src/components/DeviceStatus.tsx`
- Create: `nexus-frontend/src/components/ProgressHint.tsx`
- Modify: `nexus-frontend/src/lib/useWebSocket.ts`

**Purpose:** Main chat interface with WebSocket, session sidebar, message rendering, progress hints, device status.

- [ ] **Step 1: Create WebSocket hook**

Full implementation of `useWebSocket.ts` with:
- Connect using JWT from localStorage
- Handle message/progress/error/session_created types
- Auto-reconnect on disconnect
- send(), newSession(), switchSession() methods

- [ ] **Step 2: Create SessionSidebar**

- Fetch sessions from `GET /api/sessions`
- Display with channel icons (💬/🎮/⏰)
- "New Chat" button sends `new_session` via WS
- Click to switch session
- Cross-channel sessions marked as read-only

- [ ] **Step 3: Create MessageList**

- User messages right-aligned with blue background
- Agent messages left-aligned with gray background
- Markdown rendering for agent messages (react-markdown)
- ProgressHint component shown at bottom during agent processing
- Auto-scroll to bottom on new message

- [ ] **Step 4: Create MessageInput**

- Text input with send button
- File upload button (calls `POST /api/files`, then sends message with media reference)
- Enter to send, Shift+Enter for newline
- Disabled while agent is processing

- [ ] **Step 5: Create DeviceStatus**

- Fetch device list from `GET /api/devices`
- Show green/red dots with device names
- Refresh periodically (every 30s)

- [ ] **Step 6: Compose ChatPage**

Assemble all components in the page layout:
- Sidebar left (collapsible on mobile)
- Chat area center
- Device status top-right corner

- [ ] **Step 7: Load session history**

When switching sessions, fetch message history from `GET /api/sessions/{id}/messages` and render.

- [ ] **Step 8: Test and commit**

---

### Task 9: Settings Page

**Files:**
- Create: `nexus-frontend/src/pages/SettingsPage.tsx`
- Create: `nexus-frontend/src/components/settings/ProfileTab.tsx`
- Create: `nexus-frontend/src/components/settings/DevicesTab.tsx`
- Create: `nexus-frontend/src/components/settings/SkillsTab.tsx`
- Create: `nexus-frontend/src/components/settings/SoulTab.tsx`
- Create: `nexus-frontend/src/components/settings/PreferencesTab.tsx`
- Create: `nexus-frontend/src/components/settings/CronTab.tsx`

**Purpose:** User settings with tabbed layout. Each tab calls the corresponding REST API.

- [ ] **Step 1: Create SettingsPage with tabs**

Use shadcn/ui Tabs component. Each tab renders its own component.

- [ ] **Step 2: ProfileTab**

- `GET /api/user/profile` → display email, user_id, created_at
- Read-only for now

- [ ] **Step 3: DevicesTab**

- `GET /api/devices` → table of devices with status
- Per-device: `GET/PATCH /api/devices/{name}/policy` for FsPolicy
- Per-device: `GET/PUT /api/devices/{name}/mcp` for MCP config
- `GET /api/device-tokens` → list tokens
- `POST /api/device-tokens` → create new
- `DELETE /api/device-tokens/{token}` → revoke

- [ ] **Step 4: SkillsTab**

- `GET /api/skills` → list skills
- `POST /api/skills` → create (name + SKILL.md content textarea)
- `DELETE /api/skills/{name}` → remove

- [ ] **Step 5: SoulTab**

- `GET /api/user/soul` → load soul text
- `PATCH /api/user/soul` → save (textarea)

- [ ] **Step 6: PreferencesTab**

- `GET /api/user/preferences` → load JSON
- `PATCH /api/user/preferences` → save (JSON editor textarea)

- [ ] **Step 7: CronTab**

- `GET /api/cron-jobs` → table of jobs (name, schedule, next_run, status)
- `POST /api/cron-jobs` → create form (message, cron_expr/every_seconds/at, timezone)
- `DELETE /api/cron-jobs/{id}` → remove
- `PATCH /api/cron-jobs/{id}` → toggle enable/disable

- [ ] **Step 8: Test and commit**

---

### Task 10: Admin Page

**Files:**
- Create: `nexus-frontend/src/pages/AdminPage.tsx`
- Create: `nexus-frontend/src/components/admin/LlmConfigTab.tsx`
- Create: `nexus-frontend/src/components/admin/EmbeddingConfigTab.tsx`
- Create: `nexus-frontend/src/components/admin/ServerMcpTab.tsx`
- Create: `nexus-frontend/src/components/admin/DefaultSoulTab.tsx`

**Purpose:** Admin-only page for system configuration.

- [ ] **Step 1: Create AdminPage with admin guard**

Check `is_admin` from JWT/store. Redirect non-admins to /chat.

- [ ] **Step 2: LlmConfigTab**

- `GET /api/llm-config` → load current config
- `PUT /api/llm-config` → save form (model, api_base, api_key, context_window, max_output_tokens)

- [ ] **Step 3: EmbeddingConfigTab**

- `GET /api/embedding-config` → load
- `PUT /api/embedding-config` → save form (model, api_base, api_key, max_input_length, max_concurrency)

- [ ] **Step 4: ServerMcpTab**

- `GET /api/server-mcp` → list server MCP servers
- `PUT /api/server-mcp` → update (JSON editor for MCP server entries)

- [ ] **Step 5: DefaultSoulTab**

- `GET /api/admin/default-soul` → load
- `PUT /api/admin/default-soul` → save (textarea)

- [ ] **Step 6: Test and commit**

---

## Dependency Order

```
Phase 1 (Gateway — Rust):
  Task 1 (REST proxy) → Task 2 (static serving) → Task 3 (WebSocket)
  Task 4 (file upload) — independent of 1-3
  Task 5 (progress hints) — independent of 1-4

Phase 2 (Frontend — React):
  Task 6 (scaffold) → Task 7 (login) → Task 8 (chat) → Task 9 (settings) → Task 10 (admin)

Phase 1 and Phase 2 can be parallelized:
  - Tasks 1-2 must be done before Task 7 (login needs proxy)
  - Tasks 3+5 must be done before Task 8 (chat needs WebSocket + progress)
  - Task 4 should be done before Task 8 (chat needs file upload)
```

Recommended parallel execution:
- **Batch 1:** Tasks 1, 4, 5 (all independent gateway/server changes)
- **Batch 2:** Tasks 2, 3 (gateway static + WebSocket, depends on 1)
- **Batch 3:** Tasks 6, 7 (frontend scaffold + login)
- **Batch 4:** Task 8 (chat — the big one)
- **Batch 5:** Tasks 9, 10 (settings + admin, can be parallel)
