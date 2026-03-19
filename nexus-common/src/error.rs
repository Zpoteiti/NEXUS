/// 职责边界：
/// 1. 定义跨网络传输的标准错误结构体和 Enum 枚举。
/// 2. 方便 Server 知道 Client 为什么执行失败，也方便 Client 知道 Server 为什么拒绝它的连接。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NexusErrorPayload {
    pub code: String,    // 错误码，例如 "AUTH_FAILED", "EXECUTION_TIMEOUT"
    pub message: String, // 人类可读的错误详情
}

// TODO: 定义常见的 Error Codes 常量或 Enum