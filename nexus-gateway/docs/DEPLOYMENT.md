# Gateway Deployment

## Build

```bash
cd NEXUS
cargo build --release --package nexus-gateway
# Binary at: target/release/nexus-gateway
```

Static linking (fully portable binary):

```bash
RUSTFLAGS="-C target-feature=+crt-static" cargo build --release --package nexus-gateway --target x86_64-unknown-linux-gnu
```

## Environment Variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `NEXUS_GATEWAY_TOKEN` | Yes | -- | Shared secret for nexus-server auth (constant-time compared) |
| `JWT_SECRET` | Yes | -- | HMAC secret for browser JWT validation (must match server) |
| `GATEWAY_PORT` | Yes | -- | Listen port (e.g. `9090`) |
| `NEXUS_SERVER_API_URL` | Yes | -- | Upstream nexus-server base URL for REST proxy (e.g. `http://server:8080`) |
| `NEXUS_FRONTEND_DIR` | No | `../nexus-frontend/dist` | Path to built frontend static files |

A `.env` file in the working directory is loaded automatically via `dotenvy`.

## Deployment Topology

### Same machine (simplest)

```
Browser --[wss]--> nginx:443 ---> nexus-gateway:9090 --[ws]--> nexus-server:8080
                                       |
                                       +--> /api/* proxied to nexus-server:8080
```

Gateway and server on the same box. Gateway serves the frontend static files via `NEXUS_FRONTEND_DIR`. Nginx handles TLS.

### Edge deployment

```
Browser --[wss]--> edge-gateway:443 --[ws over WAN]--> nexus-server:8080
```

Gateway at the edge (close to users), server in a datacenter. Higher latency on the server link, but browser connections are snappy. The gateway buffers nothing -- messages are forwarded immediately.

## Nginx Reverse Proxy

```nginx
upstream gateway {
    server 127.0.0.1:9090;
}

server {
    listen 443 ssl http2;
    server_name nexus.example.com;

    ssl_certificate     /etc/letsencrypt/live/nexus.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/nexus.example.com/privkey.pem;

    # WebSocket endpoints
    location /ws/ {
        proxy_pass http://gateway;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_read_timeout 86400s;  # keep WS alive for 24h
        proxy_send_timeout 86400s;
    }

    # API + frontend (everything else)
    location / {
        proxy_pass http://gateway;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # For file uploads (proxy max body = 25MB, matching gateway limit)
        client_max_body_size 25m;
    }
}
```

## TLS with Caddy (zero-config alternative)

```
nexus.example.com {
    reverse_proxy localhost:9090
}
```

Caddy auto-provisions Let's Encrypt certs and handles WebSocket upgrade headers automatically.

## CORS

The gateway applies a permissive CORS policy via `tower-http`:

```rust
CorsLayer::new()
    .allow_origin(Any)
    .allow_methods(Any)
    .allow_headers(Any)
```

This is fine when the gateway is behind a reverse proxy that handles origin restrictions. For direct exposure, consider restricting `allow_origin` to your domain.

## Systemd Service

```ini
[Unit]
Description=nexus-gateway
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=nexus
Group=nexus
WorkingDirectory=/opt/nexus
ExecStart=/opt/nexus/nexus-gateway
EnvironmentFile=/opt/nexus/.env
Restart=always
RestartSec=3

# Hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadOnlyPaths=/opt/nexus
PrivateTmp=true

# Allow enough file descriptors for many browser connections
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
```

```bash
sudo cp nexus-gateway.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now nexus-gateway
journalctl -u nexus-gateway -f  # tail logs
```

## Frontend Serving

The gateway serves static files from `NEXUS_FRONTEND_DIR` as a fallback route, with SPA support (unknown paths serve `index.html`). Build the frontend first:

```bash
cd nexus-frontend && npm run build
# Output in nexus-frontend/dist/
```

Set `NEXUS_FRONTEND_DIR=./nexus-frontend/dist` (or absolute path) in the gateway's `.env`.
