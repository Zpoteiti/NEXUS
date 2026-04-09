# nexus-client

Lightweight execution node -- connects to nexus-server via WebSocket, registers local tools, and executes tool calls on behalf of the agent.

## Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `NEXUS_SERVER_WS_URL` | Yes | -- | Server WebSocket URL (e.g. `wss://nexus.example.com/ws`). Alias: `NEXUS_WS_URL` |
| `NEXUS_AUTH_TOKEN` | Yes | -- | Device token (must start with `nexus_dev_` + 32 random chars). Alias: `NEXUS_DEVICE_TOKEN` |

Everything else is configured per-device through the web UI (Settings > Devices). The server sends these to the client on connect:

| Setting | Description |
|---------|-------------|
| Workspace path | Root directory for file operations |
| Filesystem policy | Sandbox or Unrestricted |
| MCP servers | External tool servers to launch and discover |
| Tool timeouts | Global execution timeout, shell timeout, filesystem tool timeout |
| SSRF whitelist | CIDR ranges allowed to bypass private IP blocking in shell commands |

## Built-in Tools

These are registered automatically on connect. The server's agent can call them on any device.

| Tool | Description |
|------|-------------|
| `shell` | Execute a shell command on the device. Returns stdout/stderr. Supports `timeout_sec` (per-device configurable via web UI) and `working_dir` (must be within workspace). |
| `read_file` | Read file contents. Returns numbered lines for text, metadata for images. Supports `offset` and `limit` for pagination. |
| `write_file` | Write content to a file. Creates parent directories if needed. |
| `edit_file` | Targeted string replacement -- finds `old_string` (must appear exactly once) and replaces it with `new_string`. |
| `list_dir` | List directory contents. Supports `recursive` mode. Auto-ignores noise dirs (`.git`, `node_modules`, `__pycache__`, etc.). Max 200 entries by default. |
| `glob` | Find files by pattern (e.g., `**/*.rs`, `src/**/*.test.ts`). Returns matching file paths. |
| `grep` | Search file contents with regex. Returns matching lines with file paths and line numbers. |

All filesystem tools enforce the device's `FsPolicy`. The `shell` tool additionally runs through guardrails validation and optional bwrap sandboxing.

## MCP Support

MCP servers are configured per-device through the web UI (Settings > Devices), not locally on the client. Each device's MCP config is a list of servers with name, command, args, and transport type.

Supported transport types: `stdio` (default), `sse`, `streamableHttp`.

On connect (and reconnect), the client reads its MCP config from the server, launches the configured MCP servers locally, discovers their tools via `tools/list`, and registers them with the prefix `mcp_{server_name}_*`. Each tool call is forwarded to the owning MCP server session.

## Security

### Filesystem Policy (FsPolicy)

Every device has an `FsPolicy` that controls what the agent can access. Set it via the web UI (Settings > Devices). Two modes:

| Policy | Filesystem | Shell | Description |
|--------|-----------|-------|-------------|
| **Sandbox** (default) | Read/write only within workspace | Guardrails active: dangerous command regex, SSRF detection, path traversal blocked. On Linux with `bwrap` installed, shell commands run inside a bubblewrap namespace (workspace r/w, system dirs read-only, config/secrets hidden behind tmpfs). | Locked down. Good for untrusted or shared environments. |
| **Unrestricted** | All paths allowed, no restrictions. | Guardrails skipped entirely. No command filtering, no SSRF checks. Env isolation still applies (commands get a minimal `PATH`, `HOME`, `LANG`, `TERM`). | Full trust. Use only on personal machines you control. |

### Guardrails (Sandbox mode)

Active in Sandbox mode. Checks every shell command before execution:

- **Dangerous command deny-list**: regex patterns block `rm -rf`, `dd if=`, `format`, `shutdown`/`reboot`, fork bombs, direct `/dev/sd*` writes, `mkfifo`/`mknod` in `/dev`
- **SSRF protection**: extracts URLs from commands, resolves hostnames via async DNS, blocks requests to private/internal networks (127.0.0.0/8, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, link-local, IPv6 loopback/ULA/link-local). Unresolvable hostnames are conservatively rejected.
- **Path traversal**: blocks `../` in shell commands. Absolute paths outside workspace are rejected.

### Bwrap Sandbox (Linux only)

When the policy is Sandbox and `bwrap` (bubblewrap) is installed, shell commands are wrapped in a namespace:

- Workspace directory: read-write bind mount
- `/usr`, `/bin`, `/lib`, `/lib64`: read-only
- `/etc/ssl/certs`, `/etc/resolv.conf`, `/etc/ld.so.cache`: read-only
- `/proc`, `/dev`: mounted
- `/tmp`: tmpfs (isolated)
- Home directory (workspace parent): hidden behind tmpfs (masks `~/.nexus/` config)

### Environment Isolation

All shell commands run with `env_clear()` -- only `PATH` (safe default), `HOME`, `LANG`, and `TERM` are set. No host environment variables leak to agent-executed commands.

## Build & Run

```bash
# Build
cargo build --package nexus-client

# Run
NEXUS_SERVER_WS_URL=ws://127.0.0.1:8080/ws \
NEXUS_AUTH_TOKEN=nexus_dev_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx \
cargo run --package nexus-client

# Lint
cargo clippy --package nexus-client

# Format check
cargo fmt --package nexus-client --check
```
