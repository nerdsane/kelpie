//! Cluster Membership Types
//!
//! Types shared between `ClusterMembership` (FDB) and `TestableClusterMembership` (DST).
//! These types do not require FDB and can be used in all contexts.
//!
//! TigerStyle: Explicit state management, clear serialization.

use crate::membership::NodeState;
use crate::node::NodeId;
use serde::{Deserialize, Serialize};

// =============================================================================
// ClusterNodeInfo
// =============================================================================

/// Node information stored in cluster namespace
///
/// Used by both FDB-backed and mock storage implementations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterNodeInfo {
    /// Node ID
    pub id: NodeId,
    /// Node state matching TLA+
    pub state: NodeState,
    /// Last heartbeat timestamp (epoch ms)
    pub last_heartbeat_ms: u64,
    /// RPC address for communication
    pub rpc_addr: String,
    /// When node joined (epoch ms)
    pub joined_at_ms: u64,
}

impl ClusterNodeInfo {
    /// Create new cluster node info
    ///
    /// # Arguments
    /// * `id` - Node ID
    /// * `rpc_addr` - RPC address for communication
    /// * `now_ms` - Current timestamp
    ///
    /// # Preconditions
    /// * `rpc_addr` must be non-empty
    pub fn new(id: NodeId, rpc_addr: String, now_ms: u64) -> Self {
        assert!(!rpc_addr.is_empty(), "rpc_addr cannot be empty");

        Self {
            id,
            state: NodeState::Left,
            last_heartbeat_ms: now_ms,
            rpc_addr,
            joined_at_ms: 0,
        }
    }

    /// Check if heartbeat has timed out
    ///
    /// # Arguments
    /// * `now_ms` - Current timestamp
    /// * `timeout_ms` - Timeout threshold
    ///
    /// # Returns
    /// * `true` if heartbeat is older than timeout
    pub fn is_heartbeat_timeout(&self, now_ms: u64, timeout_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_heartbeat_ms) > timeout_ms
    }
}

// =============================================================================
// MigrationCandidate
// =============================================================================

/// Actor that needs to be migrated due to node failure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationCandidate {
    /// Actor that needs migration
    pub actor_id: String,
    /// Node that failed
    pub failed_node_id: NodeId,
    /// When the failure was detected (epoch ms)
    pub detected_at_ms: u64,
}

impl MigrationCandidate {
    /// Create a new migration candidate
    ///
    /// # Preconditions
    /// * `actor_id` must be non-empty
    pub fn new(actor_id: String, failed_node_id: NodeId, detected_at_ms: u64) -> Self {
        assert!(!actor_id.is_empty(), "actor_id cannot be empty");

        Self {
            actor_id,
            failed_node_id,
            detected_at_ms,
        }
    }
}

// =============================================================================
// MigrationResult
// =============================================================================

/// Result of an actor migration
#[derive(Debug, Clone)]
pub enum MigrationResult {
    /// Migration succeeded, actor now on new node
    Success {
        actor_id: String,
        new_node_id: NodeId,
    },
    /// Migration failed, no capacity available
    NoCapacity { actor_id: String },
    /// Migration failed with error
    Failed { actor_id: String, reason: String },
}

impl MigrationResult {
    /// Check if migration was successful
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }

    /// Get the actor ID
    pub fn actor_id(&self) -> &str {
        match self {
            Self::Success { actor_id, .. } => actor_id,
            Self::NoCapacity { actor_id } => actor_id,
            Self::Failed { actor_id, .. } => actor_id,
        }
    }
}

// =============================================================================
// MigrationQueue
// =============================================================================

/// Actors pending migration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MigrationQueue {
    /// Actors pending migration
    pub candidates: Vec<MigrationCandidate>,
    /// Last updated timestamp
    pub updated_at_ms: u64,
}

impl MigrationQueue {
    /// Create a new empty migration queue
    pub fn new() -> Self {
        Self {
            candidates: Vec::new(),
            updated_at_ms: 0,
        }
    }

