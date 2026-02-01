//! DST tests for lease management
//!
//! TigerStyle: Deterministic testing of lease acquisition, renewal, and expiry
//! verifying invariants from KelpieLease.tla:
//!
//! - LeaseUniqueness: At most one node holds a valid lease per actor
//! - RenewalRequiresOwnership: Only lease holder can renew
//! - ExpiredLeaseClaimable: Expired leases don't block acquisition
//!
//! Related: docs/tla/KelpieLease.tla, GitHub Issue #22

use async_trait::async_trait;
use kelpie_core::actor::ActorId;
use kelpie_core::error::Error as CoreError;
use kelpie_core::io::TimeProvider;
use kelpie_core::Runtime;
use kelpie_dst::{FaultConfig, FaultType, SimConfig, Simulation};
use kelpie_registry::{LeaseConfig, LeaseManager, MemoryLeaseManager, NodeId, RegistryError};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

// =============================================================================
// Test Clock
// =============================================================================

/// A test clock with manually controllable time.
///
/// Uses AtomicU64 for thread-safe reads across concurrent tasks.
/// SeqCst ordering ensures all tasks see consistent time values.
#[derive(Debug)]
struct TestClock {
    time_ms: AtomicU64,
}

impl TestClock {
    fn new(initial_ms: u64) -> Self {
        Self {
            time_ms: AtomicU64::new(initial_ms),
        }
    }

    fn advance(&self, ms: u64) {
        self.time_ms.fetch_add(ms, Ordering::SeqCst);
    }
}

#[async_trait]
impl TimeProvider for TestClock {
    fn now_ms(&self) -> u64 {
        self.time_ms.load(Ordering::SeqCst)
    }

    async fn sleep_ms(&self, ms: u64) {
        self.time_ms.fetch_add(ms, Ordering::SeqCst);
    }

    fn monotonic_ms(&self) -> u64 {
        self.now_ms()
    }
}

// =============================================================================
// Test Helpers
// =============================================================================

fn test_node_id(n: u32) -> NodeId {
    NodeId::new(format!("node-{}", n)).unwrap()
}

fn test_actor_id(n: u32) -> ActorId {
    ActorId::new("dst-lease", format!("actor-{}", n)).unwrap()
}

/// Convert RegistryError to CoreError for test compatibility
fn to_core_error(e: RegistryError) -> CoreError {
    CoreError::Internal {
        message: e.to_string(),
    }
}

// =============================================================================
// Lease Acquisition Tests
// =============================================================================

/// Test that lease acquisition race results in exactly one winner.
///
/// Verifies TLA+ invariant: LeaseUniqueness
/// At most one node believes it holds a valid lease per actor
#[test]
fn test_dst_lease_acquisition_race_single_winner() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let clock = Arc::new(TestClock::new(env.now_ms()));
        let lease_config = LeaseConfig::for_testing();
        let lease_mgr = Arc::new(MemoryLeaseManager::new(lease_config, clock));

        let actor_id = test_actor_id(1);

        // Multiple nodes try to acquire the same lease concurrently
        // Due to serialization, we simulate this by attempting in sequence
        // but all at the "same" time
        let mut successes = 0;
        let mut failures = 0;

        for i in 1..=5 {
            let node_id = test_node_id(i);
            match lease_mgr.acquire(&node_id, &actor_id).await {
                Ok(_) => successes += 1,
                Err(RegistryError::LeaseHeldByOther { .. }) => failures += 1,
                Err(e) => return Err(to_core_error(e)),
            }
        }

        // LeaseUniqueness: exactly one node should win
        assert_eq!(successes, 1, "Exactly one node should acquire the lease");
        assert_eq!(failures, 4, "Other nodes should fail with LeaseHeldByOther");

        // Verify the holder is valid
        let lease = lease_mgr
            .get_lease(&actor_id)
            .await
            .expect("Lease should exist");
        assert!(lease.is_valid(env.now_ms()), "Lease should be valid");

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test concurrent acquisition attempts with spawned tasks
#[test]
fn test_dst_lease_acquisition_race_concurrent_tasks() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let clock = Arc::new(TestClock::new(env.now_ms()));
        let lease_config = LeaseConfig::for_testing();
        let lease_mgr = Arc::new(MemoryLeaseManager::new(lease_config, clock));

        let actor_id = test_actor_id(1);
        let runtime = kelpie_core::current_runtime();

        // Spawn concurrent tasks trying to acquire
        let mut handles = Vec::new();
        for i in 1..=5 {
            let mgr = lease_mgr.clone();
            let id = actor_id.clone();
            let node_id = test_node_id(i);

            let handle = runtime.spawn(async move { mgr.acquire(&node_id, &id).await });
            handles.push(handle);
        }

        // Collect results
        let results: Vec<_> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        let successes = results.iter().filter(|r| r.is_ok()).count();

        // LeaseUniqueness: exactly one winner
        assert_eq!(successes, 1, "Exactly one task should succeed");

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Lease Expiry Tests
// =============================================================================

