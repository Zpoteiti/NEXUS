//! Shared constants between Server and Client.
//! Prevents hardcoded magic numbers or strings on either side.

pub const PROTOCOL_VERSION: &str = "1.0";
pub const HEARTBEAT_INTERVAL_SEC: u64 = 15;
pub const DEFAULT_MCP_TOOL_TIMEOUT_SEC: u64 = 30;
pub const MAX_AGENT_ITERATIONS: u32 = 200;
pub const MAX_TOOL_OUTPUT_CHARS: usize = 10_000;
pub const TOOL_OUTPUT_HEAD_CHARS: usize = 5_000;
pub const TOOL_OUTPUT_TAIL_CHARS: usize = 5_000;

pub const EXIT_CODE_SUCCESS: i32 = 0;
pub const EXIT_CODE_ERROR: i32 = 1;
pub const EXIT_CODE_TIMEOUT: i32 = -1;
pub const EXIT_CODE_CANCELLED: i32 = -2;

pub const DEVICE_TOKEN_PREFIX: &str = "nexus_dev_";
pub const DEVICE_TOKEN_RANDOM_LEN: usize = 32;

pub const SERVER_DEVICE_NAME: &str = "server";

// File tool limits
pub const MAX_READ_FILE_CHARS: usize = 128_000;
pub const DEFAULT_READ_FILE_LIMIT: usize = 2000;
pub const DEFAULT_LIST_DIR_MAX: usize = 200;

// Shell timeout
pub const DEFAULT_SHELL_TIMEOUT_SEC: u64 = 60;

// Server constants
pub const TOOL_EXECUTION_TIMEOUT_SEC: u64 = 120;
pub const MEMORY_TEXT_MAX_CHARS: usize = 4096;
pub const USER_MESSAGE_MAX_CHARS: usize = 4000;
pub const CONTEXT_COMPRESSION_THRESHOLD: usize = 16_000;
pub const FILE_UPLOAD_MAX_BYTES: usize = 25 * 1024 * 1024;
pub const FILE_CLEANUP_AGE_HOURS: u64 = 24;
pub const WEB_FETCH_MAX_BODY_BYTES: usize = 1_048_576;
pub const WEB_FETCH_MAX_OUTPUT_CHARS: usize = 50_000;
pub const WEB_FETCH_TIMEOUT_SEC: u64 = 15;
pub const WEB_FETCH_CONNECT_TIMEOUT_SEC: u64 = 10;
pub const WEB_FETCH_MAX_REDIRECTS: usize = 5;
pub const WEB_FETCH_CONCURRENT_MAX: usize = 50;
pub const DB_POOL_MAX_CONNECTIONS: u32 = 200;
pub const RATE_LIMIT_CACHE_TTL_SEC: u64 = 60;
pub const JWT_EXPIRY_DAYS: i64 = 7;
pub const BCRYPT_COST: u32 = 12;
pub const HEARTBEAT_REAPER_INTERVAL_SEC: u64 = 30;
pub const CRON_POLL_INTERVAL_SEC: u64 = 10;
pub const COMPRESSION_SUMMARY_MAX_TOKENS: u32 = 12_000;
pub const MAX_UNCOMPRESSED_MESSAGES: i64 = 2000;

// Message roles (prevents typos in stringly-typed matching)
pub const ROLE_SYSTEM: &str = "system";
pub const ROLE_USER: &str = "user";
pub const ROLE_ASSISTANT: &str = "assistant";
pub const ROLE_TOOL: &str = "tool";

// Channel names
pub const CHANNEL_GATEWAY: &str = "gateway";
pub const CHANNEL_DISCORD: &str = "discord";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants_values() {
        assert_eq!(PROTOCOL_VERSION, "1.0");
        assert_eq!(HEARTBEAT_INTERVAL_SEC, 15);
        assert_eq!(
            MAX_TOOL_OUTPUT_CHARS,
            TOOL_OUTPUT_HEAD_CHARS + TOOL_OUTPUT_TAIL_CHARS
        );
        assert_eq!(EXIT_CODE_SUCCESS, 0);
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
