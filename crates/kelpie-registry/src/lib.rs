//! Kelpie Registry
//!
//! Actor placement and discovery for Kelpie clusters.
//!
//! # Overview
//!
//! The registry provides:
//! - Actor-to-node placement tracking (single activation guarantee)
//! - Heartbeat-based failure detection
//! - Actor migration coordination
//! - Multiple backends (Memory, FoundationDB planned)
//!
//! # TigerStyle
//! - Explicit placement with linearizable operations
//! - Bounded heartbeat intervals
//! - Observable failure detection
//!
//! # Example
//!
//! ```rust,ignore
//! use kelpie_registry::{MemoryRegistry, Registry, NodeInfo, NodeId};
//!
//! let registry = MemoryRegistry::new();
//!
//! // Register a node
//! let node_id = NodeId::new("node-1")?;
//! let info = NodeInfo::new(node_id.clone(), "127.0.0.1:8080".parse()?);
//! registry.register_node(info).await?;
//!
//! // Register an actor
//! let actor_id = ActorId::new("agents", "agent-1")?;
//! registry.register_actor(actor_id, node_id).await?;
//! ```

#[cfg(feature = "fdb")]
mod cluster;
mod cluster_storage;
mod cluster_testable;
mod cluster_types;
mod error;
#[cfg(feature = "fdb")]
mod fdb;
mod heartbeat;
mod lease;
mod membership;
mod node;
mod placement;
mod registry;

#[cfg(feature = "fdb")]
pub use cluster::{ClusterMembership, ELECTION_TIMEOUT_MS, PRIMARY_STEPDOWN_DELAY_MS};
pub use cluster_storage::{ClusterStorageBackend, MockClusterStorage};
pub use cluster_testable::{
    TestableClusterMembership, ELECTION_TIMEOUT_MS as TESTABLE_ELECTION_TIMEOUT_MS,
    PRIMARY_STEPDOWN_DELAY_MS as TESTABLE_PRIMARY_STEPDOWN_DELAY_MS,
};
pub use cluster_types::{ClusterNodeInfo, MigrationCandidate, MigrationQueue, MigrationResult};
pub use error::{RegistryError, RegistryResult};
#[cfg(feature = "fdb")]
pub use fdb::{FdbRegistry, FdbRegistryConfig, Lease as FdbLease, LeaseRenewalTask};
pub use heartbeat::{
    Heartbeat, HeartbeatConfig, HeartbeatTracker, NodeHeartbeatState, HEARTBEAT_FAILURE_COUNT,
    HEARTBEAT_INTERVAL_MS_MAX, HEARTBEAT_INTERVAL_MS_MIN, HEARTBEAT_SUSPECT_COUNT,
};
pub use lease::{
    Lease, LeaseConfig, LeaseManager, MemoryLeaseManager, LEASE_DURATION_MS_DEFAULT,
    LEASE_DURATION_MS_MAX, LEASE_DURATION_MS_MIN,
};
pub use membership::{
    ClusterState, MembershipView, NodeState, PrimaryInfo, HEARTBEAT_FAILURE_THRESHOLD,
    HEARTBEAT_INTERVAL_MS, HEARTBEAT_SUSPECT_THRESHOLD, MEMBERSHIP_VIEW_NUMBER_MAX,
    PRIMARY_TERM_MAX,
};
pub use node::{NodeId, NodeInfo, NodeStatus, NODE_ID_LENGTH_BYTES_MAX};
pub use placement::{
    validate_placement, ActorPlacement, PlacementContext, PlacementDecision, PlacementStrategy,
};
// Re-export deprecated items for backward compatibility
#[allow(deprecated)]
pub use registry::{Clock, MemoryRegistry, MockClock, Registry, SystemClock};

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn test_addr() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080)
    }

    #[test]
    fn test_registry_module_compiles() {
        // Verify public types are accessible
        let _node_id = NodeId::new("test-node").unwrap();
        let _status = NodeStatus::Active;
    }

    #[tokio::test]
    async fn test_memory_registry_basic() {
        let registry = MemoryRegistry::new();

        let node_id = NodeId::new("node-1").unwrap();
        let mut info = NodeInfo::new(node_id.clone(), test_addr());
        info.status = NodeStatus::Active;

        registry.register_node(info).await.unwrap();

        let nodes = registry.list_nodes().await.unwrap();
        assert_eq!(nodes.len(), 1);
    }
}
