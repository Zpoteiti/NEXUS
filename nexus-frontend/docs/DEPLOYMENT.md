# Frontend Deployment

## Build

```bash
cd nexus-frontend
npm ci
npm run build    # runs: tsc -b && vite build
# Output: dist/
```

The `dist/` directory contains static files (HTML, JS, CSS, assets). No server runtime needed.

## Option 1: Serve from nexus-gateway (simplest)

The gateway already serves static files. Just point it at the build output:

```bash
NEXUS_FRONTEND_DIR=./nexus-frontend/dist nexus-gateway
```

The gateway's fallback route serves `index.html` for any path not matching `/ws/*` or `/api/*`, enabling client-side routing.

Done. No separate web server needed.

## Option 2: Nginx static serving

If you want nginx to serve the frontend directly (e.g., CDN caching, separate scaling):

```nginx
server {
    listen 443 ssl http2;
    server_name nexus.example.com;

    ssl_certificate     /etc/letsencrypt/live/nexus.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/nexus.example.com/privkey.pem;

    root /var/www/nexus-frontend/dist;
    index index.html;

    # SPA fallback -- serve index.html for all routes
    location / {
        try_files $uri $uri/ /index.html;
    }

    # Cache static assets aggressively (Vite adds content hashes)
    location /assets/ {
        expires 1y;
        add_header Cache-Control "public, immutable";
    }

    # Proxy API and WS to gateway
    location /api/ {
        proxy_pass http://127.0.0.1:9090;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location /ws/ {
        proxy_pass http://127.0.0.1:9090;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_read_timeout 86400s;
    }
}
```

## Option 3: S3 + CloudFront (CDN)

```bash
# Upload build output
aws s3 sync dist/ s3://nexus-frontend-bucket/ --delete

# Invalidate cache
aws cloudfront create-invalidation \
  --distribution-id E1234567890 \
  --paths "/*"
```

CloudFront behavior rules:

| Path pattern | Origin | Notes |
|---|---|---|
| `/api/*` | Gateway ALB/origin | Forward all headers, no caching |
| `/ws/*` | Gateway ALB/origin | WebSocket not supported by CloudFront -- use ALB directly |
| `/assets/*` | S3 | Cache 1 year (content-hashed filenames) |
| `Default (*)` | S3 | Custom error page: 403/404 -> `/index.html` (SPA routing) |

Note: CloudFront doesn't proxy WebSockets. Browsers must connect to the gateway directly for `/ws/chat`. Use a subdomain like `ws.nexus.example.com` pointing to the gateway, and configure the frontend to use it (see Gateway URL section below).

## Gateway URL Configuration

### Development (default -- no config needed)

Vite proxies `/api` and `/ws` to `localhost:9090`. The frontend derives the WebSocket URL from `window.location`:

```ts
const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
const wsUrl = `${protocol}//${window.location.host}/ws/chat?token=${token}`
```

This works in any same-origin deployment without changes.

### Production with separate WebSocket host

If the gateway is on a different domain (e.g., CDN serves frontend, gateway on `ws.nexus.example.com`), you need to inject the URL at build time or runtime.

**Build-time** (Vite env vars):

```bash
# .env.production
VITE_WS_URL=wss://ws.nexus.example.com
VITE_API_URL=https://api.nexus.example.com
```

Then in code: `import.meta.env.VITE_WS_URL` (requires code changes to `useWebSocket.ts` and `api.ts`).

**Runtime** (config injection):

```html
<!-- index.html -->
<script>
  window.__NEXUS_CONFIG__ = {
    wsUrl: "wss://ws.nexus.example.com",
    apiUrl: "https://api.nexus.example.com"
  };
</script>
```

Replace values at deploy time via envsubst or a shell script. No rebuild needed.

## Docker Multi-Stage Build

```dockerfile
# Stage 1: Build
FROM node:22-alpine AS build
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY . .
RUN npm run build

# Stage 2: Serve
FROM nginx:alpine
COPY --from=build /app/dist /usr/share/nginx/html

# SPA routing + proxy config
COPY <<'EOF' /etc/nginx/conf.d/default.conf
server {
    listen 80;
    root /usr/share/nginx/html;
    index index.html;

    location / {
        try_files $uri $uri/ /index.html;
    }

    location /assets/ {
        expires 1y;
        add_header Cache-Control "public, immutable";
    }

    location /api/ {
        proxy_pass http://gateway:9090;
    }

    location /ws/ {
        proxy_pass http://gateway:9090;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_read_timeout 86400s;
    }
}
EOF

EXPOSE 80
```

```bash
docker build -t nexus-frontend .
docker run -p 80:80 nexus-frontend
```

In Docker Compose, set `gateway` as the service name so nginx can resolve it.

## Environment Variables

The frontend has no server-side runtime, so "env vars" are build-time only:

| Variable | When | Description |
|---|---|---|
| `VITE_*` | Build time | Baked into the JS bundle via `import.meta.env.VITE_*` |
| `window.__NEXUS_CONFIG__` | Runtime | Injected into `index.html` at deploy time |

Currently, the frontend uses neither -- it derives everything from `window.location`. You only need env vars if the gateway lives on a different origin than the frontend.
