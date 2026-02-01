//! Kelpie Cluster
//!
//! Cluster coordination for Kelpie.
//!
//! # Overview
//!
//! The cluster module provides:
//! - Node lifecycle management (join/leave)
//! - Cluster membership tracking
//! - Inter-node RPC communication
//! - Actor migration on node failure
//! - Heartbeat-based failure detection
//!
//! # TigerStyle
//! - Explicit cluster state transitions
//! - Bounded heartbeat intervals
//! - Observable migration state
//!
//! # Example
//!
//! ```rust,ignore
//! use kelpie_cluster::{Cluster, ClusterConfig};
//! use kelpie_registry::{MemoryRegistry, NodeId, NodeInfo};
//!
//! // Create cluster configuration
//! let config = ClusterConfig::single_node("127.0.0.1:9000".parse()?);
//!
//! // Create registry and transport
//! let registry = Arc::new(MemoryRegistry::new());
//! let transport = Arc::new(MemoryTransport::new(node_id, addr));
//!
//! // Create and start cluster
//! let cluster = Cluster::new(node, config, registry, transport);
//! cluster.start().await?;
//! ```

mod cluster;
mod config;
mod error;
mod handler;
mod migration;
mod rpc;

pub use cluster::{Cluster, ClusterState};
pub use config::{
    ClusterConfig, BOOTSTRAP_RETRY_COUNT_MAX, BOOTSTRAP_RETRY_INTERVAL_MS,
    MIGRATION_BATCH_SIZE_DEFAULT,
};
pub use error::{ClusterError, ClusterResult};
pub use handler::{ActorInvoker, ClusterRpcHandler, MigrationReceiver};
pub use migration::{plan_migrations, MigrationCoordinator, MigrationInfo, MigrationState};
pub use rpc::{MemoryTransport, RequestId, RpcHandler, RpcMessage, RpcTransport, TcpTransport};

#[cfg(test)]
mod tests {
    use super::*;
    use kelpie_core::TokioRuntime;
    use kelpie_registry::{MemoryRegistry, NodeId, NodeInfo, NodeStatus};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Arc;

    fn test_addr() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 9000)
    }

    fn test_runtime() -> TokioRuntime {
        TokioRuntime
    }

    #[test]
    fn test_cluster_module_compiles() {
        // Verify public types are accessible
        let _state = ClusterState::Stopped;
        let _config = ClusterConfig::default();
    }

    #[tokio::test]
    async fn test_cluster_basic() {
        let node_id = NodeId::new("test-node").unwrap();
        let addr = test_addr();
        let runtime = test_runtime();
        let mut node = NodeInfo::with_timestamp(node_id.clone(), addr, 1000);
        node.status = NodeStatus::Active;

        let config = ClusterConfig::single_node(addr);
        let registry = Arc::new(MemoryRegistry::new());
        let transport = Arc::new(MemoryTransport::new(node_id, addr, runtime.clone()));

        let cluster = Cluster::new(node, config, registry, transport, runtime);
        assert_eq!(cluster.state().await, ClusterState::Stopped);
    }
}
