//! DST tests for network partition tolerance
//!
//! TigerStyle: Deterministic testing of CP semantics - minority partitions
//! become unavailable, majority partitions continue serving, no split-brain.
//!
//! Tests verify ADR-004 requirements:
//! - "Minority partitions fail operations (unavailable)"
//! - "Majority partition continues serving"
//! - "Asymmetric behavior (not eventual consistency)"

use bytes::Bytes;
use kelpie_cluster::ClusterError;
use kelpie_core::actor::ActorId;
use kelpie_core::Error as CoreError;
use kelpie_dst::{FaultConfig, FaultType, SimConfig, Simulation};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// =============================================================================
// Error Conversion Helper
// =============================================================================

fn to_core_error(e: ClusterError) -> CoreError {
    CoreError::Internal {
        message: e.to_string(),
    }
}

// =============================================================================
// Simulated Cluster Node for Partition Testing
// =============================================================================

/// A simulated cluster node that tracks quorum
///
/// This is a simplified model for testing partition behavior.
/// Production quorum checking will be via FDB transactions.
#[derive(Debug, Clone)]
struct SimClusterNode {
    id: String,
    /// All node IDs in the cluster (for quorum calculation)
    cluster_members: Vec<String>,
    /// Actor placements owned by this node
    owned_actors: Arc<RwLock<HashMap<String, Bytes>>>,
    /// Simulated network connectivity
    reachable_nodes: Arc<RwLock<Vec<String>>>,
}

impl SimClusterNode {
    fn new(id: impl Into<String>, cluster_members: Vec<String>) -> Self {
        let id = id.into();
        let mut reachable = cluster_members.clone();
        // Initially all nodes are reachable
        reachable.retain(|n| n != &id); // Don't include self

        Self {
            id,
            cluster_members,
            owned_actors: Arc::new(RwLock::new(HashMap::new())),
            reachable_nodes: Arc::new(RwLock::new(reachable)),
        }
    }

    /// Get total cluster size
    fn cluster_size(&self) -> usize {
        self.cluster_members.len()
    }

    /// Get count of reachable nodes (including self)
    async fn reachable_count(&self) -> usize {
        self.reachable_nodes.read().await.len() + 1 // +1 for self
    }

    /// Check if this node has quorum
    async fn has_quorum(&self) -> bool {
        let reachable = self.reachable_count().await;
        let total = self.cluster_size();
        reachable > total / 2
    }

    /// Simulate losing connectivity to certain nodes
    async fn lose_connectivity_to(&self, nodes: &[&str]) {
        let mut reachable = self.reachable_nodes.write().await;
        for node in nodes {
            reachable.retain(|n| n != *node);
        }
    }

    /// Simulate restoring connectivity to certain nodes
    async fn restore_connectivity_to(&self, nodes: &[&str]) {
        let mut reachable = self.reachable_nodes.write().await;
        for node in nodes {
            if !reachable.contains(&node.to_string()) && *node != self.id {
                reachable.push(node.to_string());
            }
        }
    }

    /// Try to place an actor (requires quorum)
    async fn place_actor(&self, actor_id: &ActorId, state: Bytes) -> Result<(), ClusterError> {
        let reachable = self.reachable_count().await;
        let total = self.cluster_size();

        ClusterError::check_quorum(reachable, total, "place_actor")?;

        // With quorum, placement succeeds
        let mut actors = self.owned_actors.write().await;
        actors.insert(actor_id.qualified_name(), state);
        Ok(())
    }

    /// Get an actor's state (requires quorum for consistent read)
    async fn get_actor(&self, actor_id: &ActorId) -> Result<Option<Bytes>, ClusterError> {
        let reachable = self.reachable_count().await;
        let total = self.cluster_size();

        ClusterError::check_quorum(reachable, total, "get_actor")?;

        let actors = self.owned_actors.read().await;
        Ok(actors.get(&actor_id.qualified_name()).cloned())
    }

    /// Update an actor's state (requires quorum)
    async fn update_actor(&self, actor_id: &ActorId, state: Bytes) -> Result<(), ClusterError> {
        let reachable = self.reachable_count().await;
        let total = self.cluster_size();

        ClusterError::check_quorum(reachable, total, "update_actor")?;

        let mut actors = self.owned_actors.write().await;
        match actors.entry(actor_id.qualified_name()) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                entry.insert(state);
                Ok(())
            }
            std::collections::hash_map::Entry::Vacant(_) => Err(ClusterError::Internal {
                message: format!("Actor {} not found on this node", actor_id),
            }),
        }
    }
}

