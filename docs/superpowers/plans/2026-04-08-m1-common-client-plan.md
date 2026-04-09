# M1: nexus-common + nexus-client Implementation Plan

**For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the shared protocol layer (nexus-common) and implement the execution node (nexus-client) from scratch, establishing the foundation for distributed tool execution.

**Architecture:** 
- nexus-common: Clean protocol types (ServerToClient/ClientToServer enums), shared constants, error handling, and utilities (MCP schema normalization, MIME detection)
- nexus-client: Standalone Rust binary that connects via WebSocket to server, authenticates with device token, receives config via push, executes 7 built-in tools + MCP servers, handles reconnection with exponential backoff

**Tech Stack:** Rust 1.85+, tokio, tokio-tungstenite, serde, tracing, glob, regex, rmcp (MCP client SDK)

---

## Section 1: Workspace Setup + nexus-common Cleanup

**Files:**
- Modify: `/home/yucheng/Documents/GitHub/NEXUS/Cargo.toml` (create workspace root)
- Modify: `/home/yucheng/Documents/GitHub/NEXUS/nexus-common/Cargo.toml`
- Modify: `/home/yucheng/Documents/GitHub/NEXUS/nexus-common/src/protocol.rs`
- Modify: `/home/yucheng/Documents/GitHub/NEXUS/nexus-common/src/consts.rs`

---

### Task 1: Create Cargo Workspace Root

**Files:**
- Create: `/home/yucheng/Documents/GitHub/NEXUS/Cargo.toml`

**Rationale:** Currently nexus-common is standalone. We need a workspace root to manage multiple crates (nexus-common, nexus-client, and future server/gateway/frontend).

- [ ] **Step 1: Write Cargo.toml workspace root**

Create `/home/yucheng/Documents/GitHub/NEXUS/Cargo.toml` with:

```toml
[workspace]
members = ["nexus-common", "nexus-client"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2024"
authors = ["NEXUS Team"]
license = "MIT"

[profile.release]
opt-level = 3
lto = true
```

- [ ] **Step 2: Verify workspace structure**

Run:
```bash
cd /home/yucheng/Documents/GitHub/NEXUS
cargo metadata --format-version 1 | jq '.workspace_members | length'
```

Expected output: `2` (nexus-common + nexus-client)

---

### Task 2: Update nexus-common Cargo.toml

**Files:**
- Modify: `/home/yucheng/Documents/GitHub/NEXUS/nexus-common/Cargo.toml`

**Rationale:** Use workspace package values, add all dependencies needed for the crate.

- [ ] **Step 1: Replace Cargo.toml content**

Replace `/home/yucheng/Documents/GitHub/NEXUS/nexus-common/Cargo.toml` with:

```toml
[package]
name = "nexus-common"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
axum = { version = "0.8", optional = true }
regex = "1"

[features]
default = []
axum = ["dep:axum"]

[lints.rust]
unsafe_code = "forbid"
```

- [ ] **Step 2: Verify dependencies resolve**

Run:
```bash
cd /home/yucheng/Documents/GitHub/NEXUS/nexus-common
cargo check
```

Expected: No errors.

---

### Task 3: Clean Up FsPolicy Enum

**Files:**
- Modify: `/home/yucheng/Documents/GitHub/NEXUS/nexus-common/src/protocol.rs:21-30`

**Rationale:** The spec requires removing `FsPolicy::Whitelist` variant (replaced by server-side SSRF whitelist). M1 only supports Sandbox and Unrestricted.

- [ ] **Step 1: Write test for FsPolicy variants**

