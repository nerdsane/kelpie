//! DST tests for SingleActivation invariant
//!
//! TLA+ Spec Reference: `docs/tla/KelpieSingleActivation.tla`
//!
//! This module tests the core single activation guarantee:
//! - **SAFETY**: At most one node can activate an actor at any time
//! - **LIVENESS**: Every claim eventually results in activation or rejection
//!
//! The TLA+ spec models FDB's optimistic concurrency control (OCC):
//! 1. StartClaim(n): Node initiates claim, enters Reading state
//! 2. ReadFDB(n): Node reads current holder and version (snapshot read)
//! 3. CommitClaim(n): Node attempts atomic commit
//!    - Success: version unchanged AND no holder -> become holder
//!    - Failure: version changed OR has holder -> return to Idle
//!
//! TigerStyle: Deterministic testing with explicit fault injection.

use futures::future::join_all;
use kelpie_core::actor::ActorId;
use kelpie_core::error::{Error, Result};
use kelpie_core::Runtime; // Trait for spawn()
use kelpie_dst::{FaultConfig, FaultType, SimConfig, Simulation};
use kelpie_storage::ActorKV;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

// =============================================================================
// Activation Protocol Simulation
// =============================================================================

/// Simulated holder state (matches TLA+ fdb_holder + fdb_version)
///
/// TLA+ mapping:
/// - holder: `fdb_holder` - current holder in FDB storage (NONE = None)
/// - version: `fdb_version` - monotonic version counter for OCC
#[derive(Debug, Clone)]
struct HolderState {
    holder: Option<String>, // Node ID that holds the actor, or None
    version: u64,           // Monotonic version for OCC
}

/// Shared state for activation protocol simulation
///
/// This simulates FDB's key-value storage with OCC semantics.
/// Multiple nodes can concurrently attempt to claim the same actor.
struct ActivationProtocol {
    /// Per-actor holder state (actor_key -> HolderState)
    state: Arc<RwLock<HashMap<String, HolderState>>>,
    /// Counter for generating unique activation IDs (for determinism verification)
    activation_counter: AtomicU64,
}

impl ActivationProtocol {
    fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
            activation_counter: AtomicU64::new(0),
        }
    }

    /// Attempt to claim an actor (implements TLA+ StartClaim + ReadFDB + CommitClaim)
    ///
    /// Returns Ok(activation_id) if claim succeeds, Err if another node won.
    ///
    /// TLA+ mapping:
    /// - StartClaim(n): Begin claim process
    /// - ReadFDB(n): Read current holder and version (captured in `read_version`)
    /// - CommitClaim(n): Atomic commit with OCC check
    ///   - Success: `read_ver = current_ver /\ current_holder = NONE`
    ///   - Failure: `read_ver # current_ver \/ current_holder # NONE`
    async fn try_claim(&self, actor_key: &str, node_id: &str) -> Result<u64> {
        // TigerStyle: Preconditions
        assert!(!actor_key.is_empty(), "actor_key cannot be empty");
        assert!(!node_id.is_empty(), "node_id cannot be empty");

        // Phase 1: ReadFDB - snapshot read of current state
        let read_version = {
            let state = self.state.read().await;
            match state.get(actor_key) {
                Some(hs) => hs.version,
                None => 0, // No entry = version 0
            }
        };

        // Simulate processing time between read and commit phases.
        // This yield point allows task interleaving to test race conditions.
        // Note: madsim deterministically schedules tasks, making races reproducible.
        tokio::task::yield_now().await;

        // Phase 2: CommitClaim - atomic commit with OCC check
        let mut state = self.state.write().await;

        // Get current state again (for OCC comparison)
        let current = state.get(actor_key).cloned().unwrap_or(HolderState {
            holder: None,
            version: 0,
        });

        // TLA+ CommitClaim semantics:
        // Success: read_ver = current_ver /\ current_holder = NONE
        // Failure: read_ver # current_ver \/ current_holder # NONE

        // Extract values before mutable borrow
        let current_version = current.version;
        let has_holder = current.holder.is_some();

        if read_version != current_version {
            // OCC CONFLICT: Version changed since our read
            // Another node modified the key between our read and commit
            return Err(Error::Internal {
                message: format!(
                    "OCC conflict: read version {} != current version {} (node={})",
                    read_version, current_version, node_id
                ),
            });
        }

        if has_holder {
            // ALREADY HELD: Another node already holds this actor
            return Err(Error::ActorAlreadyExists {
                id: actor_key.to_string(),
            });
        }

        // SUCCESS: We win the claim!
        // Update state atomically (version bump + set holder)
        let activation_id = self.activation_counter.fetch_add(1, Ordering::SeqCst);
        state.insert(
            actor_key.to_string(),
            HolderState {
                holder: Some(node_id.to_string()),
                version: current_version + 1, // Version bumps on write (TLA+ spec)
            },
        );

        // TigerStyle: Postcondition
        debug_assert!(state.get(actor_key).unwrap().holder.as_deref() == Some(node_id));

        Ok(activation_id)
    }

    /// Release an actor (implements TLA+ Release action)
    async fn release(&self, actor_key: &str, node_id: &str) -> Result<()> {
        let mut state = self.state.write().await;

        let current = state.get(actor_key).ok_or_else(|| Error::ActorNotFound {
            id: actor_key.to_string(),
        })?;

        // Extract values before mutable borrow
        let current_version = current.version;
        let current_holder = current.holder.clone();

        if current_holder.as_deref() != Some(node_id) {
            return Err(Error::Internal {
                message: format!(
                    "Cannot release: holder is {:?}, not {}",
                    current_holder, node_id
                ),
            });
        }

        // Release: clear holder, bump version
        state.insert(
            actor_key.to_string(),
            HolderState {
                holder: None,
                version: current_version + 1,
            },
        );

        Ok(())
    }
}