/// Test that expired leases can be reacquired by other nodes.
///
/// Verifies TLA+ invariant: ExpiredLeaseClaimable
/// Expired leases don't block new acquisition
#[test]
fn test_dst_lease_expiry_allows_reacquisition() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let clock = Arc::new(TestClock::new(env.now_ms()));
        // Short lease duration for testing expiry
        let lease_config = LeaseConfig::new(5000); // 5 seconds
        let lease_mgr = Arc::new(MemoryLeaseManager::new(lease_config, clock.clone()));

        let actor_id = test_actor_id(1);
        let node1 = test_node_id(1);
        let node2 = test_node_id(2);

        // Node 1 acquires lease
        let lease = lease_mgr
            .acquire(&node1, &actor_id)
            .await
            .map_err(to_core_error)?;
        assert!(
            lease.is_valid(clock.now_ms()),
            "Lease should be valid initially"
        );

        // Node 2 cannot acquire while lease is valid
        let result = lease_mgr.acquire(&node2, &actor_id).await;
        assert!(
            matches!(result, Err(RegistryError::LeaseHeldByOther { .. })),
            "Node 2 should not be able to acquire while lease is valid"
        );

        // Advance time past lease expiry
        clock.advance(6000); // 6 seconds
        env.advance_time_ms(6000);

        // Verify lease is now expired
        assert!(
            !lease_mgr.is_valid(&node1, &actor_id).await,
            "Lease should be expired after time advance"
        );

        // Node 2 can now acquire (ExpiredLeaseClaimable)
        let new_lease = lease_mgr
            .acquire(&node2, &actor_id)
            .await
            .map_err(to_core_error)?;
        assert_eq!(new_lease.holder, node2, "Node 2 should now hold the lease");
        assert!(
            new_lease.is_valid(clock.now_ms()),
            "New lease should be valid"
        );

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Lease Renewal Tests
// =============================================================================

/// Test that lease renewal extends validity.
#[test]
fn test_dst_lease_renewal_extends_validity() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let clock = Arc::new(TestClock::new(env.now_ms()));
        let lease_config = LeaseConfig::new(5000); // 5 seconds
        let lease_mgr = Arc::new(MemoryLeaseManager::new(lease_config, clock.clone()));

        let actor_id = test_actor_id(1);
        let node_id = test_node_id(1);

        // Acquire lease
        let lease = lease_mgr
            .acquire(&node_id, &actor_id)
            .await
            .map_err(to_core_error)?;
        let original_expiry = lease.expiry_ms;

        // Advance time (but not past expiry)
        clock.advance(2500); // Half the lease duration
        env.advance_time_ms(2500);

        // Renew lease
        let renewed = lease_mgr
            .renew(&node_id, &actor_id)
            .await
            .map_err(to_core_error)?;
        assert_eq!(renewed.renewal_count, 1, "Renewal count should increment");
        assert!(
            renewed.expiry_ms > original_expiry,
            "Renewed expiry should be later than original"
        );

        // Advance time past original expiry
        clock.advance(3000); // Now at 5.5 seconds
        env.advance_time_ms(3000);

        // Lease should still be valid due to renewal
        assert!(
            lease_mgr.is_valid(&node_id, &actor_id).await,
            "Renewed lease should still be valid past original expiry"
        );

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test that non-holder cannot renew.
///
/// Verifies TLA+ invariant: RenewalRequiresOwnership
/// Only lease holder can renew
#[test]
fn test_dst_non_holder_cannot_renew() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let clock = Arc::new(TestClock::new(env.now_ms()));
        let lease_config = LeaseConfig::for_testing();
        let lease_mgr = Arc::new(MemoryLeaseManager::new(lease_config, clock));

        let actor_id = test_actor_id(1);
        let node1 = test_node_id(1);
        let node2 = test_node_id(2);

        // Node 1 acquires
        lease_mgr
            .acquire(&node1, &actor_id)
            .await
            .map_err(to_core_error)?;

        // Node 2 tries to renew - should fail (RenewalRequiresOwnership)
        let result = lease_mgr.renew(&node2, &actor_id).await;
        assert!(
            matches!(result, Err(RegistryError::NotLeaseHolder { .. })),
            "Non-holder should not be able to renew"
        );

        // Verify lease is still held by node 1
        let lease = lease_mgr
            .get_lease(&actor_id)
            .await
            .expect("Lease should exist");
        assert_eq!(lease.holder, node1, "Original holder should remain");

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Lease Release Tests
// =============================================================================

