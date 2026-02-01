//! DST tests for cluster membership using production TestableClusterMembership
//!
//! These tests verify TLA+ invariants against the PRODUCTION membership implementation
//! via `TestableClusterMembership`, as required by spec 077 (DST-1).
//!
//! TLA+ Invariants:
//! - NoSplitBrain: At most one node has a valid primary claim
//! - MembershipConsistency: Active nodes with same view number have same membership view
//! - JoinAtomicity: Node is either fully joined or not joined
//!
//! DST Requirements (DST-1):
//! - Tests use PRODUCTION code (TestableClusterMembership with MockClusterStorage)
//! - Tests use injected providers (TimeProvider)
//! - Logic IS the production implementation (not a mock)

use kelpie_core::io::TimeProvider;
use kelpie_dst::SimConfig;
use kelpie_registry::{
    ClusterStorageBackend, MockClusterStorage, NodeId, NodeState, TestableClusterMembership,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

// =============================================================================
// Test Helpers
// =============================================================================

fn test_node_id(n: u32) -> NodeId {
    NodeId::new(format!("node-{}", n)).unwrap()
}

/// Simulated clock for DST
///
/// TigerStyle: Explicit time control, deterministic advancement.
#[derive(Debug, Clone)]
struct SimClock {
    now_ms: Arc<AtomicU64>,
}

impl SimClock {
    fn new(initial_ms: u64) -> Self {
        Self {
            now_ms: Arc::new(AtomicU64::new(initial_ms)),
        }
    }

    fn get_now_ms(&self) -> u64 {
        self.now_ms.load(Ordering::SeqCst)
    }

    fn advance(&self, ms: u64) {
        self.now_ms.fetch_add(ms, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl TimeProvider for SimClock {
    fn now_ms(&self) -> u64 {
        self.get_now_ms()
    }

    async fn sleep_ms(&self, ms: u64) {
        self.advance(ms);
    }
}

// =============================================================================
// DST Cluster Harness
// =============================================================================

/// DST harness for testing cluster membership with production code
///
/// This wraps multiple `TestableClusterMembership` instances (one per node)
/// sharing the same `MockClusterStorage` backend.
struct DstCluster {
    /// Shared storage backend (simulates FDB)
    storage: Arc<MockClusterStorage>,
    /// Per-node membership managers (production code!)
    memberships: HashMap<NodeId, Arc<TestableClusterMembership<MockClusterStorage>>>,
    /// Shared clock for all nodes
    clock: Arc<SimClock>,
    /// Network partitions (bidirectional)
    partitions: HashSet<(NodeId, NodeId)>,
}

impl DstCluster {
    fn new(clock: Arc<SimClock>) -> Self {
        Self {
            storage: Arc::new(MockClusterStorage::new()),
            memberships: HashMap::new(),
            clock,
            partitions: HashSet::new(),
        }
    }

    /// Add a node to the cluster (creates membership manager)
    async fn add_node(&mut self, node_id: NodeId) {
        let membership = Arc::new(TestableClusterMembership::new(
            self.storage.clone(),
            node_id.clone(),
            self.clock.clone(),
        ));
        self.memberships.insert(node_id, membership);
    }

    /// Get membership manager for a node
    fn get(&self, node_id: &NodeId) -> Option<&Arc<TestableClusterMembership<MockClusterStorage>>> {
        self.memberships.get(node_id)
    }

    /// Join a node to the cluster
    async fn join_node(&self, node_id: &NodeId, rpc_addr: &str) -> Result<(), String> {
        let membership = self.get(node_id).ok_or("node not found")?;

        // Update reachability before join
        self.update_reachability(node_id).await;

        membership
            .join(rpc_addr.to_string())
            .await
            .map_err(|e| e.to_string())
    }

    /// Complete join for a node
    async fn complete_join(&self, node_id: &NodeId) -> Result<(), String> {
        let membership = self.get(node_id).ok_or("node not found")?;
        membership.complete_join().await.map_err(|e| e.to_string())
    }

    /// Create a network partition between two nodes
    fn partition(&mut self, a: &NodeId, b: &NodeId) {
        self.partitions.insert((a.clone(), b.clone()));
        self.partitions.insert((b.clone(), a.clone()));
    }

    /// Heal all network partitions
    fn heal_all_partitions(&mut self) {
        self.partitions.clear();
    }

    /// Check if two nodes can communicate
    fn can_communicate(&self, a: &NodeId, b: &NodeId) -> bool {
        !self.partitions.contains(&(a.clone(), b.clone()))
    }

    /// Update reachability for a node based on current partitions
    async fn update_reachability(&self, node_id: &NodeId) {
        let membership = match self.get(node_id) {
            Some(m) => m,
            None => return,
        };

        let mut reachable = HashSet::new();
        for other_id in self.memberships.keys() {
            if other_id != node_id && self.can_communicate(node_id, other_id) {
                reachable.insert(other_id.clone());
            }
        }
        membership.set_reachable_nodes(reachable).await;
    }

    /// Update reachability for all nodes
    async fn update_all_reachability(&self) {
        for node_id in self.memberships.keys().cloned().collect::<Vec<_>>() {
            self.update_reachability(&node_id).await;
        }
    }

    /// Count nodes with valid primary claims
    ///
    /// TLA+ NoSplitBrain: This should never be > 1
    async fn count_valid_primaries(&self) -> usize {
        let mut count = 0;
        for membership in self.memberships.values() {
            if membership.has_valid_primary_claim().await.unwrap_or(false) {
                count += 1;
            }
        }
        count
    }

    /// Check if a node has quorum
    #[allow(dead_code)]
    async fn has_quorum(&self, node_id: &NodeId) -> bool {
        let active_count = self.count_active_nodes().await;
        let reachable_count = self.count_reachable_active(node_id).await;
        // Strict majority: 2 * reachable > total
        2 * reachable_count > active_count
    }

    /// Count active nodes in the cluster
    #[allow(dead_code)]
    async fn count_active_nodes(&self) -> usize {
        let nodes = self.storage.list_nodes().await.unwrap();
        nodes
            .iter()
            .filter(|n| n.state == NodeState::Active)
            .count()
    }

    /// Count reachable active nodes from a given node's perspective
    #[allow(dead_code)]
    async fn count_reachable_active(&self, from: &NodeId) -> usize {
        let nodes = self.storage.list_nodes().await.unwrap();
        let mut count = 0;
        for node in nodes {
            if node.state == NodeState::Active {
                if &node.id == from {
                    count += 1; // Node can always reach itself
                } else if self.can_communicate(from, &node.id) {
                    count += 1;
                }
            }
        }
        count
    }

    /// Advance time for all nodes
    fn advance_time(&self, ms: u64) {
        self.clock.advance(ms);
    }
}

// =============================================================================
// TLA+ Invariant Checks
// =============================================================================

/// NoSplitBrain: At most one node has a valid primary claim
async fn check_no_split_brain(cluster: &DstCluster) -> Result<(), String> {
    let primary_count = cluster.count_valid_primaries().await;
    if primary_count > 1 {
        return Err(format!(
            "NoSplitBrain VIOLATED: {} valid primaries",
            primary_count
        ));
    }
    Ok(())
}

/// MembershipConsistency: Nodes with same view number have same membership
async fn check_membership_consistency(cluster: &DstCluster) -> Result<(), String> {
    let view = cluster
        .storage
        .read_membership_view()
        .await
        .map_err(|e| e.to_string())?;

    if let Some(v) = view {
        // All active nodes should agree on membership
        let nodes = cluster
            .storage
            .list_nodes()
            .await
            .map_err(|e| e.to_string())?;
        for node in &nodes {
            if node.state == NodeState::Active {
                // Node's local view should match stored view (or be older)
                if let Some(membership) = cluster.get(&node.id) {
                    let node_view = membership.membership_view().await;
                    if node_view.view_number > v.view_number {
                        return Err(format!(
                            "MembershipConsistency VIOLATED: node {} has view {} but storage has {}",
                            node.id, node_view.view_number, v.view_number
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

// =============================================================================
// DST Tests Using Production Code
// =============================================================================

/// Test DST-1: NoSplitBrain invariant holds with production code
///
/// Creates a 3-node cluster, causes partitions, verifies at most one primary
#[tokio::test]
async fn test_production_no_split_brain() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let clock = Arc::new(SimClock::new(1000));
    let mut cluster = DstCluster::new(clock.clone());

    // Create 3-node cluster
    let node1 = test_node_id(1);
    let node2 = test_node_id(2);
    let node3 = test_node_id(3);

    cluster.add_node(node1.clone()).await;
    cluster.add_node(node2.clone()).await;
    cluster.add_node(node3.clone()).await;

    // Node 1 joins first (becomes primary)
    cluster.join_node(&node1, "127.0.0.1:8080").await.unwrap();
    check_no_split_brain(&cluster).await.unwrap();

    // Nodes 2 and 3 join
    cluster.join_node(&node2, "127.0.0.1:8081").await.unwrap();
    cluster.complete_join(&node2).await.unwrap();
    cluster.join_node(&node3, "127.0.0.1:8082").await.unwrap();
    cluster.complete_join(&node3).await.unwrap();

    // Verify invariant holds
    check_no_split_brain(&cluster).await.unwrap();

    // Create partition between node1 and nodes 2,3
    cluster.partition(&node1, &node2);
    cluster.partition(&node1, &node3);
    cluster.update_all_reachability().await;

    // Node 1 loses quorum, should step down
    {
        let m1 = cluster.get(&node1).unwrap();
        m1.check_quorum_and_maybe_step_down().await.unwrap();
    }

    // Verify invariant still holds
    check_no_split_brain(&cluster).await.unwrap();

    // Node 2 can now try to become primary (has quorum with node 3)
    {
        let m2 = cluster.get(&node2).unwrap();
        let result = m2.try_become_primary().await.unwrap();
        // Should succeed since node1 is no longer valid primary
        assert!(
            result.is_some(),
            "node2 should become primary after node1 steps down"
        );
    }

    // Verify only one primary
    check_no_split_brain(&cluster).await.unwrap();

    // Heal partition
    cluster.heal_all_partitions();
    cluster.update_all_reachability().await;

    // Node 1 tries to become primary - should fail (node2 is valid)
    {
        let m1 = cluster.get(&node1).unwrap();
        let result = m1.try_become_primary().await.unwrap();
        assert!(
            result.is_none(),
            "node1 should not become primary when node2 is valid"
        );
    }

    // Final invariant check
    check_no_split_brain(&cluster).await.unwrap();
}

/// Test DST-1: Primary election requires quorum
#[tokio::test]
async fn test_production_primary_election_requires_quorum() {
    let clock = Arc::new(SimClock::new(1000));
    let mut cluster = DstCluster::new(clock.clone());

    // Create 5-node cluster
    let nodes: Vec<_> = (1..=5).map(test_node_id).collect();
    for node in &nodes {
        cluster.add_node(node.clone()).await;
    }

    // Node 1 joins first (becomes primary)
    cluster
        .join_node(&nodes[0], "127.0.0.1:8080")
        .await
        .unwrap();

    // All other nodes join
    for (i, node) in nodes.iter().enumerate().skip(1) {
        cluster
            .join_node(node, &format!("127.0.0.1:808{}", i))
            .await
            .unwrap();
        cluster.complete_join(node).await.unwrap();
    }

    // Partition nodes 3,4,5 from nodes 1,2
    // 1,2 have minority (2/5), 3,4,5 have majority (3/5)
    for i in 2..5 {
        for j in 0..2 {
            cluster.partition(&nodes[i], &nodes[j]);
        }
    }
    cluster.update_all_reachability().await;

    // Node 1 loses quorum, should step down
    let m1 = cluster.get(&nodes[0]).unwrap();
    let has_quorum = m1.check_quorum_and_maybe_step_down().await.unwrap();
    assert!(!has_quorum, "node1 should have lost quorum");
    assert!(!m1.is_primary().await, "node1 should have stepped down");

    // Node 3 has quorum and can become primary
    let m3 = cluster.get(&nodes[2]).unwrap();
    let result = m3.try_become_primary().await.unwrap();
    assert!(
        result.is_some(),
        "node3 should become primary with majority"
    );

    // Verify invariant
    check_no_split_brain(&cluster).await.unwrap();
}

/// Test DST-1: Primary stepdown on quorum loss
#[tokio::test]
async fn test_production_primary_stepdown_on_quorum_loss() {
    let clock = Arc::new(SimClock::new(1000));
    let mut cluster = DstCluster::new(clock.clone());

    let node1 = test_node_id(1);
    let node2 = test_node_id(2);
    let node3 = test_node_id(3);

    cluster.add_node(node1.clone()).await;
    cluster.add_node(node2.clone()).await;
    cluster.add_node(node3.clone()).await;

    // Build cluster
    cluster.join_node(&node1, "127.0.0.1:8080").await.unwrap();
    cluster.join_node(&node2, "127.0.0.1:8081").await.unwrap();
    cluster.complete_join(&node2).await.unwrap();
    cluster.join_node(&node3, "127.0.0.1:8082").await.unwrap();
    cluster.complete_join(&node3).await.unwrap();

    // Node 1 is primary
    {
        let m1 = cluster.get(&node1).unwrap();
        assert!(m1.is_primary().await);
    }

    // Partition node1 from everyone
    cluster.partition(&node1, &node2);
    cluster.partition(&node1, &node3);
    cluster.update_all_reachability().await;

    // Node 1 checks quorum and steps down
    {
        let m1 = cluster.get(&node1).unwrap();
        let has_quorum = m1.check_quorum_and_maybe_step_down().await.unwrap();
        assert!(!has_quorum);
        assert!(!m1.is_primary().await);
    }

    // Verify invariant
    check_no_split_brain(&cluster).await.unwrap();
}

/// Test DST-1: Heartbeat failure detection using production code
#[tokio::test]
async fn test_production_heartbeat_failure_detection() {
    let clock = Arc::new(SimClock::new(1000));
    let mut cluster = DstCluster::new(clock.clone());

    let node1 = test_node_id(1);
    let node2 = test_node_id(2);

    cluster.add_node(node1.clone()).await;
    cluster.add_node(node2.clone()).await;

    // Build cluster
    cluster.join_node(&node1, "127.0.0.1:8080").await.unwrap();
    cluster.join_node(&node2, "127.0.0.1:8081").await.unwrap();
    cluster.complete_join(&node2).await.unwrap();

    // Both nodes are active
    let m1 = cluster.get(&node1).unwrap();
    let m2 = cluster.get(&node2).unwrap();

    assert_eq!(m1.local_state().await, NodeState::Active);
    assert_eq!(m2.local_state().await, NodeState::Active);

    // Node 2 sends heartbeat
    m2.send_heartbeat().await.unwrap();

    // Advance time beyond heartbeat timeout
    cluster.advance_time(10_000); // 10 seconds

    // Node 1 detects node 2 as failed
    let detected = m1.detect_failure(&node2, 5_000).await.unwrap();
    assert!(detected, "node2 should be detected as failed");

    // Verify node 2 is marked failed
    let node2_info = cluster.storage.get_node(&node2).await.unwrap().unwrap();
    assert_eq!(node2_info.state, NodeState::Failed);
}

/// Test DST-1: Partition heal resolves conflict
#[tokio::test]
async fn test_production_partition_heal_resolves_conflict() {
    let clock = Arc::new(SimClock::new(1000));
    let mut cluster = DstCluster::new(clock.clone());

    let node1 = test_node_id(1);
    let node2 = test_node_id(2);
    let node3 = test_node_id(3);
    let node4 = test_node_id(4);
    let node5 = test_node_id(5);

    // Create 5-node cluster
    for node in [&node1, &node2, &node3, &node4, &node5] {
        cluster.add_node(node.clone()).await;
    }

    // Build cluster with node1 as primary
    cluster.join_node(&node1, "127.0.0.1:8080").await.unwrap();
    for (i, node) in [&node2, &node3, &node4, &node5].iter().enumerate() {
        cluster
            .join_node(node, &format!("127.0.0.1:808{}", i + 1))
            .await
            .unwrap();
        cluster.complete_join(node).await.unwrap();
    }

    // Create partition: {1,2,3} vs {4,5}
    for i in [&node1, &node2, &node3] {
        for j in [&node4, &node5] {
            cluster.partition(i, j);
        }
    }
    cluster.update_all_reachability().await;

    // {1,2,3} have majority, node1 stays primary
    let m1 = cluster.get(&node1).unwrap();
    assert!(m1.has_valid_primary_claim().await.unwrap());

    // {4,5} don't have majority, cannot elect primary
    let m4 = cluster.get(&node4).unwrap();
    let result = m4.try_become_primary().await.unwrap();
    assert!(result.is_none(), "minority partition cannot elect primary");

    // Heal partition
    cluster.heal_all_partitions();
    cluster.update_all_reachability().await;

    // Sync views
    let view = cluster
        .storage
        .read_membership_view()
        .await
        .unwrap()
        .unwrap();
    for node in [&node1, &node2, &node3, &node4, &node5] {
        let m = cluster.get(node).unwrap();
        m.sync_views(&view).await.unwrap();
    }

    // Verify single primary
    check_no_split_brain(&cluster).await.unwrap();
    check_membership_consistency(&cluster).await.unwrap();
}

/// Test DST-1: Determinism - same seed produces same results
#[tokio::test]
async fn test_production_determinism() {
    // Run the same test twice with same seed
    async fn run_test(seed: u64) -> Vec<bool> {
        let clock = Arc::new(SimClock::new(seed));
        let mut cluster = DstCluster::new(clock.clone());

        let nodes: Vec<_> = (1..=3).map(test_node_id).collect();
        for node in &nodes {
            cluster.add_node(node.clone()).await;
        }

        cluster
            .join_node(&nodes[0], "127.0.0.1:8080")
            .await
            .unwrap();
        for node in nodes.iter().skip(1) {
            cluster.join_node(node, "127.0.0.1:8081").await.unwrap();
            cluster.complete_join(node).await.unwrap();
        }

        let mut results = Vec::new();
        for node in &nodes {
            results.push(cluster.get(node).unwrap().is_primary().await);
        }
        results
    }

    let seed = 42;
    let results1 = run_test(seed).await;
    let results2 = run_test(seed).await;

    assert_eq!(results1, results2, "same seed must produce same results");
}

/// Test DST-1: Actor migration on node failure (FR-7)
#[tokio::test]
async fn test_production_actor_migration_on_node_failure() {
    let clock = Arc::new(SimClock::new(1000));
    let mut cluster = DstCluster::new(clock.clone());

    let node1 = test_node_id(1);
    let node2 = test_node_id(2);
    let node3 = test_node_id(3);

    cluster.add_node(node1.clone()).await;
    cluster.add_node(node2.clone()).await;
    cluster.add_node(node3.clone()).await;

    // Build cluster
    cluster.join_node(&node1, "127.0.0.1:8080").await.unwrap();
    cluster.join_node(&node2, "127.0.0.1:8081").await.unwrap();
    cluster.complete_join(&node2).await.unwrap();
    cluster.join_node(&node3, "127.0.0.1:8082").await.unwrap();
    cluster.complete_join(&node3).await.unwrap();

    // Node 1 is primary
    let m1 = cluster.get(&node1).unwrap();
    assert!(m1.is_primary().await);

    // Simulate node 2 failing with actors
    let actor_ids = vec!["actor-1".to_string(), "actor-2".to_string()];
    m1.handle_node_failure(&node2, actor_ids.clone())
        .await
        .unwrap();

    // Verify node 2 is marked failed
    let node2_info = cluster.storage.get_node(&node2).await.unwrap().unwrap();
    assert_eq!(node2_info.state, NodeState::Failed);

    // Verify actors are queued for migration
    let queue = m1.get_migration_queue().await.unwrap();
    assert_eq!(queue.len(), 2);
    assert!(queue.candidates.iter().any(|c| c.actor_id == "actor-1"));
    assert!(queue.candidates.iter().any(|c| c.actor_id == "actor-2"));

    // Process migration queue
    let results = m1
        .process_migration_queue(|actor_id| {
            // Select node 3 for all migrations
            if actor_id.starts_with("actor-") {
                Some(node3.clone())
            } else {
                None
            }
        })
        .await
        .unwrap();

    // Verify migrations succeeded
    assert_eq!(results.len(), 2);
    for result in &results {
        assert!(result.is_success());
    }

    // Queue should be empty now
    let queue = m1.get_migration_queue().await.unwrap();
    assert!(queue.is_empty());

    // Verify invariants
    check_no_split_brain(&cluster).await.unwrap();
}

/// Test DST-1: State transitions match TLA+ spec
#[tokio::test]
async fn test_production_state_transitions_match_tla() {
    let clock = Arc::new(SimClock::new(1000));
    let mut cluster = DstCluster::new(clock.clone());

    let node1 = test_node_id(1);
    cluster.add_node(node1.clone()).await;

    let m1 = cluster.get(&node1).unwrap();

    // Initial state: Left
    assert_eq!(m1.local_state().await, NodeState::Left);

    // Join as first node: Left -> Active (first node skips Joining)
    cluster.join_node(&node1, "127.0.0.1:8080").await.unwrap();
    assert_eq!(m1.local_state().await, NodeState::Active);
    assert!(m1.is_primary().await);

    // Leave: Active -> Leaving
    m1.leave().await.unwrap();
    assert_eq!(m1.local_state().await, NodeState::Leaving);
    assert!(!m1.is_primary().await);

    // Complete leave: Leaving -> Left
    m1.complete_leave().await.unwrap();
    assert_eq!(m1.local_state().await, NodeState::Left);
}

/// Test: Second node joins as Joining, not Active
#[tokio::test]
async fn test_production_second_node_joins_as_joining() {
    let clock = Arc::new(SimClock::new(1000));
    let mut cluster = DstCluster::new(clock.clone());

    let node1 = test_node_id(1);
    let node2 = test_node_id(2);

    cluster.add_node(node1.clone()).await;
    cluster.add_node(node2.clone()).await;

    // Node 1 joins first
    cluster.join_node(&node1, "127.0.0.1:8080").await.unwrap();
    let m1 = cluster.get(&node1).unwrap();
    assert_eq!(m1.local_state().await, NodeState::Active);

    // Node 2 joins second - should be Joining
    cluster.join_node(&node2, "127.0.0.1:8081").await.unwrap();
    let m2 = cluster.get(&node2).unwrap();
    assert_eq!(m2.local_state().await, NodeState::Joining);

    // Complete join - now Active
    cluster.complete_join(&node2).await.unwrap();
    assert_eq!(m2.local_state().await, NodeState::Active);
}

/// Test: Node recovery from Failed state
#[tokio::test]
async fn test_production_node_recover() {
    let clock = Arc::new(SimClock::new(1000));
    let mut cluster = DstCluster::new(clock.clone());

    let node1 = test_node_id(1);
    let node2 = test_node_id(2);

    cluster.add_node(node1.clone()).await;
    cluster.add_node(node2.clone()).await;

    // Build cluster
    cluster.join_node(&node1, "127.0.0.1:8080").await.unwrap();
    cluster.join_node(&node2, "127.0.0.1:8081").await.unwrap();
    cluster.complete_join(&node2).await.unwrap();

    // Mark node 2 as failed
    let m1 = cluster.get(&node1).unwrap();
    m1.mark_node_failed(&node2).await.unwrap();

    // Verify failed
    let node2_info = cluster.storage.get_node(&node2).await.unwrap().unwrap();
    assert_eq!(node2_info.state, NodeState::Failed);

    // Recover node 2
    m1.node_recover(&node2).await.unwrap();

    // Verify recovered to Left
    let node2_info = cluster.storage.get_node(&node2).await.unwrap().unwrap();
    assert_eq!(node2_info.state, NodeState::Left);
}

/// Stress test: Many partition cycles
#[tokio::test]
async fn test_production_stress_partition_cycles() {
    let config = SimConfig::from_env_or_random();
    println!("DST stress test seed: {}", config.seed);

    let clock = Arc::new(SimClock::new(1000));
    let mut cluster = DstCluster::new(clock.clone());

    let nodes: Vec<_> = (1..=5).map(test_node_id).collect();
    for node in &nodes {
        cluster.add_node(node.clone()).await;
    }

    // Build cluster
    cluster
        .join_node(&nodes[0], "127.0.0.1:8080")
        .await
        .unwrap();
    for (i, node) in nodes.iter().enumerate().skip(1) {
        cluster
            .join_node(node, &format!("127.0.0.1:808{}", i))
            .await
            .unwrap();
        cluster.complete_join(node).await.unwrap();
    }

    // Run 20 partition cycles
    for cycle in 0..20 {
        // Create random partition
        let partition_point = (cycle % 4) + 1;
        for i in 0..partition_point {
            for j in partition_point..5 {
                cluster.partition(&nodes[i], &nodes[j]);
            }
        }
        cluster.update_all_reachability().await;

        // Let nodes react
        for node in &nodes {
            let m = cluster.get(node).unwrap();
            let _ = m.check_quorum_and_maybe_step_down().await;
        }

        // Try to elect new primary if needed
        for node in &nodes {
            let m = cluster.get(node).unwrap();
            if !m.is_primary().await && m.local_state().await == NodeState::Active {
                let _ = m.try_become_primary().await;
            }
        }

        // Verify invariant after each cycle
        check_no_split_brain(&cluster)
            .await
            .unwrap_or_else(|e| panic!("Cycle {}: {}", cycle, e));

        // Heal partitions
        cluster.heal_all_partitions();
        cluster.update_all_reachability().await;
    }

    // Final verification
    check_no_split_brain(&cluster).await.unwrap();
}
