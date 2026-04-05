/// 职责边界：
/// 1. 存放 Server 和 Client 共享的全局常量。
/// 2. 杜绝在两端代码中硬编码 (Hardcode) 魔法数字或字符串。

pub const PROTOCOL_VERSION: &str = "1.0";
pub const HEARTBEAT_INTERVAL_SEC: u64 = 15;
pub const DEFAULT_MCP_TOOL_TIMEOUT_SEC: u64 = 30;
pub const MAX_AGENT_ITERATIONS: u32 = 200;
pub const MAX_HISTORY_MESSAGES: usize = 500;
pub const MAX_TOOL_OUTPUT_CHARS: usize = 10_000;
pub const TOOL_OUTPUT_HEAD_CHARS: usize = 5_000;
pub const TOOL_OUTPUT_TAIL_CHARS: usize = 5_000;

pub const EXIT_CODE_SUCCESS: i32 = 0;
pub const EXIT_CODE_ERROR: i32 = 1;
pub const EXIT_CODE_TIMEOUT: i32 = -1;
pub const EXIT_CODE_CANCELLED: i32 = -2;
pub const EXIT_CODE_VALIDATION_FAILED: i32 = -3;

pub const DEVICE_TOKEN_PREFIX: &str = "nexus_dev_";
pub const DEVICE_TOKEN_RANDOM_LEN: usize = 32;
