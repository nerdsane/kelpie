//! Cluster Storage Backend Abstraction
//!
//! Provides a trait-based abstraction for cluster state persistence,
//! allowing the use of FDB in production and mock storage in DST tests.
//!
//! TigerStyle: Explicit trait bounds, explicit error handling.

use crate::cluster_types::{ClusterNodeInfo, MigrationQueue};
use crate::error::RegistryResult;
use crate::membership::{MembershipView, PrimaryInfo};
use crate::node::NodeId;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// =============================================================================
// ClusterStorageBackend Trait
// =============================================================================

/// Backend trait for cluster state persistence
///
/// This trait abstracts the storage operations needed by `ClusterMembership`,
/// allowing different implementations for production (FDB) and testing (Mock).
///
/// TigerStyle: Each method has explicit error handling, no silent failures.
#[async_trait]
pub trait ClusterStorageBackend: Send + Sync {
    // =========================================================================
    // Node Operations
    // =========================================================================

    /// Get a cluster node by ID
    async fn get_node(&self, node_id: &NodeId) -> RegistryResult<Option<ClusterNodeInfo>>;

    /// Write/update a cluster node
    async fn write_node(&self, info: &ClusterNodeInfo) -> RegistryResult<()>;

    /// List all cluster nodes
    async fn list_nodes(&self) -> RegistryResult<Vec<ClusterNodeInfo>>;

    // =========================================================================
    // Membership View Operations
    // =========================================================================

    /// Read the current membership view
    async fn read_membership_view(&self) -> RegistryResult<Option<MembershipView>>;

    /// Write/update the membership view
    async fn write_membership_view(&self, view: &MembershipView) -> RegistryResult<()>;

    // =========================================================================
    // Primary Operations
    // =========================================================================

    /// Read the current primary info
    async fn read_primary(&self) -> RegistryResult<Option<PrimaryInfo>>;

    /// Write/update the primary info
    async fn write_primary(&self, primary: &PrimaryInfo) -> RegistryResult<()>;

    /// Clear the primary (step down)
    async fn clear_primary(&self) -> RegistryResult<()>;

    /// Read the current primary term
    async fn read_primary_term(&self) -> RegistryResult<Option<u64>>;

    /// Write/update the primary term
    async fn write_primary_term(&self, term: u64) -> RegistryResult<()>;

    // =========================================================================
    // Migration Queue Operations
    // =========================================================================

    /// Read the migration queue
    async fn read_migration_queue(&self) -> RegistryResult<Option<MigrationQueue>>;

    /// Write/update the migration queue
    async fn write_migration_queue(&self, queue: &MigrationQueue) -> RegistryResult<()>;
}

// =============================================================================
// MockClusterStorage (for DST)
// =============================================================================

/// In-memory cluster storage for DST testing
///
/// Implements `ClusterStorageBackend` using in-memory HashMaps,
/// allowing `ClusterMembership` to be tested without FDB.
///
/// TigerStyle: All state changes are explicit, deterministic ordering.
#[derive(Debug, Clone)]
pub struct MockClusterStorage {
    /// Node storage
    nodes: Arc<RwLock<HashMap<NodeId, ClusterNodeInfo>>>,
    /// Membership view
    membership_view: Arc<RwLock<Option<MembershipView>>>,
    /// Primary info
    primary: Arc<RwLock<Option<PrimaryInfo>>>,
    /// Primary term counter
    primary_term: Arc<RwLock<Option<u64>>>,
    /// Migration queue
    migration_queue: Arc<RwLock<Option<MigrationQueue>>>,
}

impl MockClusterStorage {
    /// Create new empty mock storage
    pub fn new() -> Self {
        Self {
            nodes: Arc::new(RwLock::new(HashMap::new())),
            membership_view: Arc::new(RwLock::new(None)),
            primary: Arc::new(RwLock::new(None)),
            primary_term: Arc::new(RwLock::new(None)),
            migration_queue: Arc::new(RwLock::new(None)),
        }
    }

    /// Get all node IDs (for testing)
    pub async fn node_ids(&self) -> Vec<NodeId> {
        let nodes = self.nodes.read().await;
        nodes.keys().cloned().collect()
    }

    /// Clear all data (for test reset)
    pub async fn clear(&self) {
        self.nodes.write().await.clear();
        *self.membership_view.write().await = None;
        *self.primary.write().await = None;
        *self.primary_term.write().await = None;
        *self.migration_queue.write().await = None;
    }
}

impl Default for MockClusterStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ClusterStorageBackend for MockClusterStorage {
    async fn get_node(&self, node_id: &NodeId) -> RegistryResult<Option<ClusterNodeInfo>> {
        let nodes = self.nodes.read().await;
        Ok(nodes.get(node_id).cloned())
    }