/// Test that lease release allows immediate reacquisition.
#[test]
fn test_dst_lease_release_allows_reacquisition() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let clock = Arc::new(TestClock::new(env.now_ms()));
        let lease_config = LeaseConfig::for_testing();
        let lease_mgr = Arc::new(MemoryLeaseManager::new(lease_config, clock));

        let actor_id = test_actor_id(1);
        let node1 = test_node_id(1);
        let node2 = test_node_id(2);

        // Node 1 acquires
        lease_mgr
            .acquire(&node1, &actor_id)
            .await
            .map_err(to_core_error)?;

        // Node 2 cannot acquire
        let result = lease_mgr.acquire(&node2, &actor_id).await;
        assert!(matches!(
            result,
            Err(RegistryError::LeaseHeldByOther { .. })
        ));

        // Node 1 releases
        lease_mgr
            .release(&node1, &actor_id)
            .await
            .map_err(to_core_error)?;

        // Verify lease is gone
        assert!(
            !lease_mgr.is_valid(&node1, &actor_id).await,
            "Lease should be invalid after release"
        );

        // Node 2 can now acquire immediately (without waiting for expiry)
        let lease = lease_mgr
            .acquire(&node2, &actor_id)
            .await
            .map_err(to_core_error)?;
        assert_eq!(lease.holder, node2, "Node 2 should now hold the lease");

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test that non-holder cannot release.
#[test]
fn test_dst_non_holder_cannot_release() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let clock = Arc::new(TestClock::new(env.now_ms()));
        let lease_config = LeaseConfig::for_testing();
        let lease_mgr = Arc::new(MemoryLeaseManager::new(lease_config, clock));

        let actor_id = test_actor_id(1);
        let node1 = test_node_id(1);
        let node2 = test_node_id(2);

        // Node 1 acquires
        lease_mgr
            .acquire(&node1, &actor_id)
            .await
            .map_err(to_core_error)?;

        // Node 2 tries to release - should fail
        let result = lease_mgr.release(&node2, &actor_id).await;
        assert!(
            matches!(result, Err(RegistryError::NotLeaseHolder { .. })),
            "Non-holder should not be able to release"
        );

        // Verify lease is still held by node 1
        assert!(
            lease_mgr.is_valid(&node1, &actor_id).await,
            "Lease should still be valid"
        );

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Fault Injection Tests
// =============================================================================

/// Test lease operations with storage faults.
#[test]
fn test_dst_lease_with_storage_faults() {
    let config = SimConfig::new(42);

    // Run with storage write faults (10% probability)
    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.1))
        .run(|env| async move {
            let clock = Arc::new(TestClock::new(env.now_ms()));
            let lease_config = LeaseConfig::for_testing();
            let lease_mgr = Arc::new(MemoryLeaseManager::new(lease_config, clock.clone()));

            // Perform multiple lease operations
            for i in 1..=10 {
                let actor_id = test_actor_id(i);
                let node_id = test_node_id(1);

                // These should all succeed (MemoryLeaseManager doesn't use storage layer)
                // but this tests the DST infrastructure
                let _ = lease_mgr.acquire(&node_id, &actor_id).await;
            }

            // Verify at least some operations succeeded
            let leases = lease_mgr.get_leases_for_node(&test_node_id(1)).await;
            assert!(!leases.is_empty(), "Should have some leases");

            Ok(())
        });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Determinism Tests
// =============================================================================