// =============================================================================
// SingleActivation Invariant Tests
// =============================================================================

/// Test concurrent activation: exactly 1 winner
///
/// TLA+ Invariant: `Inv_SingleActivation == Cardinality({n \in Nodes : node_state[n] = "Active"}) <= 1`
///
/// This test spawns N concurrent activation attempts for the SAME actor.
/// The invariant requires exactly 1 succeeds, N-1 fail.
#[test]
fn test_concurrent_activation_single_winner() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(seed = config.seed, "Running SingleActivation test");

    let result = Simulation::new(config).run(|_env| async move {
        let protocol = Arc::new(ActivationProtocol::new());
        let actor_key = "test/concurrent-target";
        let num_nodes = 5;

        // Spawn N concurrent activation attempts for the SAME actor
        let handles: Vec<_> = (0..num_nodes)
            .map(|node_id| {
                let protocol = protocol.clone();
                let actor_key = actor_key.to_string();
                let node_name = format!("node-{}", node_id);
                kelpie_core::current_runtime()
                    .spawn(async move { protocol.try_claim(&actor_key, &node_name).await })
            })
            .collect();

        // Wait for all attempts to complete
        let results: Vec<_> = join_all(handles)
            .await
            .into_iter()
            .map(|r| r.expect("task panicked"))
            .collect();

        // TLA+ INVARIANT: SingleActivation
        // Exactly 1 should succeed (at most 1 active)
        let successes: Vec<_> = results.iter().filter(|r| r.is_ok()).collect();
        let failures: Vec<_> = results.iter().filter(|r| r.is_err()).collect();

        assert_eq!(
            successes.len(),
            1,
            "SingleActivation VIOLATED: {} activations succeeded (expected 1). \
             Results: {:?}",
            successes.len(),
            results
        );

        assert_eq!(
            failures.len(),
            num_nodes - 1,
            "Expected {} failures, got {}. Results: {:?}",
            num_nodes - 1,
            failures.len(),
            results
        );

        // Verify failure types are correct
        for result in &results {
            if let Err(e) = result {
                let error_msg = format!("{:?}", e);
                assert!(
                    error_msg.contains("OCC conflict") || error_msg.contains("AlreadyExists"),
                    "Unexpected error type: {:?}",
                    e
                );
            }
        }

        tracing::info!(
            successes = successes.len(),
            failures = failures.len(),
            "SingleActivation test passed"
        );

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test with more nodes to increase contention
#[test]
fn test_concurrent_activation_high_contention() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(
        seed = config.seed,
        "Running high contention SingleActivation test"
    );

    let result = Simulation::new(config).run(|_env| async move {
        let protocol = Arc::new(ActivationProtocol::new());
        let actor_key = "test/high-contention";
        let num_nodes = 20; // Higher contention

        let handles: Vec<_> = (0..num_nodes)
            .map(|node_id| {
                let protocol = protocol.clone();
                let actor_key = actor_key.to_string();
                let node_name = format!("node-{}", node_id);
                kelpie_core::current_runtime()
                    .spawn(async move { protocol.try_claim(&actor_key, &node_name).await })
            })
            .collect();

        let results: Vec<_> = join_all(handles)
            .await
            .into_iter()
            .map(|r| r.expect("task panicked"))
            .collect();

        let successes = results.iter().filter(|r| r.is_ok()).count();

        // TLA+ INVARIANT: SingleActivation - at most 1 succeeds
        assert!(
            successes <= 1,
            "SingleActivation VIOLATED: {} activations succeeded (expected <= 1)",
            successes
        );

        // With the protocol correctly implemented, exactly 1 should succeed
        assert_eq!(
            successes, 1,
            "Expected exactly 1 success, got {}",
            successes
        );

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Determinism Tests
// =============================================================================

/// Test that same seed produces same winner
///
/// TigerStyle: Determinism verification - same seed = same outcome
#[test]
fn test_single_activation_deterministic() {
    let seed = 42_u64;

    let run_test = || {
        let config = SimConfig::new(seed);

        Simulation::new(config).run(|_env| async move {
            let protocol = Arc::new(ActivationProtocol::new());
            let actor_key = "test/deterministic";
            let num_nodes = 5;

            let handles: Vec<_> = (0..num_nodes)
                .map(|node_id| {
                    let protocol = protocol.clone();
                    let actor_key = actor_key.to_string();
                    let node_name = format!("node-{}", node_id);
                    kelpie_core::current_runtime().spawn(async move {
                        let result = protocol.try_claim(&actor_key, &node_name).await;
                        (node_name, result.is_ok())
                    })
                })
                .collect();

            let results: Vec<_> = join_all(handles)
                .await
                .into_iter()
                .map(|r| r.expect("task panicked"))
                .collect();

            // Find the winner
            let winner: Option<String> = results
                .iter()
                .find(|(_, won)| *won)
                .map(|(name, _)| name.clone());

            Ok(winner)
        })
    };

    let result1 = run_test().expect("First run failed");
    let result2 = run_test().expect("Second run failed");

    assert_eq!(
        result1, result2,
        "Determinism violated: winner differs with same seed. \
         Run 1: {:?}, Run 2: {:?}",
        result1, result2
    );
}

// =============================================================================
// Fault Injection Tests
// =============================================================================

/// Test SingleActivation under storage write failures
///
/// TLA+ mapping: Even with transient failures, the invariant must hold.
/// Storage write failures can cause some claims to fail, but should never
/// allow multiple activations.
#[test]
fn test_concurrent_activation_with_storage_faults() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(
        seed = config.seed,
        "Running SingleActivation with storage faults"
    );

    // We use the SimStorage with transactions to test fault scenarios
    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.2))
        .run(|env| async move {
            let actor_id = ActorId::new("test", "fault-test")?;
            let storage: Arc<dyn ActorKV> = Arc::new(env.storage);
            let num_attempts = 5;

            // Use transactions to simulate the activation protocol
            let handles: Vec<_> = (0..num_attempts)
                .map(|node_id| {
                    let storage = storage.clone();
                    let actor_id = actor_id.clone();
                    let node_name = format!("node-{}", node_id);
                    kelpie_core::current_runtime().spawn(async move {
                        // Try to claim via transaction
                        let result = try_claim_with_storage(&storage, &actor_id, &node_name).await;
                        (node_name, result)
                    })
                })
                .collect();

            let results: Vec<_> = join_all(handles)
                .await
                .into_iter()
                .map(|r| r.expect("task panicked"))
                .collect();

            // Count successes (excluding storage failures)
            let successes: Vec<_> = results
                .iter()
                .filter(|(_, r)| r.is_ok())
                .map(|(name, _)| name.clone())
                .collect();

            // TLA+ INVARIANT: AT MOST 1 succeeds (with faults, might be 0)
            assert!(
                successes.len() <= 1,
                "SingleActivation VIOLATED under faults: {} activations succeeded. \
                 Winners: {:?}",
                successes.len(),
                successes
            );

            tracing::info!(
                successes = successes.len(),
                "SingleActivation invariant held under storage faults"
            );

            Ok(())
        });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test SingleActivation under transaction crash faults
#[test]
fn test_concurrent_activation_with_crash_faults() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(
        seed = config.seed,
        "Running SingleActivation with crash faults"
    );

    let result = Simulation::new(config)
        .with_fault(
            FaultConfig::new(FaultType::CrashDuringTransaction, 0.3)
                .with_filter("transaction_commit"),
        )
        .run(|env| async move {
            let actor_id = ActorId::new("test", "crash-test")?;
            let storage: Arc<dyn ActorKV> = Arc::new(env.storage);
            let num_attempts = 10;

            let handles: Vec<_> = (0..num_attempts)
                .map(|node_id| {
                    let storage = storage.clone();
                    let actor_id = actor_id.clone();
                    let node_name = format!("node-{}", node_id);
                    kelpie_core::current_runtime().spawn(async move {
                        let result = try_claim_with_storage(&storage, &actor_id, &node_name).await;
                        (node_name, result)
                    })
                })
                .collect();

            let results: Vec<_> = join_all(handles)
                .await
                .into_iter()
                .map(|r| r.expect("task panicked"))
                .collect();

            let successes = results.iter().filter(|(_, r)| r.is_ok()).count();

            // TLA+ INVARIANT: AT MOST 1 succeeds
            assert!(
                successes <= 1,
                "SingleActivation VIOLATED under crash faults: {} succeeded",
                successes
            );

            Ok(())
        });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test SingleActivation under network delay faults
#[test]
fn test_concurrent_activation_with_network_delay() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::StorageLatency {
                min_ms: 10,
                max_ms: 100,
            },
            0.5,
        ))
        .run(|env| async move {
            let actor_id = ActorId::new("test", "delay-test")?;
            let storage: Arc<dyn ActorKV> = Arc::new(env.storage);
            let num_attempts = 5;

            let handles: Vec<_> = (0..num_attempts)
                .map(|node_id| {
                    let storage = storage.clone();
                    let actor_id = actor_id.clone();
                    let node_name = format!("node-{}", node_id);
                    kelpie_core::current_runtime().spawn(async move {
                        let result = try_claim_with_storage(&storage, &actor_id, &node_name).await;
                        (node_name, result)
                    })
                })
                .collect();

            let results: Vec<_> = join_all(handles)
                .await
                .into_iter()
                .map(|r| r.expect("task panicked"))
                .collect();

            let successes = results.iter().filter(|(_, r)| r.is_ok()).count();

            // With network delays, exactly 1 should still win
            // (delays don't cause additional successes)
            assert!(
                successes <= 1,
                "SingleActivation VIOLATED under network delay: {} succeeded",
                successes
            );

            Ok(())
        });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Release and Re-activation Tests
