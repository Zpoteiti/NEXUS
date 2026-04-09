# NEXUS

**Run AI agents on one server. Execute tools on any machine.**

NEXUS is a distributed AI agent platform built in Rust. The core idea: separate *thinking* (LLM orchestration) from *doing* (tool execution). Your agent brain lives on a central server. Your tools run on remote machines — dev laptops, production servers, cloud VMs, wherever you need them.

```
You ── Browser ── Gateway ── Server ── Client (your laptop)
                                    ├── Client (prod server)
                                    ├── Client (cloud VM)
                                    └── Client (...)
```

Heavily inspired by [nanobot](https://github.com/nanobot-ai/nanobot) — a brilliant Python-based personal AI assistant framework. NEXUS takes the patterns we learned from studying nanobot (multi-channel support, tool orchestration, memory, skills, cron, security) and re-architects them for distributed, multi-user, multi-user, multi-machine deployment in Rust.

## Why NEXUS?

Most AI agent frameworks assume everything runs on one machine. That breaks when:

- You want one agent managing tools across **multiple machines** (your laptop + a server + a cloud instance)
- You need **hundreds of users** sharing a single platform with proper isolation
- You want your agent accessible from a **web browser, Discord, Telegram** — all at once
- You need **real security** — sandboxed execution, rate limiting, SSRF protection, env isolation

NEXUS solves all of these by splitting the architecture: the server handles the agent loop, memory, sessions, and LLM calls. Lightweight clients on remote machines just expose and execute tools.

## Features

- **Distributed tool execution** — connect any machine as an execution node
- **Multi-channel** — talk to your agent via web UI, Discord, or API (Telegram, Slack planned)
- **ReAct agent loop** — up to 200 iterations, tool call deduplication, automatic rethink
- **Per-device security policies** — sandbox or unrestricted filesystem access
- **Shell sandbox** — dangerous pattern blocking, env isolation, optional bubblewrap (Linux)
- **MCP support** — mount any MCP server on any client, tools auto-discovered
- **Skills system** — install from GitHub (via web UI or agent-driven), per-user isolated, always-on or on-demand with progressive disclosure
- **Memory & context compression** — persistent memory per user, automatic conversation compression
- **Cron jobs** — schedule recurring tasks, one-shot reminders, cross-channel delivery
- **Rate limiting** — per-user message throttle, admin-configurable
- **Built for scale** — DashMap-based routing, concurrent DB pool, designed for 1K users and 500 concurrent sessions

## Quick Start

### Prerequisites

- Rust 1.85+ (edition 2024)
- PostgreSQL 15+
- Node.js 18+ (for building the web frontend)

### 1. Build the frontend

```bash
cd nexus-frontend
npm install && npm run build
```

### 2. Start the server and gateway

```bash
# Set required env vars (or use a .env file)
export DATABASE_URL=postgres://user:pass@localhost/nexus
export ADMIN_TOKEN=your-admin-secret
export JWT_SECRET=your-jwt-secret-at-least-32-chars
export SERVER_PORT=8080
export GATEWAY_PORT=9090
export NEXUS_GATEWAY_WS_URL=ws://localhost:9090/ws/nexus
export NEXUS_GATEWAY_TOKEN=your-gateway-secret
export NEXUS_SERVER_API_URL=http://localhost:8080

# Start the server
cd nexus-server && cargo run &

# Start the gateway (serves the web UI automatically)
cd nexus-gateway && cargo run &
```

### 3. Set up via the web UI

Open `http://localhost:9090` in your browser. The gateway serves the frontend automatically. The first user to register becomes the admin and is guided through the setup wizard:

1. **Register** your admin account
2. **Configure LLM** — point to any OpenAI-compatible API (OpenAI, Anthropic via proxy, local models, etc.)
3. **Set rate limits** and other platform settings
4. **Create a device token** — gives you a token to connect your first client

### 4. Connect a client

```bash
cd nexus-client

export NEXUS_SERVER_WS_URL=ws://localhost:8080/ws
export NEXUS_AUTH_TOKEN=<paste your device token here>

cargo run
```

Your machine is now an execution node. The agent can run shell commands, read/write files, and use any MCP servers you configure — all on your machine. Connect as many machines as you want.

### 5. Start chatting

Go back to the web UI and send your first message. The agent is ready.

## Architecture

```
nexus-common/     Shared protocol types, error codes, constants
nexus-server/     Orchestration hub — agent loop, DB, auth, channels, tools
nexus-client/     Execution node — tool runtime, MCP, shell sandbox
nexus-gateway/    Browser WebSocket bridge — proxies between web UI and server
nexus-frontend/   React web UI — chat, settings, admin panel
```

| Component | Role | Scales to |
|-----------|------|-----------|
| **Server** | Agent brain, LLM calls, memory, sessions | 1K users, 500 concurrent sessions |
| **Client** | Tool execution, file I/O, shell commands | Thousands of connections per server |
| **Gateway** | Browser ↔ Server bridge, JWT auth | Thousands of browser sessions |
| **Frontend** | Chat UI, settings, admin | Served as static files |

## How It Works

1. You send a message (from browser or Discord and more in the future — all chat goes through WebSocket)
2. The server's agent loop builds a system prompt with your soul, memory, available tools, and skills
3. The LLM responds — either with text (done) or tool calls (continue)
4. Tool calls get routed to the right client by device name
5. The client executes the tool (with security guards) and returns the result
6. Loop back to step 3 until the LLM says it's done (or hits 200 iterations)

All messages, tool results, and memory are persisted in PostgreSQL. Context compression kicks in automatically when the conversation gets long.

## Security

NEXUS uses a server-authoritative security model — the server defines policy, clients enforce it.

- **Filesystem policy** — per-device: Sandbox (workspace only) or Unrestricted (full access)
- **Shell guards** — dangerous pattern blocking (`rm -rf`, `mkfs`, fork bombs, etc.), environment variable isolation (always on, even in Unrestricted mode)
- **Bubblewrap sandbox** — optional Linux process isolation, workspace rw + system ro + secrets hidden
- **SSRF protection** — blocks private IPs, link-local, CGNAT on both client shell and server web_fetch
- **Rate limiting** — per-user message throttle, admin-configurable
- **Untrusted content flagging** — web_fetch results marked as data, not instructions
- **Channel access control** — Discord allowlist per user

Security model inspired by [nanobot's defense-in-depth approach](https://github.com/nanobot-ai/nanobot/blob/main/SECURITY.md).

## Acknowledgements

NEXUS wouldn't exist without [nanobot](https://github.com/nanobot-ai/nanobot). We're not affiliated with the nanobot project — we're just fans who learned a ton from studying their codebase. Many design decisions in NEXUS — the tool execution model, cron scheduling, skill system, SSRF protection, shell guardrails, channel architecture — were directly informed by nanobot's battle-tested patterns. If you need a lightweight single-machine AI assistant, check out nanobot. If you need distributed multi-user deployment, that's where NEXUS comes in.

## License

MIT
