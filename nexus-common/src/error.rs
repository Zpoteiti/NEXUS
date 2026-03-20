/// 职责边界：
/// 1. 定义跨网络传输的标准错误结构体和 Enum 枚举。
/// 2. 方便 Server 知道 Client 为什么执行失败，也方便 Client 知道 Server 为什么拒绝它的连接。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NexusErrorPayload {
    pub code: String,    // 错误码，例如 "AUTH_FAILED", "EXECUTION_TIMEOUT"
    pub message: String, // 人类可读的错误详情
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NexusErrorCode {
    AuthFailed,
    AuthTokenExpired,
    ExecutionTimeout,
    ExecutionCancelled,
    ValidationFailed,
    DeviceNotFound,
    ProtocolMismatch,
    InternalError,
}

impl NexusErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            NexusErrorCode::AuthFailed => "AUTH_FAILED",
            NexusErrorCode::AuthTokenExpired => "AUTH_TOKEN_EXPIRED",
            NexusErrorCode::ExecutionTimeout => "EXECUTION_TIMEOUT",
            NexusErrorCode::ExecutionCancelled => "EXECUTION_CANCELLED",
            NexusErrorCode::ValidationFailed => "VALIDATION_FAILED",
            NexusErrorCode::DeviceNotFound => "DEVICE_NOT_FOUND",
            NexusErrorCode::ProtocolMismatch => "PROTOCOL_MISMATCH",
            NexusErrorCode::InternalError => "INTERNAL_ERROR",
        }
    }
}
