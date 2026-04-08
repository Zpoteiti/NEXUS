# nexus-server Database Schema

PostgreSQL. All tables created by `db::init_db` on startup (idempotent `CREATE TABLE IF NOT EXISTS`).

---

## users

Core user accounts.

| Column | Type | Description |
|---|---|---|
| `user_id` | `TEXT PRIMARY KEY` | UUID v4 |
| `email` | `TEXT UNIQUE NOT NULL` | Login identifier |
| `password_hash` | `TEXT NOT NULL DEFAULT ''` | bcrypt hash |
| `is_admin` | `BOOLEAN NOT NULL DEFAULT FALSE` | Admin flag |
| `soul` | `TEXT` | Custom system prompt (nullable) |
| `memory_text` | `TEXT NOT NULL DEFAULT ''` | Persistent memory (4K char cap enforced at API level) |
| `created_at` | `TIMESTAMPTZ DEFAULT NOW()` | |

**Read by:** `auth/mod.rs` (login), `context.rs` (soul, memory), `api.rs` (profile, soul, memory)
**Written by:** `auth/mod.rs` (register), `api.rs` (update soul/memory), `server_tools/memory.rs`

---

## device_tokens

Device authentication and per-device configuration.

| Column | Type | Description |
|---|---|---|
| `token` | `TEXT PRIMARY KEY` | `nexus_dev_` + 32 hex chars |
| `user_id` | `TEXT NOT NULL REFERENCES users(user_id)` | Owner |
| `device_name` | `TEXT NOT NULL` | Human-readable name |
| `fs_policy` | `JSONB NOT NULL DEFAULT '{"mode":"sandbox"}'` | Filesystem access policy |
| `mcp_config` | `JSONB NOT NULL DEFAULT '[]'` | MCP server entries for this device |
| `created_at` | `TIMESTAMPTZ DEFAULT NOW()` | |

**Indexes:**
- `idx_device_tokens_user_device` -- `UNIQUE (user_id, device_name)` -- one token per device name per user

**Read by:** `ws.rs` (token verification, policy/MCP on login and heartbeat), `api.rs` (list devices), `auth/device.rs` (CRUD)
**Written by:** `auth/device.rs` (create/delete token, update policy/MCP)

---

## sessions

Conversation sessions. One per chat context (web UI tab, Discord channel, cron job).

| Column | Type | Description |
|---|---|---|
| `session_id` | `TEXT PRIMARY KEY` | UUID or `cron:{job_id}` |
| `user_id` | `TEXT NOT NULL REFERENCES users(user_id)` | Owner |
| `created_at` | `TIMESTAMPTZ DEFAULT NOW()` | |

**Read by:** `auth/mod.rs` (list sessions), `db/sessions.rs` (get messages, ownership check)
**Written by:** `agent_loop.rs` (ensure_session on first message), `auth/mod.rs` (delete)

---

## messages

All messages in all sessions (user, assistant, tool results).

| Column | Type | Description |
|---|---|---|
| `message_id` | `TEXT PRIMARY KEY` | UUID v4 |
| `session_id` | `TEXT NOT NULL REFERENCES sessions(session_id)` | Parent session |
| `role` | `TEXT NOT NULL` | `user`, `assistant`, or `tool` |
| `content` | `TEXT NOT NULL` | Message text |
| `tool_call_id` | `TEXT` | Links tool result to tool call |
| `tool_name` | `TEXT` | Tool name (assistant role with tool calls) |
| `tool_arguments` | `TEXT` | JSON string of tool arguments |
| `compressed` | `BOOLEAN DEFAULT FALSE` | Marked true after context compression |
| `created_at` | `TIMESTAMPTZ DEFAULT NOW()` | |

**Read by:** `db/messages.rs` (session history reconstruction, session messages API)
**Written by:** `agent_loop.rs` (save_message for user/assistant/tool), `memory.rs` (mark_messages_compressed)

History reconstruction (`get_session_history`): merges consecutive assistant rows with `tool_name` into a single assistant message with a `tool_calls` array. Excludes rows where `compressed = TRUE`.

---

## discord_configs

Per-user Discord bot configuration.

| Column | Type | Description |
|---|---|---|
| `user_id` | `TEXT PRIMARY KEY REFERENCES users(user_id)` | One config per user |
| `bot_token` | `TEXT NOT NULL` | Discord bot token |
| `bot_user_id` | `TEXT` | Bot's own Discord user ID (set after first connection) |
| `owner_discord_id` | `TEXT` | Discord user ID of the owner (for security boundaries) |
| `enabled` | `BOOLEAN NOT NULL DEFAULT TRUE` | |
| `allowed_users` | `TEXT[] NOT NULL DEFAULT '{}'` | Discord user IDs allowed to interact |
| `created_at` | `TIMESTAMPTZ DEFAULT NOW()` | |
| `updated_at` | `TIMESTAMPTZ DEFAULT NOW()` | |