// =============================================================================
// Test Helpers
// =============================================================================

fn test_actor_id(n: u32) -> ActorId {
    ActorId::new("partition-test", format!("actor-{}", n)).unwrap()
}

fn create_cluster_nodes(count: usize) -> Vec<SimClusterNode> {
    let members: Vec<String> = (1..=count).map(|i| format!("node-{}", i)).collect();
    members
        .iter()
        .map(|id| SimClusterNode::new(id.clone(), members.clone()))
        .collect()
}

/// Simulate a network partition between two groups
async fn partition_groups(
    nodes: &[SimClusterNode],
    group_a_indices: &[usize],
    group_b_indices: &[usize],
) {
    // Get node IDs for each group
    let group_a_ids: Vec<String> = group_a_indices
        .iter()
        .map(|&i| nodes[i].id.clone())
        .collect();
    let group_b_ids: Vec<String> = group_b_indices
        .iter()
        .map(|&i| nodes[i].id.clone())
        .collect();

    // Group A loses connectivity to Group B
    for &i in group_a_indices {
        let ids_ref: Vec<&str> = group_b_ids.iter().map(|s| s.as_str()).collect();
        nodes[i].lose_connectivity_to(&ids_ref).await;
    }

    // Group B loses connectivity to Group A
    for &i in group_b_indices {
        let ids_ref: Vec<&str> = group_a_ids.iter().map(|s| s.as_str()).collect();
        nodes[i].lose_connectivity_to(&ids_ref).await;
    }
}

/// Heal partition between two groups
async fn heal_partition(nodes: &[SimClusterNode]) {
    // Restore all connectivity
    let all_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
    for node in nodes {
        let ids_ref: Vec<&str> = all_ids.iter().map(|s| s.as_str()).collect();
        node.restore_connectivity_to(&ids_ref).await;
    }
}

// =============================================================================
// Test 1: Minority Partition Unavailable
// =============================================================================

