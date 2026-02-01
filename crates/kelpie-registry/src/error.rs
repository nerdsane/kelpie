//! Registry error types
//!
//! TigerStyle: Explicit error variants with context.

use kelpie_core::actor::ActorId;
use thiserror::Error;

/// Registry-specific errors
#[derive(Error, Debug)]
pub enum RegistryError {
    /// Node not found in registry
    #[error("node not found: {node_id}")]
    NodeNotFound { node_id: String },

    /// Node already exists in registry
    #[error("node already exists: {node_id}")]
    NodeAlreadyExists { node_id: String },

    /// Actor not found in registry
    #[error("actor not found: {actor_id}")]
    ActorNotFound { actor_id: String },

    /// Actor already registered on another node
    #[error("actor {actor_id} already registered on node {existing_node}")]
    ActorAlreadyRegistered {
        actor_id: String,
        existing_node: String,
    },

    /// Node failed heartbeat timeout
    #[error("node {node_id} heartbeat timeout after {timeout_ms}ms")]
    HeartbeatTimeout { node_id: String, timeout_ms: u64 },

    /// Invalid node ID
    #[error("invalid node ID: {id}, reason: {reason}")]
    InvalidNodeId { id: String, reason: String },

    /// Storage operation failed
    #[error("storage error: {reason}")]
    StorageError { reason: String },

    /// Internal registry error
    #[error("internal error: {message}")]
    Internal { message: String },

    /// Lease is held by another node
    #[error("lease for actor {actor_id} held by {holder}, expires at {expiry_ms}ms")]
    LeaseHeldByOther {
        actor_id: String,
        holder: String,
        expiry_ms: u64,
    },

    /// Lease not found (no active lease for actor)
    #[error("no lease found for actor {actor_id}")]
    LeaseNotFound { actor_id: String },

    /// Not the lease holder (cannot renew/release)
    #[error("node {requester} is not the lease holder for actor {actor_id}, holder is {holder}")]
    NotLeaseHolder {
        actor_id: String,
        holder: String,
        requester: String,
    },

    /// Lease has expired
    #[error("lease for actor {actor_id} expired at {expiry_ms}ms")]
    LeaseExpired { actor_id: String, expiry_ms: u64 },
}

impl From<RegistryError> for kelpie_core::error::Error {
    fn from(err: RegistryError) -> Self {
        use kelpie_core::error::Error;
        match err {
            RegistryError::NodeNotFound { node_id } => Error::not_found("node", node_id),
            RegistryError::ActorNotFound { actor_id } => Error::not_found("actor", actor_id),
            RegistryError::HeartbeatTimeout {
                node_id,
                timeout_ms,
            } => Error::timeout(format!("heartbeat for node {}", node_id), timeout_ms),
            _ => Error::internal(err.to_string()),
        }
    }
}

impl RegistryError {
    /// Create a node not found error
    pub fn node_not_found(node_id: impl Into<String>) -> Self {
        Self::NodeNotFound {
            node_id: node_id.into(),
        }
    }

    /// Create an actor not found error
    pub fn actor_not_found(actor_id: &ActorId) -> Self {
        Self::ActorNotFound {
            actor_id: actor_id.qualified_name(),
        }
    }

    /// Create an actor already registered error
    pub fn actor_already_registered(actor_id: &ActorId, existing_node: impl Into<String>) -> Self {
        Self::ActorAlreadyRegistered {
            actor_id: actor_id.qualified_name(),
            existing_node: existing_node.into(),
        }
    }

    /// Check if this error indicates a retriable condition
    pub fn is_retriable(&self) -> bool {
        matches!(self, Self::StorageError { .. })
    }
}

/// Result type for registry operations
pub type RegistryResult<T> = std::result::Result<T, RegistryError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = RegistryError::node_not_found("node-1");
        assert!(err.to_string().contains("node-1"));
    }

    #[test]
    fn test_error_retriable() {
        let storage_err = RegistryError::StorageError {
            reason: "timeout".into(),
        };
        assert!(storage_err.is_retriable());

        let not_found = RegistryError::node_not_found("x");
        assert!(!not_found.is_retriable());
    }
}
