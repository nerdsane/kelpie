//! Registry trait and implementations
//!
//! TigerStyle: Explicit trait with single activation guarantee.

use crate::error::{RegistryError, RegistryResult};
use crate::heartbeat::{Heartbeat, HeartbeatConfig, HeartbeatTracker};
use crate::node::{NodeId, NodeInfo, NodeStatus};
use crate::placement::{ActorPlacement, PlacementContext, PlacementDecision, PlacementStrategy};
use async_trait::async_trait;
use kelpie_core::actor::ActorId;
use kelpie_core::io::{RngProvider, StdRngProvider, TimeProvider, WallClockTime};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// =============================================================================
// Clock Abstraction (for backward compatibility)
// =============================================================================

/// Clock trait for time operations
///
/// This is a simpler synchronous trait for code that doesn't need async sleep.
/// For full DST compatibility with sleep support, use `TimeProvider` instead.
#[deprecated(
    since = "0.2.0",
    note = "Use TimeProvider from kelpie_core::io instead"
)]
pub trait Clock: Send + Sync {
    /// Get the current time in milliseconds since Unix epoch
    fn now_ms(&self) -> u64;
}

/// System clock implementation using WallClockTime
#[deprecated(
    since = "0.2.0",
    note = "Use WallClockTime from kelpie_core::io instead"
)]
#[derive(Debug, Default)]
pub struct SystemClock;

#[allow(deprecated)]
impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        WallClockTime::new().now_ms()
    }
}

// =============================================================================
// Registry Trait
// =============================================================================

/// The registry trait for actor placement and node management
///
/// # Guarantees
/// - Single activation: An actor is active on at most one node at any time
/// - Linearizable operations: All operations appear to execute atomically
///
/// # TigerStyle
/// - All operations are explicit and async
/// - Errors are returned, never panics
#[async_trait]
pub trait Registry: Send + Sync {
    // =========================================================================
    // Node Management
    // =========================================================================

    /// Register a new node in the cluster
    ///
    /// # Arguments
    /// * `info` - The node's information
    ///
    /// # Returns
    /// Ok if the node was registered successfully.
    ///
    /// # Errors
    /// Returns error if a node with the same ID already exists.
    async fn register_node(&self, info: NodeInfo) -> RegistryResult<()>;

    /// Unregister a node from the cluster
    ///
    /// This should be called when a node is gracefully leaving.
    /// Active actors on this node should be migrated first.
    async fn unregister_node(&self, node_id: &NodeId) -> RegistryResult<()>;

    /// Get information about a node
    async fn get_node(&self, node_id: &NodeId) -> RegistryResult<Option<NodeInfo>>;

    /// List all registered nodes
    async fn list_nodes(&self) -> RegistryResult<Vec<NodeInfo>>;

    /// List nodes with a specific status
    async fn list_nodes_by_status(&self, status: NodeStatus) -> RegistryResult<Vec<NodeInfo>>;

    /// Update a node's status
    async fn update_node_status(&self, node_id: &NodeId, status: NodeStatus) -> RegistryResult<()>;

    /// Update node's heartbeat
    async fn receive_heartbeat(&self, heartbeat: Heartbeat) -> RegistryResult<()>;

    // =========================================================================
    // Actor Placement
    // =========================================================================

    /// Get the current placement of an actor
    ///
    /// Returns None if the actor is not currently active.
    async fn get_placement(&self, actor_id: &ActorId) -> RegistryResult<Option<ActorPlacement>>;

    /// Register an actor as active on a node
    ///
    /// # Single Activation Guarantee
    /// This operation will fail if the actor is already registered on a different node.
    /// Use `try_claim_actor` for compare-and-set semantics.
    async fn register_actor(&self, actor_id: ActorId, node_id: NodeId) -> RegistryResult<()>;

    /// Unregister an actor (mark as inactive)
    ///
    /// Should be called when an actor is deactivated.
    async fn unregister_actor(&self, actor_id: &ActorId) -> RegistryResult<()>;

    /// Try to claim an actor for a node (compare-and-set)
    ///
    /// This is the primary operation for ensuring single activation.
    /// Returns Ok with the placement if successful, or the existing placement if another
    /// node already has it.
    async fn try_claim_actor(
        &self,
        actor_id: ActorId,
        node_id: NodeId,
    ) -> RegistryResult<PlacementDecision>;

