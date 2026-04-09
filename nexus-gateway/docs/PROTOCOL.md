# Gateway WebSocket Protocol

All messages are JSON over WebSocket. Discriminated by `"type"` field. Two independent WebSocket endpoints:

- `/ws/chat` -- browser clients
- `/ws/nexus` -- nexus-server (exactly one connection allowed)

REST calls to `/api/*` are JWT-validated and reverse-proxied to nexus-server (see proxy section below).

---

## Browser -> Gateway (`/ws/chat`)

### `message` -- send a chat message

```json
{"type": "message", "content": "hello world"}
```

With media attachments (optional array of file reference strings):

```json
{"type": "message", "content": "check this out", "media": ["file1:screenshot.png"]}
```

### `new_session` -- start a fresh conversation

```json
{"type": "new_session"}
```

### `switch_session` -- resume an existing session

```json
{"type": "switch_session", "session_id": "gateway:user42:a1b2c3d4"}
```

---

## Gateway -> Browser

### `message` -- agent reply (final)

```json
{
  "type": "message",
  "content": "Here's what I found...",
  "session_id": "gateway:user42:a1b2c3d4"
}
```

With media (omitted from JSON when `None`):

```json
{
  "type": "message",
  "content": "Here's the image",
  "session_id": "gateway:user42:a1b2c3d4",
  "media": ["https://files.example.com/result.png"]
}
```

### `progress` -- streaming/thinking indicator

```json
{
  "type": "progress",
  "content": "Searching the codebase...",
  "session_id": "gateway:user42:a1b2c3d4"
}
```

### `error` -- something went wrong

```json
{"type": "error", "reason": "Nexus server not connected"}
```

### `session_created` -- response to `new_session` (or initial connection)

```json
{"type": "session_created", "session_id": "gateway:user42:a1b2c3d4"}
```

### `session_switched` -- response to `switch_session`

```json
{"type": "session_switched", "session_id": "gateway:user42:a1b2c3d4"}
```

---

## Gateway -> Nexus Server (`/ws/nexus`)

Sent when a browser user sends a message. Wraps the browser message with routing metadata.

### `message` -- forwarded user message

```json
{
  "type": "message",
  "chat_id": "e3f4a5b6-...",
  "sender_id": "user42",
  "content": "hello world",
  "session_id": "gateway:user42:a1b2c3d4"
}
```

With media:

```json
{
  "type": "message",
  "chat_id": "e3f4a5b6-...",
  "sender_id": "user42",
  "content": "check this",
  "session_id": "gateway:user42:a1b2c3d4",
  "media": ["file1:screenshot.png"]
}
```

### `auth_ok` -- server auth succeeded

```json
{"type": "auth_ok"}
```

### `auth_fail` -- server auth rejected

```json
{"type": "auth_fail", "reason": "invalid token"}
```

---

## Nexus Server -> Gateway (`/ws/nexus`)

### `auth` -- first message, must be sent immediately on connect

```json
{"type": "auth", "token": "your-gateway-token-here"}
```

### `send` -- push a message to a browser

```json
{
  "type": "send",
  "chat_id": "e3f4a5b6-...",
  "content": "Here's your answer"
}
```

With metadata (progress indicator, media, fallback routing):

```json
{
  "type": "send",
  "chat_id": "e3f4a5b6-...",
  "content": "thinking...",
  "metadata": {"_progress": true}
}
```

```json
{
  "type": "send",
  "chat_id": "e3f4a5b6-...",
  "content": "done",
  "metadata": {"sender_id": "user42", "media": ["https://files.example.com/out.png"]}
}
```

---

## Auth Flows

### Browser auth (`/ws/chat`)

1. Browser opens `ws(s)://host/ws/chat?token=<JWT>` (or passes `Authorization: Bearer <JWT>` header, but browsers can't do this on WS upgrade so query param is the standard path)
2. Gateway validates JWT using `jsonwebtoken` crate against `JWT_SECRET`
3. JWT claims: `{ "sub": "user42", "is_admin": false, "exp": 1234567890 }`
4. On success: connection upgraded, `session_created` sent immediately
5. On failure: HTTP 401 returned before upgrade

### Nexus server auth (`/ws/nexus`)

1. Server opens `ws://host/ws/nexus` (no auth on upgrade)
2. First message must be `{"type": "auth", "token": "..."}`
3. Token compared against `NEXUS_GATEWAY_TOKEN` using **constant-time comparison** (`subtle` crate)
4. On success: `auth_ok` sent back
5. On failure: `auth_fail` sent, connection dropped
6. Only one nexus connection allowed at a time -- duplicate connections are rejected with `auth_fail`

---

## Message Routing

Browser connects -> gateway assigns a random `chat_id` (UUIDv4) and `session_id` (`"gateway:{user_id}:{uuid}"`).

When nexus-server sends a `send` message:

1. **Direct lookup**: `chat_id` looked up in `DashMap<String, BrowserConnection>`
2. **Fallback**: if `chat_id` not found, checks `metadata.sender_id` and finds any connected browser for that user (handles cases like cron-triggered messages)
3. **Progress vs final**: if `metadata._progress == true`, sends `progress` type; otherwise sends `message` type
4. **Media extraction**: `metadata.media` array (if present) is forwarded to the browser
5. **No match**: warning logged, message dropped silently

---

## REST Proxy (`/api/*`)

All `/api/*` requests are reverse-proxied to nexus-server (`NEXUS_SERVER_API_URL`).

- **Public endpoints** (no JWT required): `/api/auth/login`, `/api/auth/register`
- **All other paths**: require `Authorization: Bearer <JWT>` header, validated at gateway
- Path traversal blocked (`..` rejected)
- Max body size: 25 MB (request and response)
- Hop-by-hop headers stripped (`host`, `connection`, `transfer-encoding`, `upgrade`)
- Uses connection-pooled `reqwest::Client` shared across all requests
