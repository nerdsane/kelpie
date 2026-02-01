//! Cluster error types
//!
//! TigerStyle: Explicit error variants with context.

use kelpie_registry::{NodeId, RegistryError};
use thiserror::Error;

/// Cluster-specific errors
#[derive(Error, Debug)]
pub enum ClusterError {
    /// Local node not initialized
    #[error("local node not initialized")]
    NotInitialized,

    /// Cluster already started
    #[error("cluster already started")]
    AlreadyStarted,

    /// Cluster not started
    #[error("cluster not started")]
    NotStarted,

    /// Node not reachable
    #[error("node {node_id} not reachable: {reason}")]
    NodeUnreachable { node_id: String, reason: String },

    /// RPC failed
    #[error("RPC to {node_id} failed: {reason}")]
    RpcFailed { node_id: String, reason: String },

    /// RPC timeout
    #[error("RPC to {node_id} timed out after {timeout_ms}ms")]
    RpcTimeout { node_id: String, timeout_ms: u64 },

    /// Migration failed
    #[error("migration of actor {actor_id} from {from_node} to {to_node} failed: {reason}")]
    MigrationFailed {
        actor_id: String,
        from_node: String,
        to_node: String,
        reason: String,
    },

    /// No available nodes for placement
    #[error("no available nodes for actor {actor_id}")]
    NoAvailableNodes { actor_id: String },

    /// No quorum - insufficient nodes to form majority
    ///
    /// This error occurs during network partitions when this partition
    /// cannot reach enough nodes to form a majority (strict > total/2).
    #[error(
        "no quorum: {available_nodes} of {total_nodes} nodes reachable (need > {total_nodes}/2)"
    )]
    NoQuorum {
        /// Number of nodes reachable in this partition
        available_nodes: usize,
        /// Total cluster size
        total_nodes: usize,
        /// Operation that was attempted
        operation: String,
    },

    /// Registry error
    #[error("registry error: {0}")]
    Registry(#[from] RegistryError),

    /// Core error
    #[error("core error: {0}")]
    Core(#[from] kelpie_core::error::Error),

    /// Internal error
    #[error("internal error: {message}")]
    Internal { message: String },
}

impl ClusterError {
    /// Create a node unreachable error
    pub fn node_unreachable(node_id: &NodeId, reason: impl Into<String>) -> Self {
        Self::NodeUnreachable {
            node_id: node_id.to_string(),
            reason: reason.into(),
        }
    }

    /// Create an RPC failed error
    pub fn rpc_failed(node_id: &NodeId, reason: impl Into<String>) -> Self {
        Self::RpcFailed {
            node_id: node_id.to_string(),
            reason: reason.into(),
        }
    }

    /// Create an RPC timeout error
    pub fn rpc_timeout(node_id: &NodeId, timeout_ms: u64) -> Self {
        Self::RpcTimeout {
            node_id: node_id.to_string(),
            timeout_ms,
        }
    }

    /// Create a no quorum error
    pub fn no_quorum(
        available_nodes: usize,
        total_nodes: usize,
        operation: impl Into<String>,
    ) -> Self {
        Self::NoQuorum {
            available_nodes,
            total_nodes,
            operation: operation.into(),
        }
    }

    /// Check if we have quorum (strict majority)
    ///
    /// Quorum requires > total/2 nodes (e.g., 3 of 5, 2 of 3).
    pub fn check_quorum(
        available: usize,
        total: usize,
        operation: impl Into<String>,
    ) -> Result<(), Self> {
        if available > total / 2 {
            Ok(())
        } else {
            Err(Self::no_quorum(available, total, operation))
        }
    }

    /// Check if this error is retriable
    pub fn is_retriable(&self) -> bool {
        matches!(
            self,
            Self::NodeUnreachable { .. }
                | Self::RpcFailed { .. }
                | Self::RpcTimeout { .. }
                | Self::Registry(RegistryError::StorageError { .. })
        )
    }
}

/// Result type for cluster operations
pub type ClusterResult<T> = std::result::Result<T, ClusterError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = ClusterError::NotInitialized;
        assert!(err.to_string().contains("not initialized"));
    }

    #[test]
    fn test_error_retriable() {
        let rpc_err = ClusterError::RpcTimeout {
            node_id: "node-1".into(),
            timeout_ms: 5000,
        };
        assert!(rpc_err.is_retriable());

        let not_init = ClusterError::NotInitialized;
        assert!(!not_init.is_retriable());
    }
}
