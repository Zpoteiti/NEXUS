# nexus-frontend

React web UI for NEXUS.

## Tech Stack

- **React 19** + **React Router 7** -- SPA with client-side routing
- **Zustand** -- lightweight state management (auth state, etc.)
- **Tailwind CSS 4** -- styling, dark theme throughout
- **react-markdown** + **remark-gfm** + **react-syntax-highlighter** -- renders agent replies as rich markdown with GFM tables, code blocks with syntax highlighting (One Dark theme)
- **Lucide React** -- icon set
- **Vite 8** -- dev server and build tooling
- **TypeScript 5.9**

## Dev Setup

```bash
npm install
npm run dev
```

Opens on `http://localhost:5173`. The Vite dev server proxies `/api/*` and `/ws/*` to `localhost:9090` (the gateway), so make sure the gateway is running.

## Build for Production

```bash
npm run build
```

Output goes to `dist/`. The gateway serves this directory as static files in production -- you don't need a separate web server.

## Pages

### `/login` -- Login & Registration

Email/password auth with a toggle between login and register modes. Registration optionally accepts an admin token for bootstrapping. JWT is stored in localStorage.

### `/chat` -- Chat Interface

The main page. Features:

- **Markdown rendering** -- agent replies render as full markdown (code blocks, tables, lists, etc.)
- **Progress indicators** -- shows intermediate "thinking" updates from the agent while it works
- **Session sidebar** -- list of past sessions with timestamps, create new sessions, switch between them
- **Device status** -- shows connected nexus-client devices and their online/offline state
- **File uploads** -- attach files to messages (uploaded via REST API, referenced by file ID)
- **Collapsible sidebar** -- toggle the session list for more chat space

### `/settings` -- User Settings

Tabs:

- **Profile** -- view account info (email, role, creation date)
- **Devices** -- manage connected nexus-client devices
- **Skills** -- install skills from GitHub, manage installed skills, toggle always-on (per-user isolated, stored on server)
- **Soul** -- configure your agent's personality/system prompt
- **Memory** -- view and manage the agent's memory about you
- **Cron Jobs** -- set up scheduled/recurring agent tasks

### `/admin` -- Admin Panel

Admin-only (redirects non-admins to `/chat`). Tabs:

- **LLM Config** -- configure the AI provider (API base URL, model, API key, context window)
- **Server MCP** -- manage server-side MCP tool servers
- **Default Soul** -- set the default system prompt for all users
- **Rate Limit** -- configure per-user rate limiting

## How It Connects

The frontend talks exclusively to the gateway -- never directly to the nexus-server.

- **WebSocket** at `/ws/chat` for real-time chat (messages, progress updates, session management). JWT is passed as a `?token=` query parameter on the upgrade request.
- **REST** at `/api/*` for everything else (auth, sessions, settings, admin). JWT is sent as a `Bearer` token in the `Authorization` header. The gateway proxies these to the server.

## Environment

No `VITE_*` env vars needed. The gateway URL is configured in `vite.config.ts` as a dev proxy:

```ts
server: {
  proxy: {
    '/api': 'http://localhost:9090',
    '/ws': { target: 'ws://localhost:9090', ws: true },
  },
}
```

In production, the frontend is served by the gateway itself, so all paths resolve naturally without any proxy config.
