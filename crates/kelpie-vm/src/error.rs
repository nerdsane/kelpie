//! Error types for VM operations
//!
//! TigerStyle: Explicit error variants with context for debugging.

use thiserror::Error;

/// Result type for libkrun operations
pub type VmResult<T> = Result<T, VmError>;

/// Errors that can occur during VM operations
#[derive(Error, Debug)]
pub enum VmError {
    // ========================================================================
    // Configuration Errors
    // ========================================================================
    /// Invalid VM configuration
    #[error("invalid VM configuration: {reason}")]
    ConfigInvalid { reason: String },

    /// Root disk not found
    #[error("root disk not found: {path}")]
    RootDiskNotFound { path: String },

    /// Root disk not accessible
    #[error("root disk not accessible: {path}: {reason}")]
    RootDiskNotAccessible { path: String, reason: String },

    // ========================================================================
    // Lifecycle Errors
    // ========================================================================
    /// VM creation failed
    #[error("VM creation failed: {reason}")]
    CreateFailed { reason: String },

    /// VM boot failed
    #[error("VM boot failed: {reason}")]
    BootFailed { reason: String },

    /// VM boot timed out
    #[error("VM boot timed out after {timeout_ms}ms")]
    BootTimeout { timeout_ms: u64 },

    /// VM not running when operation requires it
    #[error("VM not running, current state: {state}")]
    NotRunning { state: String },

    /// VM already running when trying to start
    #[error("VM already running")]
    AlreadyRunning,

    /// VM crash detected
    #[error("VM crashed: {reason}")]
    Crashed { reason: String },

    // ========================================================================
    // Execution Errors
    // ========================================================================
    /// Command execution failed
    #[error("exec failed: {reason}")]
    ExecFailed { reason: String },

    /// Command execution timed out
    #[error("exec timed out after {timeout_ms}ms")]
    ExecTimeout { timeout_ms: u64 },

    // ========================================================================
    // Pause/Resume Errors
    // ========================================================================
    /// Pause operation failed
    #[error("pause failed: {reason}")]
    PauseFailed { reason: String },

    /// Resume operation failed
    #[error("resume failed: {reason}")]
    ResumeFailed { reason: String },

    // ========================================================================
    // Snapshot Errors
    // ========================================================================
    /// Snapshot creation failed
    #[error("snapshot creation failed: {reason}")]
    SnapshotFailed { reason: String },

    /// Snapshot too large
    #[error("snapshot too large: {size_bytes} bytes exceeds max {max_bytes}")]
    SnapshotTooLarge { size_bytes: u64, max_bytes: u64 },

    /// Snapshot restore failed
    #[error("snapshot restore failed: {reason}")]
    RestoreFailed { reason: String },

    /// Snapshot corrupted
    #[error("snapshot corrupted: checksum mismatch")]
    SnapshotCorrupted,

    // ========================================================================
    // VirtioFs Errors
    // ========================================================================
    /// VirtioFs mount failed
    #[error("virtio-fs mount failed for {path}: {reason}")]
    VirtioFsMountFailed { path: String, reason: String },

    /// Too many VirtioFs mounts
    #[error("too many virtio-fs mounts: {count} exceeds max {max}")]
    VirtioFsTooManyMounts { count: usize, max: usize },

    // ========================================================================
    // FFI Errors
    // ========================================================================
    /// FFI call failed
    #[error("libkrun FFI error: {reason}")]
    FfiError { reason: String },

    /// libkrun context creation failed
    #[error("libkrun context creation failed: {reason}")]
    ContextCreationFailed { reason: String },

    /// VM configuration failed
    #[error("VM configuration failed: {reason}")]
    ConfigurationFailed { reason: String },

    // ========================================================================
    // Internal Errors
    // ========================================================================
    /// Internal error (should not happen)
    #[error("internal error: {reason}")]
    Internal { reason: String },
}

impl From<VmError> for kelpie_core::error::Error {
    fn from(err: VmError) -> Self {
        use kelpie_core::error::Error;
        match err {
            VmError::BootTimeout { timeout_ms } => Error::timeout("VM boot", timeout_ms),
            VmError::ExecTimeout { timeout_ms } => Error::timeout("VM exec", timeout_ms),
            VmError::ConfigInvalid { reason } | VmError::ConfigurationFailed { reason } => {
                Error::config(reason)
            }
            VmError::RootDiskNotFound { path } => Error::not_found("root_disk", path),
            _ => Error::internal(err.to_string()),
        }
    }
}

impl VmError {
    /// Check if this error is retriable
    pub fn is_retriable(&self) -> bool {
        matches!(
            self,
            VmError::BootTimeout { .. } | VmError::ExecTimeout { .. } | VmError::Crashed { .. }
        )
    }

    /// Check if this error indicates the VM should be recreated
    pub fn requires_recreate(&self) -> bool {
        matches!(
            self,
            VmError::Crashed { .. } | VmError::SnapshotCorrupted | VmError::Internal { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = VmError::BootTimeout { timeout_ms: 5000 };
        assert!(err.to_string().contains("5000ms"));
    }

    #[test]
    fn test_error_retriable() {
        assert!(VmError::BootTimeout { timeout_ms: 5000 }.is_retriable());
        assert!(VmError::ExecTimeout { timeout_ms: 1000 }.is_retriable());
        assert!(!VmError::ConfigInvalid {
            reason: "test".into()
        }
        .is_retriable());
    }

    #[test]
    fn test_error_requires_recreate() {
        assert!(VmError::Crashed {
            reason: "test".into()
        }
        .requires_recreate());
        assert!(VmError::SnapshotCorrupted.requires_recreate());
        assert!(!VmError::BootTimeout { timeout_ms: 5000 }.requires_recreate());
    }
}
