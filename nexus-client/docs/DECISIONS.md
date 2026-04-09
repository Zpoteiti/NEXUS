# nexus-client Architecture Decisions

## Why bwrap for sandbox (not Docker/Firecracker)

The client runs on user machines -- laptops, dev servers, Raspberry Pis. Docker requires a daemon, root access (or rootless setup), and has significant per-container overhead. Firecracker needs KVM and is designed for multi-tenant cloud, not single-user agents.

Bubblewrap (`bwrap`) is the right tool here:
- **Zero daemon** -- single static binary, `apt install bubblewrap` and done.
- **User-namespace isolation** -- no root required on modern kernels.
- **Microsecond startup** -- it's a namespace wrapper, not a VM.
- **Graceful degradation** -- if bwrap isn't installed, commands run without sandboxing (the other guardrails still apply). No crash, no config change needed.
- **Minimal attack surface** -- bwrap itself is ~2k lines of C, audited by Flatpak/GNOME. Docker's attack surface is orders of magnitude larger.

**Adopted from nanobot.** Our bwrap mount layout is directly based on [nanobot's sandbox.py](https://github.com/nanobot-ai/nanobot). The pattern is clean and battle-tested:
- `/usr`, `/bin`, `/lib`, `/lib64` -- read-only bind mounts
- `/etc/alternatives`, `/etc/ssl/certs`, `/etc/resolv.conf`, `/etc/ld.so.cache` -- read-only (optional, skip if missing)
- `/proc`, `/dev` -- minimal proc/dev
- `/tmp` -- fresh tmpfs
- Workspace parent (e.g., `~`) -- masked with tmpfs (hides `~/.nexus/`, `~/.ssh/`, etc.)
- Workspace directory -- read-write bind mount on top of the tmpfs mask
- Media directory -- read-only bind mount (uploaded files accessible to agent)
- `--new-session` + `--die-with-parent` -- process isolation

We also adopt nanobot's pluggable backend pattern (`_BACKENDS` dict) so future sandbox backends can be added without touching the shell tool.

## Why env isolation is always on (even in Unrestricted mode)

In `shell.rs::run_shell_command`, the process is spawned with `env_clear()` + `min_env()` regardless of `FsPolicy`. This is intentional.

`min_env()` passes through exactly 4 variables:
- `PATH` -- hardcoded safe default (`/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`), NOT inherited from the parent process
- `HOME` -- from the parent (needed for `~` expansion)
- `LANG` -- hardcoded `en_US.UTF-8`
- `TERM` -- hardcoded `xterm-256color`

Everything else is stripped: `AWS_SECRET_ACCESS_KEY`, `GITHUB_TOKEN`, `DATABASE_URL`, the client's own `NEXUS_AUTH_TOKEN`, etc. Even if the agent has `Unrestricted` filesystem access, it shouldn't be able to exfiltrate credentials from the host environment. The filesystem policy and the environment policy are orthogonal security layers.

## Push-based config and MCP discovery (not heartbeat polling)

Config changes (MCP servers, FsPolicy, workspace, shell timeout) are pushed from server to client immediately when an admin updates them — not polled on heartbeat.

The flow:
1. Admin changes device config in web UI → API saves to DB
2. Server resolves the device's WebSocket connection and pushes a `ConfigUpdate` message immediately
3. Client receives the new config, applies it (update policy, workspace, etc.)
4. For MCP changes: client runs `list_tools` on each configured MCP server
5. Client sends `RegisterTools` with full schema (native + MCP tools) back to server
6. Server dedupes tools across devices and injects `device_name` enum for routing

Why push instead of heartbeat polling:
1. **Zero wasted work** -- no MCP discovery every 15s, no config hash checking, no DB queries on heartbeat
2. **Instant propagation** -- config changes take effect in seconds, not up to 15s
3. **Simpler heartbeat** -- heartbeat becomes just `{ status: "online" }` → ack. No config payload in either direction, no dirty flags
4. **Server is the single source of truth** -- client just reacts to pushes

## Why MCP tool names are prefixed with mcp_{server}_{tool}

When multiple MCP servers expose tools, name collisions are inevitable. Two servers might both have a `search` tool, or a `run` tool. The naming convention `mcp_{server_name}_{original_tool_name}` provides:

1. **Namespace isolation** -- `mcp_github_search` vs `mcp_jira_search` are unambiguous.
2. **Routing** -- the `mcp_` prefix tells the executor to route to MCP instead of local tools. The server name segment tells `McpClientManager::call_tool()` which session to forward to (via the reverse index).
3. **Transparency** -- the LLM sees the full prefixed name in the tool schema and can reason about which server it's calling.

The prefix is applied in `McpSession::list_tools()` and reversed in `McpSession::call_tool()` via the `tool_name_map`.
