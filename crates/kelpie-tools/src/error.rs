//! Tool error types
//!
//! TigerStyle: Explicit error variants with context.

use thiserror::Error;

/// Result type for tool operations
pub type ToolResult<T> = Result<T, ToolError>;

/// Errors that can occur during tool operations
#[derive(Error, Debug)]
pub enum ToolError {
    /// Tool not found in registry
    #[error("tool not found: {name}")]
    NotFound { name: String },

    /// Tool already registered
    #[error("tool already registered: {name}")]
    AlreadyRegistered { name: String },

    /// Invalid tool input
    #[error("invalid input for tool '{tool}': {reason}")]
    InvalidInput { tool: String, reason: String },

    /// Missing required parameter
    #[error("missing required parameter '{param}' for tool '{tool}'")]
    MissingParameter { tool: String, param: String },

    /// Invalid parameter type
    #[error(
        "invalid parameter type for '{param}' in tool '{tool}': expected {expected}, got {actual}"
    )]
    InvalidParameterType {
        tool: String,
        param: String,
        expected: String,
        actual: String,
    },

    /// Tool execution failed
    #[error("tool '{tool}' execution failed: {reason}")]
    ExecutionFailed { tool: String, reason: String },

    /// Tool execution timed out
    #[error("tool '{tool}' execution timed out after {timeout_ms}ms")]
    ExecutionTimeout { tool: String, timeout_ms: u64 },

    /// Permission denied for tool operation
    #[error("permission denied for tool '{tool}': {reason}")]
    PermissionDenied { tool: String, reason: String },

    /// Sandbox execution error
    #[error("sandbox error for tool '{tool}': {reason}")]
    SandboxError { tool: String, reason: String },

    /// MCP protocol error
    #[error("MCP protocol error: {reason}")]
    McpProtocolError { reason: String },

    /// MCP connection error
    #[error("MCP connection error: {reason}")]
    McpConnectionError { reason: String },

    /// Configuration error
    #[error("configuration error: {reason}")]
    ConfigError { reason: String },

    /// IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// JSON serialization error
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    /// Internal error
    #[error("internal error: {message}")]
    Internal { message: String },
}

impl From<ToolError> for kelpie_core::error::Error {
    fn from(err: ToolError) -> Self {
        use kelpie_core::error::Error;
        match err {
            ToolError::NotFound { name } => Error::not_found("tool", name),
            ToolError::ExecutionTimeout { tool, timeout_ms } => {
                Error::timeout(format!("tool execution: {}", tool), timeout_ms)
            }
            ToolError::ConfigError { reason } => Error::config(reason),
            ToolError::IoError(io_err) => Error::Io(io_err),
            _ => Error::internal(err.to_string()),
        }
    }
}

impl From<kelpie_sandbox::SandboxError> for ToolError {
    fn from(err: kelpie_sandbox::SandboxError) -> Self {
        ToolError::SandboxError {
            tool: "unknown".to_string(),
            reason: err.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = ToolError::NotFound {
            name: "test-tool".to_string(),
        };
        assert!(err.to_string().contains("test-tool"));
    }

    #[test]
    fn test_missing_parameter_display() {
        let err = ToolError::MissingParameter {
            tool: "shell".to_string(),
            param: "command".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("shell"));
        assert!(msg.contains("command"));
    }

    #[test]
    fn test_execution_timeout_display() {
        let err = ToolError::ExecutionTimeout {
            tool: "slow-tool".to_string(),
            timeout_ms: 30000,
        };
        let msg = err.to_string();
        assert!(msg.contains("slow-tool"));
        assert!(msg.contains("30000"));
    }
}