    /// Get all actors on a specific node
    async fn list_actors_on_node(&self, node_id: &NodeId) -> RegistryResult<Vec<ActorPlacement>>;

    /// Migrate an actor from one node to another
    ///
    /// # Arguments
    /// * `actor_id` - The actor to migrate
    /// * `from_node` - Expected current node (for CAS)
    /// * `to_node` - Target node
    ///
    /// # Errors
    /// Returns error if the actor is not on `from_node` or `to_node` has no capacity.
    async fn migrate_actor(
        &self,
        actor_id: &ActorId,
        from_node: &NodeId,
        to_node: &NodeId,
    ) -> RegistryResult<()>;

    // =========================================================================
    // Placement Strategy
    // =========================================================================

    /// Select a node for actor placement
    ///
    /// Uses the configured placement strategy to choose the best node.
    async fn select_node_for_placement(
        &self,
        context: PlacementContext,
    ) -> RegistryResult<PlacementDecision>;
}

/// In-memory registry implementation
///
/// Suitable for single-node deployment or testing.
/// All state is lost on restart.
pub struct MemoryRegistry {
    /// Node information
    nodes: RwLock<HashMap<NodeId, NodeInfo>>,
    /// Actor placements
    placements: RwLock<HashMap<ActorId, ActorPlacement>>,
    /// Heartbeat tracker
    heartbeat_tracker: RwLock<HeartbeatTracker>,
    /// Time provider (for DST compatibility)
    time: Arc<dyn TimeProvider>,
    /// RNG provider (for DST compatibility)
    rng: Arc<dyn RngProvider>,
    /// Round-robin index for placement strategy
    round_robin_index: std::sync::atomic::AtomicUsize,
}

/// Mock clock for testing (implements TimeProvider)
#[derive(Debug)]
pub struct MockClock {
    time_ms: RwLock<u64>,
}

impl MockClock {
    /// Create a new mock clock
    pub fn new(initial_ms: u64) -> Self {
        Self {
            time_ms: RwLock::new(initial_ms),
        }
    }

    /// Advance time by the given milliseconds
    pub async fn advance(&self, ms: u64) {
        let mut time = self.time_ms.write().await;
        *time = time.saturating_add(ms);
    }

    /// Set time to a specific value
    pub async fn set(&self, ms: u64) {
        let mut time = self.time_ms.write().await;
        *time = ms;
    }
}

#[async_trait]
impl TimeProvider for MockClock {
    fn now_ms(&self) -> u64 {
        // Use try_read for sync context, fallback to blocking
        self.time_ms.try_read().map(|t| *t).unwrap_or(0)
    }

    async fn sleep_ms(&self, ms: u64) {
        // In mock, just advance time
        self.advance(ms).await;
    }

    fn monotonic_ms(&self) -> u64 {
        self.now_ms()
    }
}

impl MemoryRegistry {
    /// Create a new in-memory registry with production I/O providers
    pub fn new() -> Self {
        Self::with_config(HeartbeatConfig::default())
    }

