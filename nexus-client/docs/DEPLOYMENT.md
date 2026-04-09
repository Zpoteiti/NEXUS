# nexus-client Deployment Guide

## Build

```bash
# Release binary (from repo root)
cargo build --release --package nexus-client

# Binary lands at:
# target/release/nexus-client
```

Cross-compilation works with standard Rust targets. The binary is statically linkable with musl:
```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --package nexus-client --target x86_64-unknown-linux-musl
```

## Required Environment Variables

From `config.rs::load_config()` (also reads `.env` via dotenvy):

| Variable | Required | Format | Notes |
|---|---|---|---|
| `NEXUS_SERVER_WS_URL` | yes | `ws://host:port/ws` or `wss://...` | Aliases: `NEXUS_WS_URL` |
| `NEXUS_AUTH_TOKEN` | yes | `nexus_dev_` + 32 random chars | Aliases: `NEXUS_DEVICE_TOKEN`. Created via server admin API |

Optional:

| Variable | Default | Notes |
|---|---|---|
| `RUST_LOG` | (none) | Standard tracing filter, e.g. `info`, `nexus_client=debug` |

> **Note:** Workspace path, filesystem policy, MCP servers, and shell timeout are all configured per-device through the web UI (Settings > Devices). The server sends these to the client on connect.

Token format validation (enforced at startup, panics on mismatch):
- Must start with `nexus_dev_` (the `DEVICE_TOKEN_PREFIX` constant)
- Random segment must be exactly 32 characters (`DEVICE_TOKEN_RANDOM_LEN`)

## Install on a Remote Machine

```bash
# On build machine
cargo build --release --package nexus-client
scp target/release/nexus-client user@remote:/usr/local/bin/

# On remote machine
cat > ~/.nexus/.env << 'EOF'
NEXUS_SERVER_WS_URL=wss://nexus.example.com/ws
NEXUS_AUTH_TOKEN=nexus_dev_abcdef1234567890abcdef1234567890
EOF

# Create workspace
mkdir -p ~/.nexus/workspace

# Test
RUST_LOG=info nexus-client
```

## Auto-start with systemd (Linux)

```ini
# /etc/systemd/system/nexus-client.service
[Unit]
Description=NEXUS Client
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=nexus
Group=nexus
WorkingDirectory=/home/nexus/.nexus
EnvironmentFile=/home/nexus/.nexus/.env
Environment=RUST_LOG=info
ExecStart=/usr/local/bin/nexus-client
Restart=always
RestartSec=5

# Hardening (optional, complements bwrap)
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/home/nexus/.nexus/workspace

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now nexus-client.service
sudo journalctl -u nexus-client -f
```

## Auto-start with launchd (macOS)

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.nexus.client</string>

    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/nexus-client</string>
    </array>

    <key>EnvironmentVariables</key>
    <dict>
        <key>NEXUS_SERVER_WS_URL</key>
        <string>wss://nexus.example.com/ws</string>
        <key>NEXUS_AUTH_TOKEN</key>
        <string>nexus_dev_abcdef1234567890abcdef1234567890</string>
        <key>RUST_LOG</key>
        <string>info</string>
    </dict>

    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>

    <key>StandardOutPath</key>
    <string>/tmp/nexus-client.stdout.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/nexus-client.stderr.log</string>
</dict>
</plist>
```

```bash
cp com.nexus.client.plist ~/Library/LaunchAgents/
launchctl load ~/Library/LaunchAgents/com.nexus.client.plist
launchctl list | grep nexus
```

## MCP Server Configuration

MCP servers are configured **on the server**, not locally on the client. The server pushes the config to the client via `LoginSuccess` and `HeartbeatAck`.

Each `McpServerEntry` includes:

| Field | Type | Notes |
|---|---|---|
| `name` | string | Server identifier, used in tool name prefix |
| `transport_type` | string | `"stdio"` (default, only implemented), `"sse"`, `"streamableHttp"` |
| `command` | string | Binary to spawn (e.g., `npx`, `uvx`, path to binary) |
| `args` | string[] | Command arguments |
| `env` | map | Extra env vars for the MCP server process |
| `tool_timeout` | u64 | Per-tool call timeout in seconds (default: 30) |
| `enabled` | bool | Default true. Set false to disable without removing |

The client:
1. Receives the config on login.
2. Spawns each enabled stdio MCP server as a child process.
3. Runs the MCP `initialize` handshake + `tools/list`.
4. Registers prefixed tools (`mcp_{name}_{tool}`) with the server.
5. On each heartbeat, checks if the config hash changed. If so, reinitializes.

The MCP server process inherits the env from `McpServerConfig.env`, not from the host (the client doesn't apply `env_clear` to MCP servers -- they get their own explicit env).

## Multiple Clients on the Same Machine

Each client needs its own token. The server assigns a `device_name` per token at creation time.

```bash
# Client A
NEXUS_SERVER_WS_URL=wss://nexus.example.com/ws \
NEXUS_AUTH_TOKEN=nexus_dev_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
nexus-client

# Client B
NEXUS_SERVER_WS_URL=wss://nexus.example.com/ws \
NEXUS_AUTH_TOKEN=nexus_dev_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb \
nexus-client
```

Each client gets its own:
- WebSocket connection and session
- Workspace directory (configured per-device in the web UI)
- FsPolicy (configured per-device in the web UI)
- MCP server config (set per-device on the server)

Both share the same binary. Different systemd service files (e.g., `nexus-client-a.service`, `nexus-client-b.service`) with different `EnvironmentFile` paths.

## Bubblewrap Installation

bwrap is optional (Sandbox mode works without it -- just no namespace isolation). Linux only.

```bash
# Debian/Ubuntu
sudo apt install bubblewrap

# Fedora/RHEL
sudo dnf install bubblewrap

# Arch
sudo pacman -S bubblewrap

# Verify
bwrap --version
```

The client checks for bwrap availability once at startup. If installed after the client starts, restart the client to pick it up.

On macOS and Windows, bwrap is not available. The client relies on guardrails + env isolation only (no namespace sandbox).

## Connection Behavior

The client reconnects automatically with exponential backoff (1s, 2s, 4s, ... up to 30s). On each reconnect:
1. Full handshake (RequireLogin -> SubmitToken -> LoginSuccess).
2. Tool discovery and re-registration.
3. Heartbeat loop resumes.

No manual intervention needed after network interruptions.
