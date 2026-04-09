# Frontend Design Decisions

## Why React

- Ecosystem maturity. Finding a React dev or a React library for any random thing is trivial. The project needs markdown rendering, syntax highlighting, routing, state management -- all well-covered.
- React 19 used here (see `package.json`: `"react": "^19.2.4"`). Server components aren't needed (this is a pure SPA talking over WebSocket), but the improved hooks and rendering are welcome.
- Team familiarity. The alternative (Svelte/Vue) would add onboarding friction for contributors without meaningful bundle size wins given the app's complexity.

## Why Zustand (not Redux/Context)

`store.ts` uses `create` from Zustand for auth state:

```ts
export const useAuthStore = create<AuthState>((set) => ({
  token: localStorage.getItem('jwt'),
  ...
}))
```

- **Minimal boilerplate.** One `create()` call vs Redux's actions/reducers/selectors/middleware. The entire auth store is 50 lines.
- **No provider wrapping.** Zustand stores are plain imports -- no `<Provider>` at the app root. Components subscribe with `useAuthStore()` directly.
- **Selective re-renders.** Zustand's selector pattern (`useAuthStore(s => s.token)`) prevents unnecessary re-renders. React Context re-renders every consumer on any state change.
- **Small bundle.** ~2KB vs Redux Toolkit's ~30KB+.

## Why Tailwind (not CSS modules/styled-components)

- Co-location. Styles live in the JSX, not in separate files or tagged template literals. Faster iteration for a chat UI where layout changes are frequent.
- Using `@tailwindcss/vite` plugin (v4) -- zero PostCSS config, built into the Vite pipeline.
- Consistency. Tailwind's design tokens (spacing, colors) prevent one-off values. Dark theme is handled via Tailwind's dark mode utilities.
- No runtime cost. Unlike styled-components, Tailwind is pure CSS at build time.

## Why react-markdown + remark-gfm

Agent responses contain markdown (code blocks, tables, lists, links). Rendering options:

- **`dangerouslySetInnerHTML` with a markdown lib**: XSS vector. No.
- **react-markdown**: renders markdown to React elements (no `innerHTML`). Extensible via remark/rehype plugins. `remark-gfm` adds GitHub-flavored extensions (tables, strikethrough, task lists).
- **react-syntax-highlighter**: used alongside for fenced code blocks with language-specific highlighting.

This stack handles everything the agent outputs without custom parsing.

## Why Vite proxy to gateway (not direct API calls)

`vite.config.ts`:

```ts
server: {
  proxy: {
    '/api': 'http://localhost:9090',
    '/ws': { target: 'ws://localhost:9090', ws: true },
  },
}
```

- **No CORS in dev.** Browser makes requests to `localhost:5173` (Vite dev server), which proxies to the gateway at `:9090`. Same-origin, no preflight requests.
- **Production parity.** In production, the gateway serves the built frontend directly from `NEXUS_FRONTEND_DIR`. All paths (`/api/*`, `/ws/*`, static files) go through the same origin. The Vite proxy mirrors this topology.
- **No hardcoded URLs.** The frontend never references `http://localhost:9090` directly. The WebSocket URL is derived from `window.location`: `${protocol}//${window.location.host}/ws/chat?token=${token}`. Works in any deployment without env var injection.

## Stack Summary

| Concern | Choice | Version |
|---|---|---|
| Framework | React | 19.x |
| State | Zustand | 5.x |
| Styling | Tailwind CSS | 4.x |
| Markdown | react-markdown + remark-gfm | 10.x / 4.x |
| Code highlighting | react-syntax-highlighter | 16.x |
| Icons | lucide-react | 1.x |
| Routing | react-router-dom | 7.x |
| Build | Vite | 8.x |
| Language | TypeScript | 5.9 |