**Read by:** `channels/discord/` (get all enabled configs, get by user_id)
**Written by:** `auth/discord_config.rs` (upsert/delete), `channels/discord/` (update bot_user_id)

---

## system_config

Key-value store for server-wide settings.

| Column | Type | Description |
|---|---|---|
| `key` | `TEXT PRIMARY KEY` | Config key |
| `value` | `TEXT NOT NULL` | Config value (JSON or plain text) |
| `updated_at` | `TIMESTAMPTZ DEFAULT NOW()` | |

**Known keys:**
- `llm_config` -- JSON: `{ api_base, model, api_key, context_window }`
- `server_mcp_config` -- JSON: array of MCP server entries
- `rate_limit_per_min` -- integer as string, `0` = unlimited
- `default_soul` -- text: server-wide default system prompt

**Read by:** `main.rs` (startup load), `state.rs` (rate limit cache), `context.rs` (default soul cache), `auth/admin.rs`
**Written by:** `auth/admin.rs` (LLM config, rate limit, server MCP), `api.rs` (default soul)

---

## cron_jobs

Scheduled tasks that inject prompts into the agent loop.

| Column | Type | Description |
|---|---|---|
| `job_id` | `TEXT PRIMARY KEY` | First 8 chars of UUID v4 |
| `user_id` | `TEXT NOT NULL REFERENCES users(user_id)` | Owner |
| `name` | `TEXT NOT NULL` | Human-readable label |
| `enabled` | `BOOLEAN DEFAULT TRUE` | |
| `cron_expr` | `TEXT` | Standard 5-field crontab expression (converted to 7-field internally) |
| `every_seconds` | `INTEGER` | Interval-based scheduling (alternative to cron_expr) |
| `timezone` | `TEXT DEFAULT 'UTC'` | |
| `message` | `TEXT NOT NULL` | Prompt injected when job fires |
| `channel` | `TEXT NOT NULL` | Target channel (e.g., `webui`, `discord`) |
| `chat_id` | `TEXT NOT NULL` | Target chat/conversation ID |
| `delete_after_run` | `BOOLEAN DEFAULT FALSE` | One-shot: delete after first execution |
| `next_run_at` | `TIMESTAMPTZ` | Next scheduled fire time |
| `last_run_at` | `TIMESTAMPTZ` | |
| `run_count` | `INTEGER DEFAULT 0` | |
| `created_at` | `TIMESTAMPTZ DEFAULT NOW()` | |

**Read by:** `cron.rs` (get_due_cron_jobs every 10s), `auth/cron_api.rs` (list)
**Written by:** `auth/cron_api.rs` (create/update/delete), `db/cron.rs` (update after run), `server_tools/cron.rs`

---

## skills

Per-user skill definitions (SKILL.md files managed by the server).

| Column | Type | Description |
|---|---|---|
| `skill_id` | `TEXT PRIMARY KEY` | UUID v4 |
| `user_id` | `TEXT NOT NULL REFERENCES users(user_id)` | Owner |
| `name` | `TEXT NOT NULL` | Skill name (from frontmatter or API) |
| `description` | `TEXT NOT NULL DEFAULT ''` | From SKILL.md frontmatter |
| `always_on` | `BOOLEAN DEFAULT FALSE` | If true, full content injected into every prompt |
| `skill_path` | `TEXT NOT NULL` | Filesystem path to skill directory |
| `created_at` | `TIMESTAMPTZ DEFAULT NOW()` | |

**Constraints:** `UNIQUE(user_id, name)` -- one skill per name per user. Upsert on conflict.

**Read by:** `context.rs` (build_skills_section), `auth/skills_api.rs` (list/get), `server_tools/skills.rs` (read_skill)
**Written by:** `auth/skills_api.rs` (create/install/delete), `server_tools/skills.rs` (install_skill)

**Isolation:** Skills are strictly per-user. Each user's skills stored at `{skills_dir}/{user_id}/{skill_name}/`. The `read_skill` and `install_skill` server tools only access the current user's skills. Install methods: web UI, agent-driven (`install_skill` tool), or API.

---

## Migrations

Safe migrations are applied after table creation:

```sql
ALTER TABLE users ADD COLUMN IF NOT EXISTS memory_text TEXT NOT NULL DEFAULT '';
ALTER TABLE messages ADD COLUMN IF NOT EXISTS compressed BOOLEAN DEFAULT FALSE;
```

These handle upgrades from older schemas without breaking existing deployments.