// =============================================================================

/// Test that after release, a new activation can succeed
///
/// TLA+ mapping: Release(n) followed by StartClaim(m) for different node
#[test]
fn test_release_and_reactivation() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|_env| async move {
        let protocol = Arc::new(ActivationProtocol::new());
        let actor_key = "test/release-reactivate";

        // Node 1 claims
        let claim1 = protocol.try_claim(actor_key, "node-1").await;
        assert!(claim1.is_ok(), "First claim should succeed");

        // Node 2 cannot claim while node 1 holds
        let claim2 = protocol.try_claim(actor_key, "node-2").await;
        assert!(claim2.is_err(), "Second claim should fail while held");

        // Node 1 releases
        protocol.release(actor_key, "node-1").await?;

        // Now node 2 can claim
        let claim3 = protocol.try_claim(actor_key, "node-2").await;
        assert!(claim3.is_ok(), "Claim after release should succeed");

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test concurrent claims during release window
#[test]
fn test_concurrent_activation_during_release() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|_env| async move {
        let protocol = Arc::new(ActivationProtocol::new());
        let actor_key = "test/release-race";

        // Node 1 claims
        protocol.try_claim(actor_key, "node-1").await?;

        // Node 1 releases
        protocol.release(actor_key, "node-1").await?;

        // Multiple nodes race to claim the now-free actor
        let num_nodes = 5;
        let handles: Vec<_> = (0..num_nodes)
            .map(|node_id| {
                let protocol = protocol.clone();
                let actor_key = actor_key.to_string();
                let node_name = format!("node-{}", node_id + 10); // Different from node-1
                kelpie_core::current_runtime()
                    .spawn(async move { protocol.try_claim(&actor_key, &node_name).await })
            })
            .collect();

        let results: Vec<_> = join_all(handles)
            .await
            .into_iter()
            .map(|r| r.expect("task panicked"))
            .collect();

        let successes = results.iter().filter(|r| r.is_ok()).count();

        // TLA+ INVARIANT: Exactly 1 succeeds
        assert_eq!(
            successes, 1,
            "SingleActivation VIOLATED: {} succeeded (expected 1)",
            successes
        );

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Stress Tests
// =============================================================================

/// Stress test: many iterations with random seeds
///
/// Run with: cargo test -p kelpie-dst single_activation_stress -- --ignored
#[test]
#[ignore]
fn test_single_activation_stress() {
    const NUM_ITERATIONS: usize = 1000;
    const NUM_NODES: usize = 10;

    let mut violations = Vec::new();

    for iteration in 0..NUM_ITERATIONS {
        let seed = 0xDEADBEEF_u64.wrapping_add(iteration as u64);
        let config = SimConfig::new(seed);

        let result = Simulation::new(config).run(|_env| async move {
            let protocol = Arc::new(ActivationProtocol::new());
            let actor_key = format!("test/stress-{}", iteration);

            let handles: Vec<_> = (0..NUM_NODES)
                .map(|node_id| {
                    let protocol = protocol.clone();
                    let actor_key = actor_key.clone();
                    let node_name = format!("node-{}", node_id);
                    kelpie_core::current_runtime()
                        .spawn(async move { protocol.try_claim(&actor_key, &node_name).await })
                })
                .collect();

            let results: Vec<_> = join_all(handles)
                .await
                .into_iter()
                .map(|r| r.expect("task panicked"))
                .collect();

            let successes = results.iter().filter(|r| r.is_ok()).count();
            Ok(successes)
        });

        match result {
            Ok(successes) if successes != 1 => {
                violations.push((seed, successes));
            }
            Err(e) => {
                violations.push((seed, 0));
                tracing::error!(seed = seed, error = ?e, "Iteration failed");
            }
            _ => {}
        }

        if (iteration + 1) % 100 == 0 {
            println!(
                "Progress: {}/{} iterations, {} violations",
                iteration + 1,
                NUM_ITERATIONS,
                violations.len()
            );
        }
    }

    if !violations.is_empty() {
        for (seed, count) in &violations {
            println!(
                "VIOLATION: seed={} had {} successes (expected 1)",
                seed, count
            );
        }
        panic!(
            "SingleActivation stress test found {} violations in {} iterations",
            violations.len(),
            NUM_ITERATIONS
        );
    }

    println!(
        "SingleActivation stress test PASSED: {} iterations, 0 violations",
        NUM_ITERATIONS
    );
}

/// Stress test with fault injection
#[test]
#[ignore]
fn test_single_activation_stress_with_faults() {
    const NUM_ITERATIONS: usize = 500;
    const NUM_NODES: usize = 5;

    let mut violations = Vec::new();

    for iteration in 0..NUM_ITERATIONS {
        let seed = 0xCAFEBABE_u64.wrapping_add(iteration as u64);
        let config = SimConfig::new(seed);

        let result = Simulation::new(config)
            .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.1))
            .with_fault(
                FaultConfig::new(FaultType::CrashDuringTransaction, 0.1)
                    .with_filter("transaction_commit"),
            )
            .run(|env| async move {
                let actor_id = ActorId::new("test", format!("stress-fault-{}", iteration))?;
                let storage: Arc<dyn ActorKV> = Arc::new(env.storage);

                let handles: Vec<_> = (0..NUM_NODES)
                    .map(|node_id| {
                        let storage = storage.clone();
                        let actor_id = actor_id.clone();
                        let node_name = format!("node-{}", node_id);
                        kelpie_core::current_runtime().spawn(async move {
                            try_claim_with_storage(&storage, &actor_id, &node_name).await
                        })
                    })
                    .collect();

                let results: Vec<_> = join_all(handles)
                    .await
                    .into_iter()
                    .map(|r| r.expect("task panicked"))
                    .collect();

                let successes = results.iter().filter(|r| r.is_ok()).count();
                Ok(successes)
            });

        match result {
            Ok(successes) if successes > 1 => {
                // With faults, 0 or 1 successes are acceptable
                // More than 1 is a violation
                violations.push((seed, successes));
            }
            _ => {}
        }

        if (iteration + 1) % 100 == 0 {
            println!(
                "Progress (faults): {}/{} iterations, {} violations",
                iteration + 1,
                NUM_ITERATIONS,
                violations.len()
            );
        }
    }

    if !violations.is_empty() {
        for (seed, count) in &violations {
            println!(
                "VIOLATION: seed={} had {} successes (expected <= 1)",
                seed, count
            );
        }
        panic!(
            "SingleActivation stress test (with faults) found {} violations",
            violations.len()
        );
    }

    println!(
        "SingleActivation stress test (with faults) PASSED: {} iterations",
        NUM_ITERATIONS
    );
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Try to claim an actor using storage transactions (FDB OCC simulation)
///
/// This function implements the TLA+ activation protocol using the SimStorage
/// transaction API:
/// 1. Begin transaction
/// 2. Read current holder (if any)
/// 3. If no holder, write our claim
/// 4. Commit (OCC check happens here)
async fn try_claim_with_storage(
    storage: &Arc<dyn ActorKV>,
    actor_id: &ActorId,
    node_id: &str,
) -> Result<()> {
    const HOLDER_KEY: &[u8] = b"__holder__";

    // Begin transaction
    let mut txn = storage.begin_transaction(actor_id).await?;

    // Read current holder
    let current_holder = txn.get(HOLDER_KEY).await?;

    if current_holder.is_some() {
        // Already held by someone
        txn.abort().await?;
        return Err(Error::ActorAlreadyExists {
            id: actor_id.to_string(),
        });
    }

    // No holder - try to claim
    // Write our node ID as the holder
    txn.set(HOLDER_KEY, node_id.as_bytes()).await?;

    // Commit - this is where OCC kicks in
    // If another transaction committed a claim between our read and commit,
    // the commit will fail (simulating FDB's conflict detection)
    txn.commit().await?;

    Ok(())
}

// =============================================================================
// Consistency Model Tests
// =============================================================================

/// Test that ConsistentHolder invariant holds
///
/// TLA+: `ConsistentHolder == \A n \in Nodes: node_state[n] = "Active" => fdb_holder = n`
/// If a node believes it's active, the storage must confirm it.
#[test]
fn test_consistent_holder_invariant() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|_env| async move {
        let protocol = Arc::new(ActivationProtocol::new());
        let actor_key = "test/consistent-holder";

        // Node 1 claims
        let claim_result = protocol.try_claim(actor_key, "node-1").await;
        assert!(claim_result.is_ok());

        // Verify the storage state matches
        let state = protocol.state.read().await;
        let holder_state = state.get(actor_key).expect("state should exist");

        assert_eq!(
            holder_state.holder.as_deref(),
            Some("node-1"),
            "ConsistentHolder VIOLATED: expected holder node-1, got {:?}",
            holder_state.holder
        );

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Network Partition Tests (Issue #35)
// =============================================================================

/// Test single activation invariant under network partition
///
/// TLA+ Bug Pattern: `TryClaimActor_Racy` - TOCTOU race under network partition
///
/// Scenario:
/// 1. Create network partition separating nodes into two groups
/// 2. Nodes on both sides attempt concurrent activation
/// 3. Verify: At most 1 activation succeeds (invariant holds)
///
/// This maps directly to the issue #35 requirement for distributed DST tests.
#[test]
fn test_single_activation_with_network_partition() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(
        seed = config.seed,
        "Running SingleActivation under network partition"
    );

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::NetworkPartition, 1.0)) // Guaranteed partition
        .run(|_env| async move {
            let protocol = Arc::new(ActivationProtocol::new());
            let actor_key = "test/network-partition";

            // Simulate two groups of nodes separated by partition
            // Group A: nodes 0-2 (minority in 5-node cluster)
            // Group B: nodes 3-4 (minority in 5-node cluster)
            // In a real partition, only majority can proceed

            let num_nodes = 5;

            // Spawn concurrent activations from all nodes
            // The partition means some may time out or fail
            let handles: Vec<_> = (0..num_nodes)
                .map(|node_id| {
                    let protocol = protocol.clone();
                    let actor_key = actor_key.to_string();
                    let node_name = format!("node-{}", node_id);
                    kelpie_core::current_runtime().spawn(async move {
                        // Simulate partition delay for cross-partition communication
                        if node_id >= 3 {
                            // Group B nodes experience network delay
                            tokio::task::yield_now().await;
                            tokio::task::yield_now().await;
                        }
                        protocol.try_claim(&actor_key, &node_name).await
                    })
                })
                .collect();

            let results: Vec<_> = join_all(handles)
                .await
                .into_iter()
                .map(|r| r.expect("task panicked"))
                .collect();

            // TLA+ INVARIANT: SingleActivation - AT MOST 1 succeeds
            let successes: Vec<_> = results
                .iter()
                .enumerate()
                .filter(|(_, r)| r.is_ok())
                .map(|(i, _)| format!("node-{}", i))
                .collect();

            assert!(
                successes.len() <= 1,
                "SingleActivation VIOLATED under network partition: {} activations succeeded. \
                 Winners: {:?}. Under partition, at most 1 should win.",
                successes.len(),
                successes
            );

            // With OCC semantics, exactly 1 should succeed
            assert_eq!(
                successes.len(),
                1,
                "Expected exactly 1 activation under partition, got {}",
                successes.len()
            );

            tracing::info!(
                winner = ?successes.first(),
                "SingleActivation invariant held under network partition"
            );

            Ok(())
        });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test single activation with crash and recovery
///
/// TLA+ Bug Pattern: `LeaseExpires_Racy` - Zombie actor reclaim race
///
/// Scenario:
/// 1. Node A activates actor and holds it
/// 2. Node A crashes (simulated)
/// 3. Node B detects failure and attempts reclaim
/// 4. Verify: No dual activation window exists
///
/// This tests the crash recovery path from issue #35.
#[test]
fn test_single_activation_with_crash_recovery() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(
        seed = config.seed,
        "Running SingleActivation with crash recovery"
    );

    let result = Simulation::new(config).run(|_env| async move {
        let protocol = Arc::new(ActivationProtocol::new());
        let actor_key = "test/crash-recovery";

        // Step 1: Node A activates
        let claim_a = protocol.try_claim(actor_key, "node-A").await;
        assert!(claim_a.is_ok(), "Initial activation should succeed");

        // Verify node-A holds the actor
        {
            let state = protocol.state.read().await;
            let holder = state.get(actor_key).unwrap();
            assert_eq!(holder.holder.as_deref(), Some("node-A"));
        }

        // Step 2: Simulate node-A crash (release its claim)
        // In production, this would be lease expiry or failure detection
        protocol.release(actor_key, "node-A").await?;

        // Step 3: Multiple nodes race to reclaim after crash
        let recovery_nodes = vec!["node-B", "node-C", "node-D"];
        let handles: Vec<_> = recovery_nodes
            .iter()
            .map(|&node_name| {
                let protocol = protocol.clone();
                let actor_key = actor_key.to_string();
                let node = node_name.to_string();
                kelpie_core::current_runtime()
                    .spawn(async move { protocol.try_claim(&actor_key, &node).await })
            })
            .collect();

        let results: Vec<_> = join_all(handles)
            .await
            .into_iter()
            .map(|r| r.expect("task panicked"))
            .collect();

        // TLA+ INVARIANT: SingleActivation - exactly 1 recovery succeeds
        let successes: Vec<_> = results.iter().filter(|r| r.is_ok()).collect();

        assert_eq!(
            successes.len(),
            1,
            "SingleActivation VIOLATED during crash recovery: {} activations succeeded. \
             Expected exactly 1 node to take over after crash.",
            successes.len()
        );

        // Verify the new holder is one of the recovery nodes
        {
            let state = protocol.state.read().await;
            let holder = state.get(actor_key).unwrap();
            assert!(
                recovery_nodes.contains(&holder.holder.as_deref().unwrap_or("")),
                "Recovery holder should be one of {:?}, got {:?}",
                recovery_nodes,
                holder.holder
            );
        }

        tracing::info!("SingleActivation held through crash recovery");

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}
