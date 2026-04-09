# nexus-server Deployment Guide

## Build

```bash
# Release binary (single static binary, ~30MB)
cargo build --release --package nexus-server

# Binary location
ls -la target/release/nexus-server
```

Requires Rust 1.85+ (edition 2024).

## Required Environment Variables

These are checked at startup. Missing required vars cause a panic with a descriptive message.

| Variable | Required | Default | Description |
|---|---|---|---|
| `DATABASE_URL` | **yes** | -- | PostgreSQL connection string. Example: `postgres://nexus:secret@localhost/nexus` |
| `ADMIN_TOKEN` | **yes** | -- | Token for creating admin users during registration |
| `JWT_SECRET` | **yes** | -- | HMAC-SHA256 signing key for JWTs. Use at least 32 random chars |
| `SERVER_PORT` | **yes** | -- | HTTP listen port (e.g. `8080`) |
| `NEXUS_GATEWAY_WS_URL` | **yes** | -- | Gateway WebSocket URL (e.g. `ws://gateway:9090/ws/nexus`) |
| `NEXUS_GATEWAY_TOKEN` | **yes** | -- | Token for server-to-gateway auth |
| `NEXUS_SKILLS_DIR` | no | `~/.nexus/skills` | Directory for skill file storage |

A `.env` file in the working directory is loaded automatically via `dotenvy`.

## PostgreSQL Setup

```bash
# Create database and user
sudo -u postgres psql <<'SQL'
CREATE USER nexus WITH PASSWORD 'your-secure-password';
CREATE DATABASE nexus OWNER nexus;
GRANT ALL PRIVILEGES ON DATABASE nexus TO nexus;
SQL
```

Tables are created automatically on startup via `db::init_db`. No manual migrations needed.

Connection pool: **200 max connections** (hardcoded in `main.rs`). Make sure PostgreSQL's `max_connections` is at least this + headroom for other clients.

```bash
# Check PostgreSQL max_connections
sudo -u postgres psql -c "SHOW max_connections;"
# Increase if needed (in postgresql.conf):
# max_connections = 300
```

## Systemd Service

```ini
[Unit]
Description=NEXUS Server
After=network.target postgresql.service
Requires=postgresql.service

[Service]
Type=simple
User=nexus
Group=nexus
WorkingDirectory=/opt/nexus
ExecStart=/opt/nexus/nexus-server
Restart=always
RestartSec=5

# Environment (or use EnvironmentFile)
EnvironmentFile=/opt/nexus/.env

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/tmp/nexus-uploads /tmp/nexus-media
PrivateTmp=false

# Resource limits
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
```

```bash
# Install
sudo cp nexus-server.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now nexus-server
sudo journalctl -u nexus-server -f
```

## Docker

```dockerfile
# Build stage
FROM rust:1.85-bookworm AS builder
WORKDIR /build
COPY . .
RUN cargo build --release --package nexus-server

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/nexus-server /usr/local/bin/nexus-server
EXPOSE 8080
CMD ["nexus-server"]
```

```yaml
# docker-compose.yml
services:
  postgres:
    image: postgres:16
    environment:
      POSTGRES_USER: nexus
      POSTGRES_PASSWORD: secret
      POSTGRES_DB: nexus
    volumes:
      - pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U nexus"]
      interval: 5s
      timeout: 5s
      retries: 5

  nexus-server:
    build: .
    ports:
      - "8080:8080"
    environment:
      DATABASE_URL: postgres://nexus:secret@postgres/nexus
      ADMIN_TOKEN: change-me-in-production
      JWT_SECRET: generate-a-random-32-char-string
      NEXUS_GATEWAY_TOKEN: also-change-this
    depends_on:
      postgres:
        condition: service_healthy

volumes:
  pgdata:
```

## Health Checks

The server does not have a dedicated `/health` endpoint. Use:

```bash
# TCP check (server is listening)
curl -sf http://localhost:8080/api/auth/login -X POST -H 'Content-Type: application/json' -d '{}' || true
# Expected: 401 Unauthorized (server is alive)

# Or just check if the port is open
nc -z localhost 8080
```

The fallback handler returns `404 Not Found` for any unmatched route, which also confirms the server is running.

## Production Recommendations

### PostgreSQL Connection Pool

The pool is set to 200 max connections. For heavy workloads with many concurrent agent loops:

- Each agent loop holds a connection during DB operations (message saves, checkpoint writes)
- Cron scheduler polls every 10 seconds
- Config changes pushed to clients immediately (no DB queries on heartbeat)

If you see connection pool exhaustion, increase PostgreSQL's `max_connections` and restart.

### Heartbeat Timeout

Default 60s. The client sends heartbeats every 15s. The reaper checks every 30s.

- For flaky networks: increase to 120s
- For quick failover: keep at 60s (detect dead devices within ~90s worst case)

### Rate Limiting

Set via `PUT /api/admin/rate-limit`. The value is cached for 60s in memory. 0 = unlimited.

Reasonable defaults:
- Personal use: 0 (unlimited)
- Shared instance: 10-30 per minute per user

### File Storage

Files are stored in `/tmp/nexus-uploads` and `/tmp/nexus-media`. The cleanup task runs every hour and deletes files older than 24 hours.

For persistent file storage across restarts, mount a persistent volume:

```bash
# In systemd
ReadWritePaths=/var/lib/nexus/uploads /var/lib/nexus/media

# Then symlink or change the paths (currently hardcoded to /tmp)
```

### Graceful Shutdown

The server handles `SIGINT` (Ctrl+C) and `SIGTERM`:

1. Stops accepting new HTTP connections
2. Signals the message bus to shut down
3. Stops all channels (Discord bots, gateway connections) with a 10s timeout
4. Closes the database pool
5. Exits

### Logging

Uses `tracing_subscriber::fmt`. Control verbosity with `RUST_LOG`:

```bash
RUST_LOG=info           # Default: info level
RUST_LOG=debug          # Verbose: includes tool calls, LLM requests
RUST_LOG=nexus_server=debug,sqlx=warn  # Debug server, quiet sqlx
```

### Security Checklist

- [ ] Change `NEXUS_GATEWAY_TOKEN` from `dev-token`
- [ ] Use a strong `JWT_SECRET` (32+ random chars)
- [ ] Use a strong `ADMIN_TOKEN`
- [ ] Run behind a reverse proxy (nginx/caddy) with TLS
- [ ] Restrict PostgreSQL access to the server host only
- [ ] Set appropriate rate limits for multi-user deployments
- [ ] Review device filesystem policies (default: sandbox)