Add to `/home/yucheng/Documents/GitHub/NEXUS/nexus-common/src/protocol.rs` (after line 152):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fspolicy_default_is_sandbox() {
        assert_eq!(FsPolicy::default(), FsPolicy::Sandbox);
    }

    #[test]
    fn test_fspolicy_serialize_deserialize() {
        let sandbox = FsPolicy::Sandbox;
        let json = serde_json::to_string(&sandbox).unwrap();
        assert_eq!(json, r#"{"mode":"sandbox"}"#);
        let deserialized: FsPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, sandbox);

        let unrestricted = FsPolicy::Unrestricted;
        let json = serde_json::to_string(&unrestricted).unwrap();
        assert_eq!(json, r#"{"mode":"unrestricted"}"#);
        let deserialized: FsPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, unrestricted);
    }
}
```

- [ ] **Step 2: Run test to verify it fails (Whitelist still exists)**

Run:
```bash
cd /home/yucheng/Documents/GitHub/NEXUS/nexus-common
cargo test test_fspolicy
```

Expected: PASS (the test checks the current Sandbox/Unrestricted variants, which exist).

- [ ] **Step 3: Update FsPolicy enum**

Replace lines 17-30 in `/home/yucheng/Documents/GitHub/NEXUS/nexus-common/src/protocol.rs`:

```rust
/// Per-device filesystem access policy.
/// - Sandbox: only workspace (default)
/// - Unrestricted: full filesystem access
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode")]
pub enum FsPolicy {
    #[serde(rename = "sandbox")]
    Sandbox,
    #[serde(rename = "unrestricted")]
    Unrestricted,
}
```

Remove the `Whitelist` variant entirely (lines 26-27 from original).

- [ ] **Step 4: Run tests to verify no regressions**

Run:
```bash
cd /home/yucheng/Documents/GitHub/NEXUS/nexus-common
cargo test test_fspolicy
```

Expected: PASS

---

### Task 4: Update ServerToClient Enum

**Files:**
- Modify: `/home/yucheng/Documents/GitHub/NEXUS/nexus-common/src/protocol.rs:62-89`

**Rationale:** Per spec section 1.2:
- Remove: `FileUploadRequest`, `FileDownloadRequest`
- Remove: `HeartbeatAck` fields (fs_policy, mcp_servers) — becomes lightweight ping
- Add: `ConfigUpdate` variant for push-based config updates
- Add: `LoginSuccess` fields (workspace_path, shell_timeout, ssrf_whitelist)

- [ ] **Step 1: Write test for new ServerToClient variants**

Add to the `tests` module in protocol.rs (after the FsPolicy test):

```rust
#[test]
fn test_server_to_client_login_success_serialization() {
    let msg = ServerToClient::LoginSuccess {
        user_id: "user123".to_string(),
        device_name: "dev1".to_string(),
        fs_policy: FsPolicy::Sandbox,
        mcp_servers: vec![],
        workspace_path: "/home/dev".to_string(),
        shell_timeout: 60,
        ssrf_whitelist: vec!["10.0.0.0/8".to_string()],
    };
    let json = serde_json::to_string(&msg).unwrap();
    let deserialized: ServerToClient = serde_json::from_str(&json).unwrap();
    match deserialized {
        ServerToClient::LoginSuccess { 
            workspace_path,
            shell_timeout,
            ssrf_whitelist,
            ..
        } => {
            assert_eq!(workspace_path, "/home/dev");
            assert_eq!(shell_timeout, 60);
            assert_eq!(ssrf_whitelist.len(), 1);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_server_to_client_config_update() {
    let msg = ServerToClient::ConfigUpdate {
        fs_policy: Some(FsPolicy::Unrestricted),
        mcp_servers: None,
        workspace_path: None,
        shell_timeout: Some(120),
        ssrf_whitelist: None,
    };
    let json = serde_json::to_string(&msg).unwrap();
    let _: ServerToClient = serde_json::from_str(&json).unwrap();
}

#[test]
fn test_server_to_client_heartbeat_ack_lightweight() {
    // HeartbeatAck should now be a simple empty variant
    let msg = ServerToClient::HeartbeatAck;
    let json = serde_json::to_string(&msg).unwrap();
    assert_eq!(json, r#"{"type":"HeartbeatAck"}"#);
}
```

- [ ] **Step 2: Run test to verify it fails (variants don't exist yet)**

Run:
```bash
cd /home/yucheng/Documents/GitHub/NEXUS/nexus-common
cargo test test_server_to_client
```

Expected: COMPILE ERROR (new variants not defined)

- [ ] **Step 3: Update ServerToClient enum**

Replace lines 62-89 in `/home/yucheng/Documents/GitHub/NEXUS/nexus-common/src/protocol.rs`:

```rust
/// Commands sent from server to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ServerToClient {
    ExecuteToolRequest(ExecuteToolRequest),
    RequireLogin {
        message: String,
    },
    LoginSuccess {
        user_id: String,
        device_name: String,
        fs_policy: FsPolicy,
        mcp_servers: Vec<McpServerEntry>,
        workspace_path: String,
        shell_timeout: u64,
        ssrf_whitelist: Vec<String>,
    },
    LoginFailed {
        reason: String,
    },
    HeartbeatAck,
    ConfigUpdate {
        fs_policy: Option<FsPolicy>,
        mcp_servers: Option<Vec<McpServerEntry>>,
        workspace_path: Option<String>,
        shell_timeout: Option<u64>,
        ssrf_whitelist: Option<Vec<String>>,
    },
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run:
```bash
cd /home/yucheng/Documents/GitHub/NEXUS/nexus-common
cargo test test_server_to_client
```

Expected: PASS (all 3 test variants serialize/deserialize correctly)

---

### Task 5: Update ClientToServer Enum

**Files:**
- Modify: `/home/yucheng/Documents/GitHub/NEXUS/nexus-common/src/protocol.rs:105-124`

**Rationale:** Per spec section 1.2:
- Remove: `FileUploadResponse`, `FileDownloadResponse`
- Remove: `Heartbeat.hash` field — becomes lightweight status-only ping
- Keep: `ToolExecutionResult`, `SubmitToken`, `RegisterTools`

- [ ] **Step 1: Write test for new ClientToServer variants**

Add to the `tests` module in protocol.rs:

```rust
#[test]
fn test_client_to_server_heartbeat_lightweight() {
    let msg = ClientToServer::Heartbeat {
        status: DeviceStatus::Online,
    };
    let json = serde_json::to_string(&msg).unwrap();
    let deserialized: ClientToServer = serde_json::from_str(&json).unwrap();
    match deserialized {
        ClientToServer::Heartbeat { status } => {
            assert_eq!(status, DeviceStatus::Online);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_client_to_server_submit_token() {
    let msg = ClientToServer::SubmitToken {
        token: "nexus_dev_abc123".to_string(),
        protocol_version: "1.0".to_string(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let _: ClientToServer = serde_json::from_str(&json).unwrap();
}
```

- [ ] **Step 2: Run test to verify it fails (Heartbeat.hash still exists)**

Run:
```bash
cd /home/yucheng/Documents/GitHub/NEXUS/nexus-common
cargo test test_client_to_server
```

Expected: COMPILE ERROR (hash field still required in Heartbeat)

- [ ] **Step 3: Update ClientToServer enum**

Replace lines 105-124 in `/home/yucheng/Documents/GitHub/NEXUS/nexus-common/src/protocol.rs`:

```rust
/// Events reported from client to server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ClientToServer {
    ToolExecutionResult(ToolExecutionResult),
    SubmitToken {
        token: String,
        protocol_version: String,
    },
    RegisterTools {
        schemas: Vec<Value>,
    },
    Heartbeat {
        status: DeviceStatus,
    },
}
```

Remove `FileUploadResponse`, `FileDownloadResponse`, and the `hash` field from `Heartbeat`.

- [ ] **Step 4: Remove unused structs**

Delete from protocol.rs:
- `FileUploadRequest` struct (lines 99-102)
- `FileUploadResponse` struct (lines 127-133)
- `FileDownloadResponse` struct (lines 136-139)

- [ ] **Step 5: Run tests to verify they pass**

Run:
```bash
cd /home/yucheng/Documents/GitHub/NEXUS/nexus-common
cargo test test_client_to_server
```

Expected: PASS

- [ ] **Step 6: Run all nexus-common tests**

Run:
```bash
cd /home/yucheng/Documents/GitHub/NEXUS/nexus-common
cargo test
```

Expected: All tests pass (including existing mime and mcp_utils tests)

---

### Task 6: Update consts.rs for client tools

**Files:**
- Modify: `/home/yucheng/Documents/GitHub/NEXUS/nexus-common/src/consts.rs`

**Rationale:** Add constants the client will need for filesystem and shell tool limits.

- [ ] **Step 1: Write test for new constants**

Add to consts.rs (after line 22):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants_values() {
        assert_eq!(PROTOCOL_VERSION, "1.0");
        assert_eq!(HEARTBEAT_INTERVAL_SEC, 15);
        assert_eq!(DEFAULT_MCP_TOOL_TIMEOUT_SEC, 30);
        assert_eq!(MAX_AGENT_ITERATIONS, 200);
        assert_eq!(MAX_TOOL_OUTPUT_CHARS, 10_000);
        assert_eq!(TOOL_OUTPUT_HEAD_CHARS, 5_000);
        assert_eq!(TOOL_OUTPUT_TAIL_CHARS, 5_000);
        assert_eq!(EXIT_CODE_SUCCESS, 0);
        assert_eq!(EXIT_CODE_ERROR, 1);
        assert_eq!(EXIT_CODE_TIMEOUT, -1);
        assert_eq!(EXIT_CODE_CANCELLED, -2);
        assert_eq!(DEVICE_TOKEN_PREFIX, "nexus_dev_");
        assert_eq!(DEVICE_TOKEN_RANDOM_LEN, 32);
    }

    #[test]
    fn test_file_tool_constants() {
        assert_eq!(MAX_READ_FILE_CHARS, 128_000);
        assert_eq!(DEFAULT_READ_FILE_LIMIT, 2000);
        assert_eq!(DEFAULT_LIST_DIR_MAX, 200);
    }

    #[test]
    fn test_shell_timeout_default() {
        assert_eq!(DEFAULT_SHELL_TIMEOUT_SEC, 60);
    }
}
```

- [ ] **Step 2: Run test to verify it fails (new constants don't exist)**

Run:
```bash
cd /home/yucheng/Documents/GitHub/NEXUS/nexus-common
cargo test test_file_tool
```

Expected: COMPILE ERROR (MAX_READ_FILE_CHARS, etc. not defined)

- [ ] **Step 3: Update consts.rs**

Append to `/home/yucheng/Documents/GitHub/NEXUS/nexus-common/src/consts.rs` (after line 22):

```rust
// File tool limits
pub const MAX_READ_FILE_CHARS: usize = 128_000;
pub const DEFAULT_READ_FILE_LIMIT: usize = 2000;
pub const DEFAULT_LIST_DIR_MAX: usize = 200;

// Shell timeout
pub const DEFAULT_SHELL_TIMEOUT_SEC: u64 = 60;

// Exit code for validation/guardrails (not timeout or cancellation)
pub const EXIT_CODE_BLOCKED: i32 = -2;
```

- [ ] **Step 4: Run all consts tests**

Run:
```bash
cd /home/yucheng/Documents/GitHub/NEXUS/nexus-common
cargo test test_constants test_file_tool test_shell_timeout
```

Expected: PASS

---

### Task 7: Verify full nexus-common build and tests

**Files:**
- Verify: All of nexus-common/src/

- [ ] **Step 1: Clean and build**

Run:
```bash
cd /home/yucheng/Documents/GitHub/NEXUS/nexus-common
cargo clean
cargo build --all-features
```

Expected: Success, no warnings about dead code or unused variables

- [ ] **Step 2: Run all tests**

Run:
```bash
cd /home/yucheng/Documents/GitHub/NEXUS/nexus-common
cargo test --all-features
```

Expected: All tests pass (existing mime + mcp_utils + new protocol + new consts tests)

- [ ] **Step 3: Run clippy lint**

Run:
```bash
cd /home/yucheng/Documents/GitHub/NEXUS/nexus-common
cargo clippy --all-features -- -D warnings
```

Expected: No warnings

- [ ] **Step 4: Commit changes**

Run:
```bash
cd /home/yucheng/Documents/GitHub/NEXUS
git add -A
git commit -m "feat: M1 nexus-common cleanup — remove Whitelist FsPolicy, file transfer messages, push ConfigUpdate, lightweight Heartbeat"
```

Expected: Clean commit

---

## Summary: Section 1 Complete

✅ Workspace root established (Cargo.toml with members: ["nexus-common", "nexus-client"])
✅ nexus-common dependencies updated and tested
✅ FsPolicy: Whitelist variant removed, only Sandbox/Unrestricted remain
✅ ServerToClient: Added ConfigUpdate, added LoginSuccess fields (workspace_path, shell_timeout, ssrf_whitelist), removed file transfer messages, lightweight HeartbeatAck
✅ ClientToServer: Removed hash field from Heartbeat, removed file transfer messages
✅ consts.rs: Added file tool constants and shell timeout constant
✅ All tests passing, clippy clean

---

**Ready for Section 2?** (Client Skeleton: main.rs, connection.rs, heartbeat.rs, config.rs)

