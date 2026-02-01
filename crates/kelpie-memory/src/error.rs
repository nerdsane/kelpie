//! Error types for the memory system
//!
//! TigerStyle: Explicit error variants with context.

use kelpie_core::error::Error as CoreError;
use std::fmt;

/// Result type for memory operations
pub type MemoryResult<T> = Result<T, MemoryError>;

/// Memory system errors
#[derive(Debug)]
pub enum MemoryError {
    /// Core memory capacity exceeded
    CoreMemoryFull {
        current_bytes: u64,
        max_bytes: u64,
        requested_bytes: u64,
    },

    /// Block not found
    BlockNotFound { block_id: String },

    /// Invalid block type for operation
    InvalidBlockType { expected: String, actual: String },

    /// Working memory key not found
    KeyNotFound { key: String },

    /// Serialization error
    SerializationFailed { reason: String },

    /// Deserialization error
    DeserializationFailed { reason: String },

    /// Storage error (from kelpie-storage)
    StorageError { reason: String },

    /// Checkpoint error
    CheckpointFailed { reason: String },

    /// Search error
    SearchFailed { reason: String },

    /// Invalid configuration
    InvalidConfig { reason: String },

    /// Block content too large
    BlockTooLarge {
        block_id: String,
        size_bytes: u64,
        max_bytes: u64,
    },

    /// Internal error
    Internal { message: String },

    /// Embedding generation error
    EmbeddingError { message: String },
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CoreMemoryFull {
                current_bytes,
                max_bytes,
                requested_bytes,
            } => {
                write!(
                    f,
                    "core memory full: {} bytes used of {} max, cannot add {} bytes",
                    current_bytes, max_bytes, requested_bytes
                )
            }
            Self::BlockNotFound { block_id } => {
                write!(f, "memory block not found: {}", block_id)
            }
            Self::InvalidBlockType { expected, actual } => {
                write!(
                    f,
                    "invalid block type: expected {}, got {}",
                    expected, actual
                )
            }
            Self::KeyNotFound { key } => {
                write!(f, "working memory key not found: {}", key)
            }
            Self::SerializationFailed { reason } => {
                write!(f, "serialization failed: {}", reason)
            }
            Self::DeserializationFailed { reason } => {
                write!(f, "deserialization failed: {}", reason)
            }
            Self::StorageError { reason } => {
                write!(f, "storage error: {}", reason)
            }
            Self::CheckpointFailed { reason } => {
                write!(f, "checkpoint failed: {}", reason)
            }
            Self::SearchFailed { reason } => {
                write!(f, "search failed: {}", reason)
            }
            Self::InvalidConfig { reason } => {
                write!(f, "invalid configuration: {}", reason)
            }
            Self::BlockTooLarge {
                block_id,
                size_bytes,
                max_bytes,
            } => {
                write!(
                    f,
                    "block {} too large: {} bytes exceeds {} max",
                    block_id, size_bytes, max_bytes
                )
            }
            Self::Internal { message } => {
                write!(f, "internal error: {}", message)
            }
            Self::EmbeddingError { message } => {
                write!(f, "embedding error: {}", message)
            }
        }
    }
}

impl std::error::Error for MemoryError {}

impl From<CoreError> for MemoryError {
    fn from(err: CoreError) -> Self {
        Self::StorageError {
            reason: err.to_string(),
        }
    }
}

impl From<MemoryError> for CoreError {
    fn from(err: MemoryError) -> Self {
        match err {
            MemoryError::BlockNotFound { block_id } => {
                CoreError::not_found("memory_block", block_id)
            }
            MemoryError::KeyNotFound { key } => CoreError::not_found("working_memory_key", key),
            MemoryError::InvalidConfig { reason } => CoreError::config(reason),
            MemoryError::SerializationFailed { reason } => {
                CoreError::SerializationFailed { reason }
            }
            MemoryError::DeserializationFailed { reason } => {
                CoreError::DeserializationFailed { reason }
            }
            _ => CoreError::internal(err.to_string()),
        }
    }
}

impl From<serde_json::Error> for MemoryError {
    fn from(err: serde_json::Error) -> Self {
        Self::SerializationFailed {
            reason: err.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = MemoryError::CoreMemoryFull {
            current_bytes: 30000,
            max_bytes: 32768,
            requested_bytes: 5000,
        };
        let msg = err.to_string();
        assert!(msg.contains("30000"));
        assert!(msg.contains("32768"));
        assert!(msg.contains("5000"));
    }

    #[test]
    fn test_block_not_found_display() {
        let err = MemoryError::BlockNotFound {
            block_id: "block-123".to_string(),
        };
        assert!(err.to_string().contains("block-123"));
    }
}
