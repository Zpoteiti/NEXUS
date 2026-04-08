/// Responsibility boundary:
/// 1. Stores global constants shared between Server and Client.
/// 2. Prevents hardcoded magic numbers or strings on either side.

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
