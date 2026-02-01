//! Error types for sandbox operations
//!
//! TigerStyle: Explicit error variants with context.

use std::fmt;

/// Result type for sandbox operations
pub type SandboxResult<T> = Result<T, SandboxError>;

/// Sandbox errors
#[derive(Debug)]
pub enum SandboxError {
    /// Sandbox not found
    NotFound { sandbox_id: String },

    /// Sandbox already exists
    AlreadyExists { sandbox_id: String },

    /// Invalid sandbox state for operation
    InvalidState {
        sandbox_id: String,
        current: String,
        expected: String,
    },

    /// Command execution failed
    ExecFailed { command: String, reason: String },

    /// Command timed out
    ExecTimeout { command: String, timeout_ms: u64 },

    /// Snapshot operation failed
    SnapshotFailed { sandbox_id: String, reason: String },

    /// Restore operation failed
    RestoreFailed { sandbox_id: String, reason: String },

    /// Resource limit exceeded
    ResourceLimitExceeded {
        resource: String,
        limit: u64,
        used: u64,
    },

    /// Pool exhausted (no available sandboxes)
    PoolExhausted { pool_size: usize },

    /// Pool acquire timeout
    PoolAcquireTimeout { timeout_ms: u64 },

    /// Configuration error
    ConfigError { reason: String },

    /// IO error
    IoError { reason: String },

    /// Internal error
    Internal { message: String },
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { sandbox_id } => {
                write!(f, "sandbox not found: {}", sandbox_id)
            }
            Self::AlreadyExists { sandbox_id } => {
                write!(f, "sandbox already exists: {}", sandbox_id)
            }
            Self::InvalidState {
                sandbox_id,
                current,
                expected,
            } => {
                write!(
                    f,
                    "sandbox {} in invalid state: expected {}, got {}",
                    sandbox_id, expected, current
                )
            }
            Self::ExecFailed { command, reason } => {
                write!(f, "command '{}' failed: {}", command, reason)
            }
            Self::ExecTimeout {
                command,
                timeout_ms,
            } => {
                write!(f, "command '{}' timed out after {}ms", command, timeout_ms)
            }
            Self::SnapshotFailed { sandbox_id, reason } => {
                write!(f, "snapshot failed for {}: {}", sandbox_id, reason)
            }
            Self::RestoreFailed { sandbox_id, reason } => {
                write!(f, "restore failed for {}: {}", sandbox_id, reason)
            }
            Self::ResourceLimitExceeded {
                resource,
                limit,
                used,
            } => {
                write!(
                    f,
                    "resource limit exceeded for {}: {} used, {} limit",
                    resource, used, limit
                )
            }
            Self::PoolExhausted { pool_size } => {
                write!(f, "sandbox pool exhausted (size: {})", pool_size)
            }
            Self::PoolAcquireTimeout { timeout_ms } => {
                write!(f, "sandbox pool acquire timeout after {}ms", timeout_ms)
            }
            Self::ConfigError { reason } => {
                write!(f, "configuration error: {}", reason)
            }
            Self::IoError { reason } => {
                write!(f, "IO error: {}", reason)
            }
            Self::Internal { message } => {
                write!(f, "internal error: {}", message)
            }
        }
    }
}

impl std::error::Error for SandboxError {}

impl From<std::io::Error> for SandboxError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError {
            reason: err.to_string(),
        }
    }
}

impl From<SandboxError> for kelpie_core::error::Error {
    fn from(err: SandboxError) -> Self {
        use kelpie_core::error::Error;
        match err {
            SandboxError::NotFound { sandbox_id } => Error::not_found("sandbox", sandbox_id),
            SandboxError::ExecTimeout {
                command,
                timeout_ms,
            } => Error::timeout(format!("sandbox exec: {}", command), timeout_ms),
            SandboxError::PoolAcquireTimeout { timeout_ms } => {
                Error::timeout("sandbox pool acquire", timeout_ms)
            }
            SandboxError::ConfigError { reason } => Error::config(reason),
            SandboxError::IoError { reason } => Error::internal(format!("IO error: {}", reason)),
            _ => Error::internal(err.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = SandboxError::NotFound {
            sandbox_id: "test-123".to_string(),
        };
        assert!(err.to_string().contains("test-123"));
    }

    #[test]
    fn test_exec_failed_display() {
        let err = SandboxError::ExecFailed {
            command: "ls".to_string(),
            reason: "permission denied".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("ls"));
        assert!(msg.contains("permission denied"));
    }
}
