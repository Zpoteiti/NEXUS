/// Centralized error types for the NEXUS system.
///
/// `ErrorCode` is the single source of truth for all error categories.
/// `ApiError` is the standard JSON error response used by nexus-server HTTP handlers.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    // Auth
    AuthFailed,
    AuthTokenExpired,
    Unauthorized,
    Forbidden,

    // General
    NotFound,
    Conflict,
    ValidationFailed,
    InvalidParams,
    ExecutionFailed,
    ExecutionTimeout,

    // Device
    DeviceNotFound,
    DeviceOffline,

    // Protocol
    ProtocolMismatch,
    InternalError,

    // Tool execution
    ToolBlocked,
    ToolTimeout,
    ToolNotFound,
    ToolInvalidParams,

    // MCP
    McpConnectionFailed,
    McpCallFailed,

    // WebSocket / Protocol
    ConnectionFailed,
    HandshakeFailed,
    ChannelError,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::AuthFailed => "AUTH_FAILED",
            ErrorCode::AuthTokenExpired => "AUTH_TOKEN_EXPIRED",
            ErrorCode::Unauthorized => "UNAUTHORIZED",
            ErrorCode::Forbidden => "FORBIDDEN",
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::Conflict => "CONFLICT",
            ErrorCode::ValidationFailed => "VALIDATION_FAILED",
            ErrorCode::InvalidParams => "INVALID_PARAMS",
            ErrorCode::ExecutionFailed => "EXECUTION_FAILED",
            ErrorCode::ExecutionTimeout => "EXECUTION_TIMEOUT",
            ErrorCode::DeviceNotFound => "DEVICE_NOT_FOUND",
            ErrorCode::DeviceOffline => "DEVICE_OFFLINE",
            ErrorCode::ProtocolMismatch => "PROTOCOL_MISMATCH",
            ErrorCode::InternalError => "INTERNAL_ERROR",
            ErrorCode::ToolBlocked => "TOOL_BLOCKED",
            ErrorCode::ToolTimeout => "TOOL_TIMEOUT",
            ErrorCode::ToolNotFound => "TOOL_NOT_FOUND",
            ErrorCode::ToolInvalidParams => "TOOL_INVALID_PARAMS",
            ErrorCode::McpConnectionFailed => "MCP_CONNECTION_FAILED",
            ErrorCode::McpCallFailed => "MCP_CALL_FAILED",
            ErrorCode::ConnectionFailed => "CONNECTION_FAILED",
            ErrorCode::HandshakeFailed => "HANDSHAKE_FAILED",
            ErrorCode::ChannelError => "CHANNEL_ERROR",
        }
    }

    pub fn http_status(&self) -> u16 {
        match self {
            ErrorCode::AuthFailed => 401,
            ErrorCode::AuthTokenExpired => 401,
            ErrorCode::Unauthorized => 401,
            ErrorCode::Forbidden => 403,
            ErrorCode::NotFound => 404,
            ErrorCode::Conflict => 409,
            ErrorCode::ValidationFailed => 400,
            ErrorCode::InvalidParams => 400,
            ErrorCode::ExecutionFailed => 500,
            ErrorCode::ExecutionTimeout => 504,
            ErrorCode::DeviceNotFound => 404,
            ErrorCode::DeviceOffline => 503,
            ErrorCode::ProtocolMismatch => 400,
            ErrorCode::InternalError => 500,
            ErrorCode::ToolBlocked => 403,
            ErrorCode::ToolTimeout => 504,
            ErrorCode::ToolNotFound => 404,
            ErrorCode::ToolInvalidParams => 400,
            ErrorCode::McpConnectionFailed => 502,
            ErrorCode::McpCallFailed => 502,
            ErrorCode::ConnectionFailed => 502,
            ErrorCode::HandshakeFailed => 502,
            ErrorCode::ChannelError => 500,
        }
    }

    /// Parse an error code string (e.g. "AUTH_FAILED") back into an `ErrorCode`.
    pub fn from_str(s: &str) -> Option<ErrorCode> {
        match s {
            "AUTH_FAILED" => Some(ErrorCode::AuthFailed),
            "AUTH_TOKEN_EXPIRED" => Some(ErrorCode::AuthTokenExpired),
            "UNAUTHORIZED" => Some(ErrorCode::Unauthorized),
            "FORBIDDEN" => Some(ErrorCode::Forbidden),
            "NOT_FOUND" => Some(ErrorCode::NotFound),
            "CONFLICT" => Some(ErrorCode::Conflict),
            "VALIDATION_FAILED" => Some(ErrorCode::ValidationFailed),
            "INVALID_PARAMS" => Some(ErrorCode::InvalidParams),
            "EXECUTION_FAILED" => Some(ErrorCode::ExecutionFailed),
            "EXECUTION_TIMEOUT" => Some(ErrorCode::ExecutionTimeout),
            "DEVICE_NOT_FOUND" => Some(ErrorCode::DeviceNotFound),
            "DEVICE_OFFLINE" => Some(ErrorCode::DeviceOffline),
            "PROTOCOL_MISMATCH" => Some(ErrorCode::ProtocolMismatch),
            "INTERNAL_ERROR" => Some(ErrorCode::InternalError),
            "TOOL_BLOCKED" => Some(ErrorCode::ToolBlocked),
            "TOOL_TIMEOUT" => Some(ErrorCode::ToolTimeout),
            "TOOL_NOT_FOUND" => Some(ErrorCode::ToolNotFound),
            "TOOL_INVALID_PARAMS" => Some(ErrorCode::ToolInvalidParams),
            "MCP_CONNECTION_FAILED" => Some(ErrorCode::McpConnectionFailed),
            "MCP_CALL_FAILED" => Some(ErrorCode::McpCallFailed),
            "CONNECTION_FAILED" => Some(ErrorCode::ConnectionFailed),
            "HANDSHAKE_FAILED" => Some(ErrorCode::HandshakeFailed),
            "CHANNEL_ERROR" => Some(ErrorCode::ChannelError),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code: code.as_str().to_string(),
            message: message.into(),
        }
    }

    /// Derive the HTTP status code from the error code string.
    /// Delegates to `ErrorCode::http_status()` so the mapping is defined in one place.
    pub fn http_status_code(&self) -> u16 {
        ErrorCode::from_str(&self.code)
            .map(|ec| ec.http_status())
            .unwrap_or(500)
    }
}

/// Internal error type for cross-crate use (not tied to HTTP).
#[derive(Debug, Clone)]
pub struct NexusError {
    pub code: ErrorCode,
    pub message: String,
}

impl NexusError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl std::fmt::Display for NexusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for NexusError {}

impl From<NexusError> for ApiError {
    fn from(e: NexusError) -> Self {
        ApiError::new(e.code, e.message)
    }
}

#[cfg(feature = "axum")]
impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = axum::http::StatusCode::from_u16(self.http_status_code())
            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        (status, axum::Json(self)).into_response()
    }
}