    /// Create with custom heartbeat config
    pub fn with_config(heartbeat_config: HeartbeatConfig) -> Self {
        Self {
            nodes: RwLock::new(HashMap::new()),
            placements: RwLock::new(HashMap::new()),
            heartbeat_tracker: RwLock::new(HeartbeatTracker::new(heartbeat_config)),
            time: Arc::new(WallClockTime::new()),
            rng: Arc::new(StdRngProvider::new()),
            round_robin_index: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Create with custom I/O providers (for DST)
    pub fn with_providers(time: Arc<dyn TimeProvider>, rng: Arc<dyn RngProvider>) -> Self {
        Self {
            nodes: RwLock::new(HashMap::new()),
            placements: RwLock::new(HashMap::new()),
            heartbeat_tracker: RwLock::new(HeartbeatTracker::new(HeartbeatConfig::default())),
            time,
            rng,
            round_robin_index: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Create with a mock clock for testing (convenience method)
    pub fn with_clock(clock: Arc<dyn TimeProvider>) -> Self {
        Self {
            nodes: RwLock::new(HashMap::new()),
            placements: RwLock::new(HashMap::new()),
            heartbeat_tracker: RwLock::new(HeartbeatTracker::new(HeartbeatConfig::default())),
            time: clock,
            rng: Arc::new(StdRngProvider::new()),
            round_robin_index: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Check all nodes for heartbeat timeouts
    ///
    /// Returns list of nodes that transitioned to failed state.
    pub async fn check_heartbeat_timeouts(&self) -> Vec<NodeId> {
        let now_ms = self.time.now_ms();
        let mut tracker = self.heartbeat_tracker.write().await;
        let changes = tracker.check_all_timeouts(now_ms);

        let mut failed_nodes = Vec::new();

        // Update node status for any changes
        for (node_id, _old_status, new_status) in changes {
            if new_status == NodeStatus::Failed {
                failed_nodes.push(node_id.clone());
            }

            // Update the node's stored status
            let mut nodes = self.nodes.write().await;
            if let Some(info) = nodes.get_mut(&node_id) {
                info.status = new_status;
            }
        }

        failed_nodes
    }

    /// Get actors that need to be migrated due to node failure
    pub async fn get_actors_to_migrate(&self, failed_node: &NodeId) -> Vec<ActorPlacement> {
        let placements = self.placements.read().await;
        placements
            .values()
            .filter(|p| &p.node_id == failed_node)
            .cloned()
            .collect()
    }

    /// Select node using least-loaded strategy
    async fn select_least_loaded(&self) -> Option<NodeId> {
        let nodes = self.nodes.read().await;
        nodes
            .values()
            .filter(|n| n.status.can_accept_actors() && n.has_capacity())
            .min_by_key(|n| n.actor_count)
            .map(|n| n.id.clone())
    }

    /// Select node using random strategy
    async fn select_random(&self) -> Option<NodeId> {
        let nodes = self.nodes.read().await;
        let available: Vec<_> = nodes
            .values()
            .filter(|n| n.status.can_accept_actors() && n.has_capacity())
            .collect();

        if available.is_empty() {
            None
        } else {
            // Use injected RNG provider for DST determinism
            let idx = self.rng.gen_range(0, available.len() as u64) as usize;
            Some(available[idx].id.clone())
        }
    }

    /// Select node using round-robin strategy
    async fn select_round_robin(&self) -> Option<NodeId> {
        let nodes = self.nodes.read().await;
        let mut available: Vec<_> = nodes
            .values()
            .filter(|n| n.status.can_accept_actors() && n.has_capacity())
            .collect();

        if available.is_empty() {
            return None;
        }

        // Sort by node_id to ensure stable ordering
        available.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));

        // Get and increment the round-robin index atomically
        let current_idx = self
            .round_robin_index
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let selected_idx = current_idx % available.len();

        Some(available[selected_idx].id.clone())
    }
}

impl Default for MemoryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Registry for MemoryRegistry {
    async fn register_node(&self, info: NodeInfo) -> RegistryResult<()> {
        let mut nodes = self.nodes.write().await;

        // Check if node already exists
        if nodes.contains_key(&info.id) {
            return Err(RegistryError::NodeAlreadyExists {
                node_id: info.id.to_string(),
            });
        }

        // Register with heartbeat tracker
        let mut tracker = self.heartbeat_tracker.write().await;
        tracker.register_node(info.id.clone(), self.time.now_ms());

        nodes.insert(info.id.clone(), info);
        Ok(())
    }

    async fn unregister_node(&self, node_id: &NodeId) -> RegistryResult<()> {
        let mut nodes = self.nodes.write().await;

        if nodes.remove(node_id).is_none() {
            return Err(RegistryError::node_not_found(node_id.as_str()));
        }

        // Remove from heartbeat tracker
        let mut tracker = self.heartbeat_tracker.write().await;
        tracker.unregister_node(node_id);

        Ok(())
    }

    async fn get_node(&self, node_id: &NodeId) -> RegistryResult<Option<NodeInfo>> {
        let nodes = self.nodes.read().await;
        Ok(nodes.get(node_id).cloned())
    }

    async fn list_nodes(&self) -> RegistryResult<Vec<NodeInfo>> {
        let nodes = self.nodes.read().await;
        Ok(nodes.values().cloned().collect())
    }

    async fn list_nodes_by_status(&self, status: NodeStatus) -> RegistryResult<Vec<NodeInfo>> {
        let nodes = self.nodes.read().await;
        Ok(nodes
            .values()
            .filter(|n| n.status == status)
            .cloned()
            .collect())
    }

    async fn update_node_status(&self, node_id: &NodeId, status: NodeStatus) -> RegistryResult<()> {
        let mut nodes = self.nodes.write().await;

        let info = nodes
            .get_mut(node_id)
            .ok_or_else(|| RegistryError::node_not_found(node_id.as_str()))?;

        info.set_status(status);
        Ok(())
    }

    async fn receive_heartbeat(&self, heartbeat: Heartbeat) -> RegistryResult<()> {
        let now_ms = self.time.now_ms();

        // Update heartbeat tracker
        let mut tracker = self.heartbeat_tracker.write().await;
        tracker.receive_heartbeat(heartbeat.clone(), now_ms)?;

        // Update node info
        let mut nodes = self.nodes.write().await;
        if let Some(info) = nodes.get_mut(&heartbeat.node_id) {
            info.update_heartbeat(now_ms);
            info.actor_count = heartbeat.actor_count;

            // If node was suspect, restore to active
            if info.status == NodeStatus::Suspect {
                info.status = NodeStatus::Active;
            }
        }

        Ok(())
    }

    async fn get_placement(&self, actor_id: &ActorId) -> RegistryResult<Option<ActorPlacement>> {
        let placements = self.placements.read().await;
        Ok(placements.get(actor_id).cloned())
    }

    async fn register_actor(&self, actor_id: ActorId, node_id: NodeId) -> RegistryResult<()> {
        let mut placements = self.placements.write().await;

        // Check for existing registration
        if let Some(existing) = placements.get(&actor_id) {
            if existing.node_id != node_id {
                return Err(RegistryError::actor_already_registered(
                    &actor_id,
                    existing.node_id.as_str(),
                ));
            }
            // Already registered on same node, no-op
            return Ok(());
        }

        // Create new placement
        let placement = ActorPlacement::new(actor_id.clone(), node_id.clone());
        placements.insert(actor_id, placement);

        // Update node's actor count
        let mut nodes = self.nodes.write().await;
        if let Some(info) = nodes.get_mut(&node_id) {
            info.increment_actor_count();
        }

        Ok(())
    }

    async fn unregister_actor(&self, actor_id: &ActorId) -> RegistryResult<()> {
        let mut placements = self.placements.write().await;

        if let Some(placement) = placements.remove(actor_id) {
            // Update node's actor count
            let mut nodes = self.nodes.write().await;
            if let Some(info) = nodes.get_mut(&placement.node_id) {
                info.decrement_actor_count();
            }
        }

        Ok(())
    }

    async fn try_claim_actor(
        &self,
        actor_id: ActorId,
        node_id: NodeId,
    ) -> RegistryResult<PlacementDecision> {
        let mut placements = self.placements.write().await;

        // Check for existing placement
        if let Some(existing) = placements.get(&actor_id) {
            if existing.node_id == node_id {
                // Already on this node
                return Ok(PlacementDecision::Existing(existing.clone()));
            } else {
                // On another node
                return Ok(PlacementDecision::Existing(existing.clone()));
            }
        }

        // Check node capacity
        let mut nodes = self.nodes.write().await;
        let node = nodes.get_mut(&node_id);

        match node {
            Some(info) if info.has_capacity() && info.status.can_accept_actors() => {
                // Claim the actor using the registry's clock for DST compatibility
                let now_ms = self.time.now_ms();
                let placement =
                    ActorPlacement::with_timestamp(actor_id.clone(), node_id.clone(), now_ms);
                placements.insert(actor_id, placement);
                info.increment_actor_count();
                Ok(PlacementDecision::New(node_id))
            }
            Some(_) => Ok(PlacementDecision::NoCapacity),
            None => Err(RegistryError::node_not_found(node_id.as_str())),
        }
    }

    async fn list_actors_on_node(&self, node_id: &NodeId) -> RegistryResult<Vec<ActorPlacement>> {
        let placements = self.placements.read().await;
        Ok(placements
            .values()
            .filter(|p| &p.node_id == node_id)
            .cloned()
            .collect())
    }

    async fn migrate_actor(
        &self,
        actor_id: &ActorId,
        from_node: &NodeId,
        to_node: &NodeId,
    ) -> RegistryResult<()> {
        let mut placements = self.placements.write().await;
        let mut nodes = self.nodes.write().await;

        // Verify current placement
        let placement = placements
            .get_mut(actor_id)
            .ok_or_else(|| RegistryError::actor_not_found(actor_id))?;

        if &placement.node_id != from_node {
            return Err(RegistryError::ActorAlreadyRegistered {
                actor_id: actor_id.qualified_name(),
                existing_node: placement.node_id.to_string(),
            });
        }

        // Verify target node has capacity
        let target = nodes
            .get_mut(to_node)
            .ok_or_else(|| RegistryError::node_not_found(to_node.as_str()))?;

        if !target.has_capacity() || !target.status.can_accept_actors() {
            return Err(RegistryError::Internal {
                message: format!("target node {} cannot accept actors", to_node),
            });
        }

        // Update placement
        let now_ms = self.time.now_ms();
        placement.migrate_to(to_node.clone(), now_ms);

        // Update node counts
        target.increment_actor_count();

        if let Some(source) = nodes.get_mut(from_node) {
            source.decrement_actor_count();
        }

        Ok(())
    }

    async fn select_node_for_placement(
        &self,
        context: PlacementContext,
    ) -> RegistryResult<PlacementDecision> {
        // First check if actor is already placed
        if let Some(placement) = self.get_placement(&context.actor_id).await? {
            return Ok(PlacementDecision::Existing(placement));
        }

        // Select node based on strategy
        let node_id = match context.strategy {
            PlacementStrategy::LeastLoaded => self.select_least_loaded().await,
            PlacementStrategy::Random => self.select_random().await,
            PlacementStrategy::Affinity => {
                // Try preferred node first
                if let Some(ref preferred) = context.preferred_node {
                    let nodes = self.nodes.read().await;
                    if let Some(info) = nodes.get(preferred) {
                        if info.has_capacity() && info.status.can_accept_actors() {
                            return Ok(PlacementDecision::New(preferred.clone()));
                        }
                    }
                }
                // Fall back to least loaded
                self.select_least_loaded().await
            }
            PlacementStrategy::RoundRobin => self.select_round_robin().await,
        };

        match node_id {
            Some(id) => Ok(PlacementDecision::New(id)),
            None => Ok(PlacementDecision::NoCapacity),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn test_addr(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
    }

    fn test_node_id(n: u32) -> NodeId {
        NodeId::new(format!("node-{}", n)).unwrap()
    }

    fn test_actor_id(n: u32) -> ActorId {
        ActorId::new("test", format!("actor-{}", n)).unwrap()
    }

    fn test_node_info(n: u32) -> NodeInfo {
        let mut info = NodeInfo::new(test_node_id(n), test_addr(8080 + n as u16));
        info.status = NodeStatus::Active;
        info
    }

    #[tokio::test]
    async fn test_register_node() {
        let registry = MemoryRegistry::new();

        let info = test_node_info(1);
        registry.register_node(info.clone()).await.unwrap();

        let retrieved = registry.get_node(&test_node_id(1)).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, info.id);
    }

    #[tokio::test]
    async fn test_register_node_duplicate() {
        let registry = MemoryRegistry::new();

        let info = test_node_info(1);
        registry.register_node(info.clone()).await.unwrap();

        let result = registry.register_node(info).await;
        assert!(matches!(
            result,
            Err(RegistryError::NodeAlreadyExists { .. })
        ));
    }

    #[tokio::test]
    async fn test_unregister_node() {
        let registry = MemoryRegistry::new();

        registry.register_node(test_node_info(1)).await.unwrap();
        registry.unregister_node(&test_node_id(1)).await.unwrap();

        let retrieved = registry.get_node(&test_node_id(1)).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_list_nodes() {
        let registry = MemoryRegistry::new();

        registry.register_node(test_node_info(1)).await.unwrap();
        registry.register_node(test_node_info(2)).await.unwrap();
        registry.register_node(test_node_info(3)).await.unwrap();

        let nodes = registry.list_nodes().await.unwrap();
        assert_eq!(nodes.len(), 3);
    }

    #[tokio::test]
    async fn test_register_actor() {
        let registry = MemoryRegistry::new();

        registry.register_node(test_node_info(1)).await.unwrap();
        registry
            .register_actor(test_actor_id(1), test_node_id(1))
            .await
            .unwrap();

        let placement = registry.get_placement(&test_actor_id(1)).await.unwrap();
        assert!(placement.is_some());
        assert_eq!(placement.unwrap().node_id, test_node_id(1));
    }

    #[tokio::test]
    async fn test_register_actor_conflict() {
        let registry = MemoryRegistry::new();

        registry.register_node(test_node_info(1)).await.unwrap();
        registry.register_node(test_node_info(2)).await.unwrap();

        registry
            .register_actor(test_actor_id(1), test_node_id(1))
            .await
            .unwrap();

        let result = registry
            .register_actor(test_actor_id(1), test_node_id(2))
            .await;
        assert!(matches!(
            result,
            Err(RegistryError::ActorAlreadyRegistered { .. })
        ));
    }

    #[tokio::test]
    async fn test_try_claim_actor_new() {
        let registry = MemoryRegistry::new();

        registry.register_node(test_node_info(1)).await.unwrap();

        let decision = registry
            .try_claim_actor(test_actor_id(1), test_node_id(1))
            .await
            .unwrap();

        assert!(matches!(decision, PlacementDecision::New(_)));
    }

    #[tokio::test]
    async fn test_try_claim_actor_existing() {
        let registry = MemoryRegistry::new();

        registry.register_node(test_node_info(1)).await.unwrap();
        registry
            .register_actor(test_actor_id(1), test_node_id(1))
            .await
            .unwrap();

        let decision = registry
            .try_claim_actor(test_actor_id(1), test_node_id(1))
            .await
            .unwrap();

        assert!(matches!(decision, PlacementDecision::Existing(_)));
    }

    #[tokio::test]
    async fn test_migrate_actor() {
        let registry = MemoryRegistry::new();

        registry.register_node(test_node_info(1)).await.unwrap();
        registry.register_node(test_node_info(2)).await.unwrap();

        registry
            .register_actor(test_actor_id(1), test_node_id(1))
            .await
            .unwrap();

        registry
            .migrate_actor(&test_actor_id(1), &test_node_id(1), &test_node_id(2))
            .await
            .unwrap();

        let placement = registry.get_placement(&test_actor_id(1)).await.unwrap();
        assert_eq!(placement.unwrap().node_id, test_node_id(2));
    }

    #[tokio::test]
    async fn test_select_node_least_loaded() {
        let registry = MemoryRegistry::new();

        // Node 1 with high load
        let mut info1 = test_node_info(1);
        info1.actor_count = 50;
        registry.register_node(info1).await.unwrap();

        // Node 2 with low load
        let mut info2 = test_node_info(2);
        info2.actor_count = 10;
        registry.register_node(info2).await.unwrap();

        let context = PlacementContext::new(test_actor_id(1));
        let decision = registry.select_node_for_placement(context).await.unwrap();

        match decision {
            PlacementDecision::New(node_id) => assert_eq!(node_id, test_node_id(2)),
            _ => panic!("expected New decision"),
        }
    }

    #[tokio::test]
    async fn test_list_actors_on_node() {
        let registry = MemoryRegistry::new();

        registry.register_node(test_node_info(1)).await.unwrap();
        registry.register_node(test_node_info(2)).await.unwrap();

        registry
            .register_actor(test_actor_id(1), test_node_id(1))
            .await
            .unwrap();
        registry
            .register_actor(test_actor_id(2), test_node_id(1))
            .await
            .unwrap();
        registry
            .register_actor(test_actor_id(3), test_node_id(2))
            .await
            .unwrap();

        let node1_actors = registry
            .list_actors_on_node(&test_node_id(1))
            .await
            .unwrap();
        assert_eq!(node1_actors.len(), 2);

        let node2_actors = registry
            .list_actors_on_node(&test_node_id(2))
            .await
            .unwrap();
        assert_eq!(node2_actors.len(), 1);
    }

    #[tokio::test]
    async fn test_heartbeat_timeout() {
        let clock = Arc::new(MockClock::new(1000));
        let registry = MemoryRegistry::with_clock(clock.clone());

        // Register node with timestamp matching the mock clock
        let mut info = NodeInfo::with_timestamp(test_node_id(1), test_addr(8081), 1000);
        info.status = NodeStatus::Active;
        registry.register_node(info).await.unwrap();

        // Send initial heartbeat
        let hb = Heartbeat::new(test_node_id(1), 1000, NodeStatus::Active, 0, 100, 1);
        registry.receive_heartbeat(hb).await.unwrap();

        // Advance time past failure timeout
        clock.advance(10000).await;

        let failed = registry.check_heartbeat_timeouts().await;
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0], test_node_id(1));

        // Verify node status changed
        let node = registry.get_node(&test_node_id(1)).await.unwrap().unwrap();
        assert_eq!(node.status, NodeStatus::Failed);
    }

    #[tokio::test]
    async fn test_select_node_round_robin() {
        let registry = MemoryRegistry::new();

        // Register 3 nodes
        registry.register_node(test_node_info(1)).await.unwrap();
        registry.register_node(test_node_info(2)).await.unwrap();
        registry.register_node(test_node_info(3)).await.unwrap();

        // Request round-robin placements - should cycle through nodes
        let mut selected_nodes = Vec::new();
        for i in 1..=6 {
            let context = PlacementContext::new(test_actor_id(i))
                .with_strategy(PlacementStrategy::RoundRobin);
            let decision = registry.select_node_for_placement(context).await.unwrap();

            match decision {
                PlacementDecision::New(node_id) => selected_nodes.push(node_id),
                _ => panic!("expected New decision"),
            }
        }

        // Nodes are sorted by id: node-1, node-2, node-3
        // First cycle
        assert_eq!(selected_nodes[0], test_node_id(1));
        assert_eq!(selected_nodes[1], test_node_id(2));
        assert_eq!(selected_nodes[2], test_node_id(3));
        // Second cycle (wraps around)
        assert_eq!(selected_nodes[3], test_node_id(1));
        assert_eq!(selected_nodes[4], test_node_id(2));
        assert_eq!(selected_nodes[5], test_node_id(3));
    }

    #[tokio::test]
    async fn test_select_node_affinity() {
        let registry = MemoryRegistry::new();

        // Register 2 nodes
        registry.register_node(test_node_info(1)).await.unwrap();
        registry.register_node(test_node_info(2)).await.unwrap();

        // Request placement with affinity to node-2
        let context = PlacementContext::new(test_actor_id(1)).with_preferred_node(test_node_id(2));
        let decision = registry.select_node_for_placement(context).await.unwrap();

        match decision {
            PlacementDecision::New(node_id) => assert_eq!(node_id, test_node_id(2)),
            _ => panic!("expected New decision"),
        }
    }

    #[tokio::test]
    async fn test_select_node_affinity_fallback() {
        let registry = MemoryRegistry::new();

        // Register node-1 only (node-2 doesn't exist)
        registry.register_node(test_node_info(1)).await.unwrap();

        // Request placement with affinity to non-existent node-2
        let context = PlacementContext::new(test_actor_id(1)).with_preferred_node(test_node_id(2));
        let decision = registry.select_node_for_placement(context).await.unwrap();

        // Should fall back to node-1 (least loaded)
        match decision {
            PlacementDecision::New(node_id) => assert_eq!(node_id, test_node_id(1)),
            _ => panic!("expected New decision"),
        }
    }

    #[tokio::test]
    async fn test_select_node_random() {
        let registry = MemoryRegistry::new();

        // Register 3 nodes
        registry.register_node(test_node_info(1)).await.unwrap();
        registry.register_node(test_node_info(2)).await.unwrap();
        registry.register_node(test_node_info(3)).await.unwrap();

        // Request random placement - should select one of the available nodes
        let context =
            PlacementContext::new(test_actor_id(1)).with_strategy(PlacementStrategy::Random);
        let decision = registry.select_node_for_placement(context).await.unwrap();

        match decision {
            PlacementDecision::New(node_id) => {
                // Should be one of our nodes
                assert!(
                    node_id == test_node_id(1)
                        || node_id == test_node_id(2)
                        || node_id == test_node_id(3)
                );
            }
            _ => panic!("expected New decision"),
        }
    }

    #[tokio::test]
    async fn test_select_node_no_capacity() {
        let registry = MemoryRegistry::new();

        // Register a node at capacity
        let mut info = test_node_info(1);
        info.actor_capacity = 100;
        info.actor_count = 100; // At capacity
        registry.register_node(info).await.unwrap();

        // Request placement - should return NoCapacity
        let context = PlacementContext::new(test_actor_id(1));
        let decision = registry.select_node_for_placement(context).await.unwrap();

        assert!(matches!(decision, PlacementDecision::NoCapacity));
    }
}