#[test]
fn test_minority_partition_unavailable() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|_env| async move {
        // Create 5-node cluster
        let nodes = create_cluster_nodes(5);

        // Verify all nodes start with quorum
        for node in &nodes {
            assert!(
                node.has_quorum().await,
                "All nodes should have quorum initially"
            );
        }

        // Place an actor before partition
        let actor_id = test_actor_id(1);
        nodes[0]
            .place_actor(&actor_id, Bytes::from("initial"))
            .await
            .map_err(to_core_error)?;

        // Partition: [node-1, node-2] (minority) isolated from [node-3, node-4, node-5] (majority)
        partition_groups(&nodes, &[0, 1], &[2, 3, 4]).await;

        // Verify minority (nodes 0, 1) lost quorum
        assert!(
            !nodes[0].has_quorum().await,
            "Node in minority partition should not have quorum"
        );
        assert!(
            !nodes[1].has_quorum().await,
            "Node in minority partition should not have quorum"
        );

        // Verify majority (nodes 2, 3, 4) retains quorum
        assert!(
            nodes[2].has_quorum().await,
            "Node in majority partition should have quorum"
        );
        assert!(
            nodes[3].has_quorum().await,
            "Node in majority partition should have quorum"
        );
        assert!(
            nodes[4].has_quorum().await,
            "Node in majority partition should have quorum"
        );

        // Minority MUST reject operations
        let result = nodes[0]
            .place_actor(&test_actor_id(2), Bytes::from("new"))
            .await;
        assert!(result.is_err(), "Minority partition must be unavailable");
        match result {
            Err(ClusterError::NoQuorum {
                available_nodes,
                total_nodes,
                ..
            }) => {
                assert_eq!(available_nodes, 2); // node-1 + node-2
                assert_eq!(total_nodes, 5);
            }
            Err(other) => panic!("Expected NoQuorum error, got: {:?}", other),
            Ok(_) => panic!("Expected error, got success"),
        }

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Test 2: Majority Partition Continues
// =============================================================================

#[test]
fn test_majority_partition_continues() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|_env| async move {
        // Create 5-node cluster
        let nodes = create_cluster_nodes(5);

        // Partition: [node-1, node-2] isolated from [node-3, node-4, node-5]
        partition_groups(&nodes, &[0, 1], &[2, 3, 4]).await;

        // Majority (nodes 2, 3, 4) MUST continue serving
        let actor_id = test_actor_id(1);
        let result = nodes[2]
            .place_actor(&actor_id, Bytes::from("from-majority"))
            .await;
        assert!(
            result.is_ok(),
            "Majority partition must continue: {:?}",
            result.err()
        );

        // Verify actor was placed
        let state = nodes[2].get_actor(&actor_id).await.map_err(to_core_error)?;
        assert_eq!(state, Some(Bytes::from("from-majority")));

        // Can also update
        nodes[2]
            .update_actor(&actor_id, Bytes::from("updated"))
            .await
            .map_err(to_core_error)?;
        let state = nodes[2].get_actor(&actor_id).await.map_err(to_core_error)?;
        assert_eq!(state, Some(Bytes::from("updated")));

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Test 3: Symmetric Partition (Both Sides No Quorum)
// =============================================================================

#[test]
fn test_symmetric_partition_both_unavailable() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|_env| async move {
        // Create 4-node cluster (even number)
        let nodes = create_cluster_nodes(4);

        // Partition: [node-1, node-2] isolated from [node-3, node-4]
        // Neither side has majority (2 <= 4/2)
        partition_groups(&nodes, &[0, 1], &[2, 3]).await;

        // Verify BOTH sides lost quorum
        for (i, node) in nodes.iter().enumerate() {
            assert!(
                !node.has_quorum().await,
                "Node {} should not have quorum in symmetric split",
                i
            );
        }

        // Both sides MUST be unavailable
        let result1 = nodes[0]
            .place_actor(&test_actor_id(1), Bytes::from("side-a"))
            .await;
        assert!(result1.is_err(), "Side A must be unavailable");
        assert!(matches!(result1, Err(ClusterError::NoQuorum { .. })));

        let result2 = nodes[2]
            .place_actor(&test_actor_id(2), Bytes::from("side-b"))
            .await;
        assert!(result2.is_err(), "Side B must be unavailable");
        assert!(matches!(result2, Err(ClusterError::NoQuorum { .. })));

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Test 4: Partition Healing Convergence (No Split-Brain)
// =============================================================================

#[test]
fn test_partition_healing_no_split_brain() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|_env| async move {
        // Create 5-node cluster
        let nodes = create_cluster_nodes(5);

        // Place actor before partition
        let actor_id = test_actor_id(1);
        nodes[0]
            .place_actor(&actor_id, Bytes::from("initial"))
            .await
            .map_err(to_core_error)?;

        // Create partition: [0, 1] | [2, 3, 4]
        partition_groups(&nodes, &[0, 1], &[2, 3, 4]).await;

        // Minority cannot update (no quorum)
        let result = nodes[0]
            .update_actor(&actor_id, Bytes::from("minority-update"))
            .await;
        assert!(result.is_err());

        // Majority side: place different actors (to simulate independent operation)
        let actor_id_2 = test_actor_id(2);
        nodes[2]
            .place_actor(&actor_id_2, Bytes::from("majority-new"))
            .await
            .map_err(to_core_error)?;

        // Heal partition
        heal_partition(&nodes).await;

        // Verify all nodes have quorum again
        for node in &nodes {
            assert!(
                node.has_quorum().await,
                "All nodes should have quorum after healing"
            );
        }

        // Verify no duplicate placements - the key invariant:
        // Only actor_id_2 was successfully placed (by majority)
        // actor_id was placed before partition on node 0, should still exist there

        // Node 0 should still have actor_id
        let state = nodes[0].get_actor(&actor_id).await.map_err(to_core_error)?;
        assert_eq!(
            state,
            Some(Bytes::from("initial")),
            "Actor should retain original state"
        );

        // Node 2 should have actor_id_2
        let state2 = nodes[2]
            .get_actor(&actor_id_2)
            .await
            .map_err(to_core_error)?;
        assert_eq!(state2, Some(Bytes::from("majority-new")));

        // Now updates should work from any node with the actor
        nodes[0]
            .update_actor(&actor_id, Bytes::from("post-heal-update"))
            .await
            .map_err(to_core_error)?;
        let final_state = nodes[0].get_actor(&actor_id).await.map_err(to_core_error)?;
        assert_eq!(final_state, Some(Bytes::from("post-heal-update")));

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Test 5: Asymmetric Partition
// =============================================================================

#[test]
fn test_asymmetric_partition() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|_env| async move {
        // Create 3-node cluster
        let nodes = create_cluster_nodes(3);

        // Asymmetric partition: node-1 cannot reach node-2, but node-2 can reach node-1
        // This simulates network conditions where traffic flows one way

        // node-1 loses connectivity to node-2
        nodes[0].lose_connectivity_to(&["node-2"]).await;

        // node-1 loses connectivity to node-3 as well (making it isolated)
        nodes[0].lose_connectivity_to(&["node-3"]).await;

        // Node-1 should not have quorum (can only see itself)
        assert!(
            !nodes[0].has_quorum().await,
            "Isolated node should not have quorum"
        );

        // Node-2 and Node-3 still have quorum (can see each other)
        assert!(nodes[1].has_quorum().await, "Node-2 should have quorum");
        assert!(nodes[2].has_quorum().await, "Node-3 should have quorum");

        // Isolated node cannot place actors
        let result = nodes[0]
            .place_actor(&test_actor_id(1), Bytes::from("isolated"))
            .await;
        assert!(result.is_err());
        assert!(matches!(result, Err(ClusterError::NoQuorum { .. })));

        // Connected nodes can place actors
        nodes[1]
            .place_actor(&test_actor_id(2), Bytes::from("connected"))
            .await
            .map_err(to_core_error)?;

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Test 6: Actor on Minority Side During Partition
// =============================================================================

#[test]
fn test_actor_on_minority_becomes_unavailable() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|_env| async move {
        // Create 5-node cluster
        let nodes = create_cluster_nodes(5);

        // Place actor on node-1 (will be in minority)
        let actor_id = test_actor_id(1);
        nodes[0]
            .place_actor(&actor_id, Bytes::from("before-partition"))
            .await
            .map_err(to_core_error)?;

        // Partition node-1 into minority
        partition_groups(&nodes, &[0, 1], &[2, 3, 4]).await;

        // Actor operations on node-1 MUST fail (no quorum for consistent read/write)
        let result = nodes[0].get_actor(&actor_id).await;
        assert!(result.is_err(), "Get should fail without quorum");
        assert!(matches!(result, Err(ClusterError::NoQuorum { .. })));

        let result = nodes[0]
            .update_actor(&actor_id, Bytes::from("update-attempt"))
            .await;
        assert!(result.is_err(), "Update should fail without quorum");
        assert!(matches!(result, Err(ClusterError::NoQuorum { .. })));

        // Heal partition
        heal_partition(&nodes).await;

        // Now operations should succeed
        let state = nodes[0].get_actor(&actor_id).await.map_err(to_core_error)?;
        assert_eq!(state, Some(Bytes::from("before-partition")));

        nodes[0]
            .update_actor(&actor_id, Bytes::from("after-heal"))
            .await
            .map_err(to_core_error)?;
        let state = nodes[0].get_actor(&actor_id).await.map_err(to_core_error)?;
        assert_eq!(state, Some(Bytes::from("after-heal")));

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Test 7: Determinism Verification
// =============================================================================

#[test]
fn test_partition_determinism() {
    let seed = 98765;

    let run_test = || {
        let config = SimConfig::new(seed);

        Simulation::new(config).run(|_env| async move {
            let nodes = create_cluster_nodes(5);

            // Place actors
            for i in 1..=5u32 {
                let actor_id = test_actor_id(i);
                nodes[(i as usize - 1) % 5]
                    .place_actor(&actor_id, Bytes::from(format!("state-{}", i)))
                    .await
                    .map_err(to_core_error)?;
            }

            // Create partition
            partition_groups(&nodes, &[0, 1], &[2, 3, 4]).await;

            // Collect quorum states
            let mut quorum_states: Vec<bool> = Vec::new();
            for node in &nodes {
                quorum_states.push(node.has_quorum().await);
            }

            // Heal and verify
            heal_partition(&nodes).await;

            let mut final_quorum_states: Vec<bool> = Vec::new();
            for node in &nodes {
                final_quorum_states.push(node.has_quorum().await);
            }

            Ok((quorum_states, final_quorum_states))
        })
    };

    let result1 = run_test().expect("First run failed");
    let result2 = run_test().expect("Second run failed");

    assert_eq!(
        result1, result2,
        "Partition operations should be deterministic with same seed"
    );
}

// =============================================================================
// Test 8: Network Partition with SimNetwork (Integration)
// =============================================================================

#[test]
fn test_sim_network_group_partition() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let network = &env.network;

        // Partition groups
        network
            .partition_group(&["node-1", "node-2"], &["node-3", "node-4", "node-5"])
            .await;

        // Messages within groups should work
        let sent = network
            .send("node-1", "node-2", Bytes::from("intra-minority"))
            .await;
        assert!(sent, "Intra-group message should succeed");

        let sent = network
            .send("node-3", "node-4", Bytes::from("intra-majority"))
            .await;
        assert!(sent, "Intra-group message should succeed");

        // Messages across groups should fail
        let sent = network
            .send("node-1", "node-3", Bytes::from("cross-partition"))
            .await;
        assert!(!sent, "Cross-partition message should fail");

        let sent = network
            .send("node-4", "node-2", Bytes::from("cross-partition"))
            .await;
        assert!(!sent, "Cross-partition message should fail");

        // Heal all
        network.heal_all().await;

        // Messages should work again
        let sent = network
            .send("node-1", "node-3", Bytes::from("after-heal"))
            .await;
        assert!(sent, "Message should succeed after healing");

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Test 9: One-Way Partition (SimNetwork)
// =============================================================================

#[test]
fn test_sim_network_one_way_partition() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let network = &env.network;

        // Create one-way partition: node-1 -> node-2 blocked
        network.partition_one_way("node-1", "node-2").await;

        // node-1 to node-2: blocked
        let sent = network
            .send("node-1", "node-2", Bytes::from("blocked"))
            .await;
        assert!(!sent, "One-way blocked direction should fail");

        // node-2 to node-1: works
        let sent = network
            .send("node-2", "node-1", Bytes::from("allowed"))
            .await;
        assert!(sent, "Reverse direction should succeed");

        // Verify partition state
        assert!(network.is_one_way_partitioned("node-1", "node-2").await);
        assert!(!network.is_one_way_partitioned("node-2", "node-1").await);

        // Heal one-way partition
        network.heal_one_way("node-1", "node-2").await;

        // Both directions work now
        let sent = network
            .send("node-1", "node-2", Bytes::from("healed"))
            .await;
        assert!(sent, "Should work after healing");

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Stress Test: Random Partition Patterns
// =============================================================================

#[test]
#[ignore] // Run with: cargo test -p kelpie-dst -- --ignored
fn test_partition_stress_random_patterns() {
    let config = SimConfig::from_env_or_random().with_max_steps(1_000_000);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::NetworkPartition, 0.1))
        .run(|env| async move {
            let nodes = create_cluster_nodes(7);

            // Run 100 iterations of partition/heal cycles
            for iteration in 0..100usize {
                // Random partition pattern based on iteration
                let split_point = (iteration % 6) + 1;
                let group_a: Vec<usize> = (0..split_point).collect();
                let group_b: Vec<usize> = (split_point..7).collect();

                partition_groups(&nodes, &group_a, &group_b).await;

                // Try operations on all nodes
                for (i, node) in nodes.iter().enumerate() {
                    let actor_id = test_actor_id((iteration * 10 + i) as u32);
                    let result = node.place_actor(&actor_id, Bytes::from("stress")).await;

                    // Result should match quorum status
                    let has_quorum = node.has_quorum().await;
                    match (has_quorum, result.is_ok()) {
                        (true, true) => {}   // Expected: has quorum, operation succeeds
                        (false, false) => {} // Expected: no quorum, operation fails
                        (true, false) => panic!("Node with quorum should not fail"),
                        (false, true) => panic!("Node without quorum should not succeed"),
                    }
                }

                // Heal and advance time
                heal_partition(&nodes).await;
                env.advance_time_ms(100);
            }

            Ok(())
        });

    assert!(result.is_ok(), "Stress test failed: {:?}", result.err());
}