/// Test that lease operations are deterministic with the same seed.
#[test]
fn test_dst_lease_determinism() {
    let seed = 12345;

    let run_test = || {
        let config = SimConfig::new(seed);

        Simulation::new(config).run(|env| async move {
            let clock = Arc::new(TestClock::new(env.now_ms()));
            let lease_config = LeaseConfig::new(5000);
            let lease_mgr = Arc::new(MemoryLeaseManager::new(lease_config, clock.clone()));

            let mut results = Vec::new();

            // Perform deterministic sequence of operations
            for i in 1..=5 {
                let actor_id = test_actor_id(i);
                let node_id = test_node_id((i % 3) + 1);

                let result = lease_mgr.acquire(&node_id, &actor_id).await;
                results.push((
                    actor_id.qualified_name(),
                    node_id.as_str().to_string(),
                    result.is_ok(),
                ));

                // Advance time deterministically
                clock.advance(1000);
            }

            Ok(results)
        })
    };

    let result1 = run_test().expect("First run failed");
    let result2 = run_test().expect("Second run failed");

    assert_eq!(
        result1, result2,
        "Lease operations should be deterministic with same seed"
    );
}

// =============================================================================
// Invariant Verification Helpers
// =============================================================================

/// Verify LeaseUniqueness invariant: at most one node holds a valid lease per actor
async fn verify_lease_uniqueness(
    lease_mgr: &MemoryLeaseManager,
    actor_id: &ActorId,
    nodes: &[NodeId],
) -> bool {
    let valid_count =
        futures::future::join_all(nodes.iter().map(|node| lease_mgr.is_valid(node, actor_id)))
            .await
            .into_iter()
            .filter(|&valid| valid)
            .count();

    valid_count <= 1
}

/// Test LeaseUniqueness invariant across operations
#[test]
fn test_dst_lease_uniqueness_invariant() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let clock = Arc::new(TestClock::new(env.now_ms()));
        let lease_config = LeaseConfig::new(5000);
        let lease_mgr = Arc::new(MemoryLeaseManager::new(lease_config, clock.clone()));

        let actor_id = test_actor_id(1);
        let nodes: Vec<NodeId> = (1..=5).map(test_node_id).collect();

        // Verify invariant before any operations
        assert!(
            verify_lease_uniqueness(&lease_mgr, &actor_id, &nodes).await,
            "LeaseUniqueness should hold initially"
        );

        // Node 1 acquires
        lease_mgr
            .acquire(&nodes[0], &actor_id)
            .await
            .map_err(to_core_error)?;
        assert!(
            verify_lease_uniqueness(&lease_mgr, &actor_id, &nodes).await,
            "LeaseUniqueness should hold after acquisition"
        );

        // Other nodes try to acquire
        for node in &nodes[1..] {
            let _ = lease_mgr.acquire(node, &actor_id).await;
        }
        assert!(
            verify_lease_uniqueness(&lease_mgr, &actor_id, &nodes).await,
            "LeaseUniqueness should hold after failed acquisitions"
        );

        // Advance time past expiry
        clock.advance(6000);

        // Node 2 acquires after expiry
        lease_mgr
            .acquire(&nodes[1], &actor_id)
            .await
            .map_err(to_core_error)?;
        assert!(
            verify_lease_uniqueness(&lease_mgr, &actor_id, &nodes).await,
            "LeaseUniqueness should hold after reacquisition"
        );

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Stress Tests (longer duration, marked as ignored for CI)
// =============================================================================

#[test]
#[ignore] // Run with: cargo test -p kelpie-dst --test lease_dst -- --ignored
fn test_dst_lease_stress_many_actors() {
    let config = SimConfig::from_env_or_random().with_max_steps(1_000_000);

    let result = Simulation::new(config).run(|env| async move {
        let clock = Arc::new(TestClock::new(env.now_ms()));
        let lease_config = LeaseConfig::new(5000);
        let lease_mgr = Arc::new(MemoryLeaseManager::new(lease_config, clock.clone()));

        let num_actors = 100;
        let num_nodes = 10;
        let iterations = 1000;

        for iter in 0..iterations {
            let actor_idx = (iter % num_actors) + 1;
            let node_idx = ((iter / 3) % num_nodes) + 1;

            let actor_id = test_actor_id(actor_idx as u32);
            let node_id = test_node_id(node_idx as u32);

            // Try acquire or renew randomly based on iteration
            if iter % 5 == 0 {
                // Advance time occasionally to cause expirations
                clock.advance(2000);
            }

            let _ = lease_mgr.acquire(&node_id, &actor_id).await;

            // Verify invariant periodically
            if iter % 100 == 0 {
                let nodes: Vec<NodeId> = (1..=num_nodes as u32).map(test_node_id).collect();
                assert!(
                    verify_lease_uniqueness(&lease_mgr, &actor_id, &nodes).await,
                    "LeaseUniqueness violated at iteration {}",
                    iter
                );
            }
        }

        Ok(())
    });

    assert!(result.is_ok(), "Stress test failed: {:?}", result.err());
}