    async fn write_node(&self, info: &ClusterNodeInfo) -> RegistryResult<()> {
        let mut nodes = self.nodes.write().await;
        nodes.insert(info.id.clone(), info.clone());
        Ok(())
    }

    async fn list_nodes(&self) -> RegistryResult<Vec<ClusterNodeInfo>> {
        let nodes = self.nodes.read().await;
        // Return in deterministic order (sorted by node_id) for DST reproducibility
        let mut result: Vec<_> = nodes.values().cloned().collect();
        result.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        Ok(result)
    }

    async fn read_membership_view(&self) -> RegistryResult<Option<MembershipView>> {
        let view = self.membership_view.read().await;
        Ok(view.clone())
    }

    async fn write_membership_view(&self, view: &MembershipView) -> RegistryResult<()> {
        *self.membership_view.write().await = Some(view.clone());
        Ok(())
    }

    async fn read_primary(&self) -> RegistryResult<Option<PrimaryInfo>> {
        let primary = self.primary.read().await;
        Ok(primary.clone())
    }

    async fn write_primary(&self, primary: &PrimaryInfo) -> RegistryResult<()> {
        *self.primary.write().await = Some(primary.clone());
        Ok(())
    }

    async fn clear_primary(&self) -> RegistryResult<()> {
        *self.primary.write().await = None;
        Ok(())
    }

    async fn read_primary_term(&self) -> RegistryResult<Option<u64>> {
        let term = self.primary_term.read().await;
        Ok(*term)
    }

    async fn write_primary_term(&self, term: u64) -> RegistryResult<()> {
        *self.primary_term.write().await = Some(term);
        Ok(())
    }

    async fn read_migration_queue(&self) -> RegistryResult<Option<MigrationQueue>> {
        let queue = self.migration_queue.read().await;
        Ok(queue.clone())
    }

    async fn write_migration_queue(&self, queue: &MigrationQueue) -> RegistryResult<()> {
        *self.migration_queue.write().await = Some(queue.clone());
        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn test_node_id(n: u32) -> NodeId {
        NodeId::new(format!("node-{}", n)).unwrap()
    }

    #[tokio::test]
    async fn test_mock_storage_node_operations() {
        let storage = MockClusterStorage::new();

        // Initially empty
        let nodes = storage.list_nodes().await.unwrap();
        assert!(nodes.is_empty());

        // Write a node
        let node_id = test_node_id(1);
        let info = ClusterNodeInfo::new(node_id.clone(), "127.0.0.1:8080".to_string(), 1000);
        storage.write_node(&info).await.unwrap();

        // Read it back
        let retrieved = storage.get_node(&node_id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, node_id);

        // List nodes
        let nodes = storage.list_nodes().await.unwrap();
        assert_eq!(nodes.len(), 1);
    }

    #[tokio::test]
    async fn test_mock_storage_membership_view() {
        let storage = MockClusterStorage::new();

        // Initially empty
        let view = storage.read_membership_view().await.unwrap();
        assert!(view.is_none());

        // Write a view
        let mut active_nodes = HashSet::new();
        active_nodes.insert(test_node_id(1));
        active_nodes.insert(test_node_id(2));
        let view = MembershipView::new(active_nodes, 1, 1000);
        storage.write_membership_view(&view).await.unwrap();

        // Read it back
        let retrieved = storage.read_membership_view().await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().view_number, 1);
    }

    #[tokio::test]
    async fn test_mock_storage_primary() {
        let storage = MockClusterStorage::new();

        // Initially empty
        let primary = storage.read_primary().await.unwrap();
        assert!(primary.is_none());

        // Write primary
        let primary = PrimaryInfo::new(test_node_id(1), 1, 1000);
        storage.write_primary(&primary).await.unwrap();

        // Read it back
        let retrieved = storage.read_primary().await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().term, 1);

        // Clear primary
        storage.clear_primary().await.unwrap();
        let cleared = storage.read_primary().await.unwrap();
        assert!(cleared.is_none());
    }

    #[tokio::test]
    async fn test_mock_storage_deterministic_ordering() {
        let storage = MockClusterStorage::new();

        // Add nodes in non-sorted order
        for id in [3, 1, 5, 2, 4] {
            let node_id = test_node_id(id);
            let info = ClusterNodeInfo::new(node_id, "127.0.0.1:8080".to_string(), 1000);
            storage.write_node(&info).await.unwrap();
        }

        // List should return in sorted order
        let nodes = storage.list_nodes().await.unwrap();
        let ids: Vec<_> = nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["node-1", "node-2", "node-3", "node-4", "node-5"]);
    }
}
