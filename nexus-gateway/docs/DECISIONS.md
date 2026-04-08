# Gateway Design Decisions

## Why a separate gateway (not embedded in server)

The server runs the agent loop, talks to LLM providers, manages tools, and coordinates clients. Mixing browser-facing WebSocket handling into that process creates several problems:

- **Different failure domains.** If the agent loop OOMs or panics, browser connections shouldn't die. The gateway stays up and shows "server not connected" errors.
- **Edge deployment.** Gateway can run close to users (low-latency WebSocket), while the server lives near the database and GPU/API endpoints.
- **Security boundary.** Browser-facing auth (JWT), CORS, and rate limiting are gateway concerns. Server-to-server auth (device tokens) is a different trust model. Keeping them in separate binaries means a browser exploit can't directly reach the tool execution layer.
- **Horizontal scaling.** Multiple gateways can front a single server. The nexus-server connection is one WebSocket; browser connections fan out.

## Why DashMap for browser connections

`state.rs` stores browser connections in `Arc<DashMap<String, BrowserConnection>>`.

- Lock-free concurrent reads. Browser messages arrive on independent tokio tasks -- a `RwLock<HashMap>` would serialize all lookups behind a single lock.
- Shard-based writes. `DashMap` uses internal sharding so inserts/removes to different `chat_id`s don't contend.
- Simpler than channels. An alternative is a dedicated "connection manager" task with an mpsc inbox, but that adds latency and complexity for a straightforward key-value problem.
- The nexus-server connection uses `Arc<RwLock<Option<Sender>>>` instead because there's exactly one of them -- no concurrency benefit from `DashMap`.

## Why constant-time token comparison

`gateway.rs` line 175-182:

```rust
pub fn verify_token(provided: &str, expected: &str) -> bool {
    let a = provided.as_bytes();
    let b = expected.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}
```

Uses the `subtle` crate's `ct_eq` (constant-time equality). A naive `==` comparison short-circuits on the first differing byte, leaking token length and prefix through timing. An attacker probing `/ws/nexus` could reconstruct `NEXUS_GATEWAY_TOKEN` one byte at a time. The length check still leaks length, but not content -- and the token format (`NEXUS_GATEWAY_TOKEN`) isn't secret-length in practice.

## Why JWT validation at gateway level

The gateway validates JWTs on both WebSocket upgrade (`/ws/chat`) and REST proxy (`/api/*`) rather than passing them through to the server.

- **Fail fast.** Invalid/expired tokens are rejected before a WebSocket connection is established or an HTTP request is proxied. No wasted upstream resources.
- **Single trust boundary.** The gateway is the only component exposed to the internet. If JWT validation only happened at the server, a misconfigured firewall that exposed the server directly would bypass all auth.
- **Shared secret.** Both gateway and server use the same `JWT_SECRET`, so tokens validated at the gateway are equally valid at the server. The server can still re-validate if it wants defense in depth.

## Why REST proxy passes through (not reimplements) API calls

`proxy.rs` forwards `/api/*` requests to nexus-server by copying method, headers, and body. It doesn't parse request/response payloads or maintain its own API routes.

- **Zero maintenance.** Adding a new server endpoint doesn't require gateway changes. The proxy is transparent.
- **No schema coupling.** The gateway doesn't need to know about session models, memory structures, or admin APIs. It's just a pipe with auth.
- **Consistent behavior.** Error codes, pagination headers, and content types are preserved exactly as the server returns them. No translation bugs.
- **Public endpoints are whitelisted** (`/api/auth/login`, `/api/auth/register`) to skip JWT validation. Everything else requires a valid token.
