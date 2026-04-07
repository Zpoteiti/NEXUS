# nexus-frontend

Web UI for NEXUS, built with React 19, TypeScript, Vite, and Tailwind CSS 4.

## Setup

```bash
npm install
npm run dev      # start dev server (Vite HMR)
npm run build    # production build (outputs to dist/)
npm run preview  # preview production build
npm run lint     # run ESLint
```

## Pages

- **Login** -- JWT authentication
- **Chat** -- Main interface with sessions sidebar, message input with file upload, markdown rendering, device status indicators, and progress hints during agent processing
- **Settings** -- User configuration across 6 tabs
- **Admin** -- System administration across 4 tabs (device management, tokens, skills, etc.)

## Stack

- **React 19** + **TypeScript 5.9**
- **Vite 8** (build tool)
- **Tailwind CSS 4** (styling)
- **Zustand** (state management)
- **react-router-dom** (SPA routing)
- **react-markdown** (message rendering)

## Architecture

```
src/
  pages/         -- LoginPage, ChatPage, SettingsPage, AdminPage
  lib/
    api.ts       -- REST API client
    store.ts     -- Zustand store
    useWebSocket.ts -- WebSocket hook for chat
  App.tsx        -- Router setup
  main.tsx       -- Entry point
```

The frontend connects to nexus-gateway which serves it in production. During development, Vite proxies API and WebSocket requests.