    /// Add a candidate to the queue
    ///
    /// # Arguments
    /// * `candidate` - Migration candidate to add
    /// * `now_ms` - Current timestamp
    pub fn add(&mut self, candidate: MigrationCandidate, now_ms: u64) {
        self.candidates.push(candidate);
        self.updated_at_ms = now_ms;
    }

    /// Remove a candidate from the queue by actor_id
    ///
    /// # Returns
    /// * `true` if candidate was found and removed
    /// * `false` if not found
    pub fn remove(&mut self, actor_id: &str, now_ms: u64) -> bool {
        let len_before = self.candidates.len();
        self.candidates.retain(|c| c.actor_id != actor_id);
        let removed = self.candidates.len() < len_before;
        if removed {
            self.updated_at_ms = now_ms;
        }
        removed
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    /// Get the number of pending migrations
    pub fn len(&self) -> usize {
        self.candidates.len()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_node_id(n: u32) -> NodeId {
        NodeId::new(format!("node-{}", n)).unwrap()
    }

    #[test]
    fn test_cluster_node_info() {
        let node_id = test_node_id(1);
        let info = ClusterNodeInfo::new(node_id.clone(), "127.0.0.1:8080".to_string(), 1000);

        assert_eq!(info.id, node_id);
        assert_eq!(info.state, NodeState::Left);
        assert_eq!(info.last_heartbeat_ms, 1000);

        // Heartbeat timeout check
        assert!(!info.is_heartbeat_timeout(2000, 5000)); // 2000 - 1000 = 1000 < 5000
        assert!(info.is_heartbeat_timeout(7000, 5000)); // 7000 - 1000 = 6000 > 5000
    }

    #[test]
    fn test_migration_candidate() {
        let node_id = test_node_id(1);
        let candidate = MigrationCandidate::new("test/actor-1".to_string(), node_id.clone(), 1000);

        assert_eq!(candidate.actor_id, "test/actor-1");
        assert_eq!(candidate.failed_node_id, node_id);
        assert_eq!(candidate.detected_at_ms, 1000);
    }

    #[test]
    #[should_panic(expected = "actor_id cannot be empty")]
    fn test_migration_candidate_empty_actor_id_panics() {
        let node_id = test_node_id(1);
        MigrationCandidate::new(String::new(), node_id, 1000);
    }

    #[test]
    fn test_migration_result() {
        let node_id = test_node_id(1);

        let success = MigrationResult::Success {
            actor_id: "test/actor-1".to_string(),
            new_node_id: node_id,
        };
        assert!(success.is_success());
        assert_eq!(success.actor_id(), "test/actor-1");

        let no_capacity = MigrationResult::NoCapacity {
            actor_id: "test/actor-2".to_string(),
        };
        assert!(!no_capacity.is_success());
        assert_eq!(no_capacity.actor_id(), "test/actor-2");

        let failed = MigrationResult::Failed {
            actor_id: "test/actor-3".to_string(),
            reason: "connection refused".to_string(),
        };
        assert!(!failed.is_success());
        assert_eq!(failed.actor_id(), "test/actor-3");
    }

    #[test]
    fn test_migration_queue() {
        let mut queue = MigrationQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);

        let node_id = test_node_id(1);
        let candidate1 = MigrationCandidate::new("test/actor-1".to_string(), node_id.clone(), 1000);
        let candidate2 = MigrationCandidate::new("test/actor-2".to_string(), node_id.clone(), 1000);

        queue.add(candidate1, 1000);
        assert!(!queue.is_empty());
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.updated_at_ms, 1000);

        queue.add(candidate2, 2000);
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.updated_at_ms, 2000);

        // Remove first actor
        let removed = queue.remove("test/actor-1", 3000);
        assert!(removed);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.updated_at_ms, 3000);

        // Try to remove non-existent actor
        let removed = queue.remove("test/actor-nonexistent", 4000);
        assert!(!removed);
        assert_eq!(queue.updated_at_ms, 3000); // Not updated

        // Remove last actor
        let removed = queue.remove("test/actor-2", 5000);
        assert!(removed);
        assert!(queue.is_empty());
    }
}
