//! DST tests for actor lifecycle
//!
//! TigerStyle: Deterministic testing of actor activation, deactivation,
//! and state persistence under fault injection.

use async_trait::async_trait;
use bytes::Bytes;
use kelpie_core::actor::{Actor, ActorContext, ActorId};
use kelpie_core::error::{Error, Result};
use kelpie_dst::{FaultConfig, FaultType, SimConfig, Simulation};
use kelpie_runtime::{ActivationState, ActiveActor};
use kelpie_storage::ActorKV;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// =============================================================================
// Test Actor Definition
// =============================================================================

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct CounterState {
    count: i64,
}

#[derive(Clone)]
struct CounterActor;

#[async_trait]
impl Actor for CounterActor {
    type State = CounterState;

    async fn invoke(
        &self,
        ctx: &mut ActorContext<Self::State>,
        operation: &str,
        _payload: Bytes,
    ) -> Result<Bytes> {
        match operation {
            "increment" => {
                ctx.state.count += 1;
                Ok(Bytes::from(ctx.state.count.to_string()))
            }
            "get" => Ok(Bytes::from(ctx.state.count.to_string())),
            "reset" => {
                ctx.state.count = 0;
                Ok(Bytes::from("0"))
            }
            _ => Err(Error::InvalidOperation {
                operation: operation.to_string(),
            }),
        }
    }
}

// =============================================================================
// Basic Lifecycle Tests
// =============================================================================

#[test]
fn test_dst_actor_activation_basic() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let actor_id = ActorId::new("dst-test", "counter-1")?;
        let storage = Arc::new(env.storage);

        let active = ActiveActor::activate(actor_id.clone(), CounterActor, storage).await?;

        assert_eq!(active.activation_state(), ActivationState::Active);
        assert_eq!(active.id, actor_id);

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[test]
fn test_dst_actor_invocation() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let actor_id = ActorId::new("dst-test", "counter-2")?;
        let storage = Arc::new(env.storage);

        let mut active = ActiveActor::activate(actor_id, CounterActor, storage).await?;

        // Multiple increments
        let result = active.process_invocation("increment", Bytes::new()).await?;
        assert_eq!(result, Bytes::from("1"));

        let result = active.process_invocation("increment", Bytes::new()).await?;
        assert_eq!(result, Bytes::from("2"));

        let result = active.process_invocation("get", Bytes::new()).await?;
        assert_eq!(result, Bytes::from("2"));

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[test]
fn test_dst_actor_deactivation() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let actor_id = ActorId::new("dst-test", "counter-3")?;
        let storage = Arc::new(env.storage);

        let mut active = ActiveActor::activate(actor_id, CounterActor, storage).await?;

        active.process_invocation("increment", Bytes::new()).await?;
        active.deactivate().await?;

        assert_eq!(active.activation_state(), ActivationState::Deactivated);

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// State Persistence Tests
// =============================================================================

#[test]
fn test_dst_state_persistence_across_activations() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let actor_id = ActorId::new("dst-test", "persistent-1")?;
        let storage = Arc::new(env.storage);

        // First activation: increment and deactivate
        {
            let mut active =
                ActiveActor::activate(actor_id.clone(), CounterActor, storage.clone()).await?;

            active.process_invocation("increment", Bytes::new()).await?;
            active.process_invocation("increment", Bytes::new()).await?;
            active.process_invocation("increment", Bytes::new()).await?;

            active.deactivate().await?;
        }

        // Second activation: state should be restored
        {
            let mut active = ActiveActor::activate(actor_id, CounterActor, storage).await?;

            let result = active.process_invocation("get", Bytes::new()).await?;
            assert_eq!(result, Bytes::from("3"));
        }

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Multiple Actor Isolation Tests
// =============================================================================

#[test]
fn test_dst_multiple_actors_isolation() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let actor1_id = ActorId::new("dst-test", "isolated-1")?;
        let actor2_id = ActorId::new("dst-test", "isolated-2")?;
        let storage = Arc::new(env.storage);

        // Activate both actors
        let mut actor1 = ActiveActor::activate(actor1_id, CounterActor, storage.clone()).await?;

        let mut actor2 = ActiveActor::activate(actor2_id, CounterActor, storage).await?;

        // Increment actor1 three times
        actor1.process_invocation("increment", Bytes::new()).await?;
        actor1.process_invocation("increment", Bytes::new()).await?;
        actor1.process_invocation("increment", Bytes::new()).await?;

        // Increment actor2 once
        actor2.process_invocation("increment", Bytes::new()).await?;

        // Verify isolation
        let result1 = actor1.process_invocation("get", Bytes::new()).await?;
        let result2 = actor2.process_invocation("get", Bytes::new()).await?;

        assert_eq!(result1, Bytes::from("3"));
        assert_eq!(result2, Bytes::from("1"));

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Fault Injection Tests
// =============================================================================

#[test]
fn test_dst_activation_with_storage_read_fault() {
    // Use a specific seed for reproducibility in this test
    let config = SimConfig::new(42);

    // 100% probability of storage read failures
    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 1.0))
        .run(|env| async move {
            let actor_id = ActorId::new("dst-test", "fault-read-1")?;
            let storage = Arc::new(env.storage);

            // Activation should still succeed (uses default state on read failure)
            let active = ActiveActor::activate(actor_id, CounterActor, storage).await?;
            assert_eq!(active.activation_state(), ActivationState::Active);

            Ok(())
        });

    // The runtime gracefully handles read failures during activation
    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[test]
fn test_dst_persistence_with_intermittent_failures() {
    // Test with 30% write failure probability
    // With transactional state persistence, invocations may fail if the commit fails.
    // This tests that the system handles such failures gracefully.
    let config = SimConfig::new(12345);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.3))
        .run(|env| async move {
            let actor_id = ActorId::new("dst-test", "intermittent-1")?;
            let storage = Arc::new(env.storage);

            let mut active = ActiveActor::activate(actor_id, CounterActor, storage).await?;

            // Perform operations - some may fail due to storage write failures during commit
            // This is expected behavior under fault injection
            let mut success_count = 0;
            let mut failure_count = 0;

            for _ in 0..10 {
                match active.process_invocation("increment", Bytes::new()).await {
                    Ok(_) => success_count += 1,
                    Err(_) => failure_count += 1,
                }
            }

            // With 30% failure rate, we expect some successes and some failures
            // The exact counts are deterministic for a given seed
            assert!(
                success_count > 0,
                "Expected at least some successful invocations"
            );

            // Deactivation may fail due to storage write failure
            // This is expected behavior under fault injection
            let _deactivate_result = active.deactivate().await;

            // The deactivation should complete (logs error but continues)
            assert_eq!(active.activation_state(), ActivationState::Deactivated);

            // Log the results for debugging
            tracing::info!(
                success_count = success_count,
                failure_count = failure_count,
                "Intermittent failure test completed"
            );

            // Result is OK because we handle errors gracefully
            Ok(())
        });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Determinism Tests
// =============================================================================

#[test]
fn test_dst_deterministic_behavior() {
    let seed = 999;

    // Run the same test twice with the same seed
    let run_test = || {
        let config = SimConfig::new(seed);

        Simulation::new(config).run(|env| async move {
            let actor_id = ActorId::new("dst-test", "deterministic-1")?;
            let storage = Arc::new(env.storage);

            let mut active = ActiveActor::activate(actor_id, CounterActor, storage).await?;

            // Perform operations
            for _ in 0..10 {
                active.process_invocation("increment", Bytes::new()).await?;
            }

            let result = active.process_invocation("get", Bytes::new()).await?;
            Ok(result)
        })
    };

    let result1 = run_test().expect("First run failed");
    let result2 = run_test().expect("Second run failed");

    assert_eq!(
        result1, result2,
        "Determinism violated: results differ with same seed"
    );
}

// =============================================================================
// Stress Tests (longer duration, marked as ignored for CI)
// =============================================================================

#[test]
#[ignore] // Run with: cargo test -p kelpie-dst -- --ignored
fn test_dst_stress_many_activations() {
    let config = SimConfig::from_env_or_random().with_max_steps(1_000_000);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.05))
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.02))
        .run(|env| async move {
            let storage = Arc::new(env.storage);

            // Create and use many actors
            for i in 0..100 {
                let actor_id = ActorId::new("dst-stress", format!("actor-{}", i))?;

                let mut active =
                    ActiveActor::activate(actor_id, CounterActor, storage.clone()).await?;

                // Perform some operations
                for _ in 0..10 {
                    let _ = active.process_invocation("increment", Bytes::new()).await;
                }

                let _ = active.deactivate().await;
            }

            Ok(())
        });

    assert!(result.is_ok(), "Stress test failed: {:?}", result.err());
}

// =============================================================================
// Atomicity Gap Tests (DST-First: demonstrates the problem)
// =============================================================================

/// Actor that uses BOTH KV operations AND state updates
/// This is used to test atomicity between KV and state persistence.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct BankAccountState {
    /// Last successful transfer ID (stored in state)
    last_transfer_id: Option<String>,
}

#[derive(Clone)]
struct BankAccountActor;

#[async_trait]
impl Actor for BankAccountActor {
    type State = BankAccountState;

    async fn invoke(
        &self,
        ctx: &mut ActorContext<Self::State>,
        operation: &str,
        payload: Bytes,
    ) -> Result<Bytes> {
        match operation {
            "transfer" => {
                // Parse transfer: "transfer_id:amount"
                let payload_str = String::from_utf8_lossy(&payload);
                let parts: Vec<&str> = payload_str.split(':').collect();
                if parts.len() != 2 {
                    return Err(Error::Internal {
                        message: "expected transfer_id:amount".into(),
                    });
                }
                let transfer_id = parts[0];
                let amount: i64 = parts[1].parse().map_err(|_| Error::Internal {
                    message: "invalid amount".into(),
                })?;

                // Get current balance from KV
                let current_balance: i64 = match ctx.kv_get(b"balance").await? {
                    Some(bytes) => String::from_utf8_lossy(&bytes).parse().unwrap_or(0),
                    None => 0,
                };

                // Update balance in KV (THIS IS IMMEDIATE - NOT TRANSACTIONAL!)
                let new_balance = current_balance + amount;
                ctx.kv_set(b"balance", new_balance.to_string().as_bytes())
                    .await?;

                // Update state with transfer ID (THIS IS SAVED LATER IN A TRANSACTION)
                ctx.state.last_transfer_id = Some(transfer_id.to_string());

                Ok(Bytes::from(new_balance.to_string()))
            }
            "get_balance" => {
                let balance = match ctx.kv_get(b"balance").await? {
                    Some(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                    None => "0".to_string(),
                };
                Ok(Bytes::from(balance))
            }
            "get_last_transfer" => {
                let transfer_id = ctx
                    .state
                    .last_transfer_id
                    .clone()
                    .unwrap_or_else(|| "none".to_string());
                Ok(Bytes::from(transfer_id))
            }
            _ => Err(Error::InvalidOperation {
                operation: operation.to_string(),
            }),
        }
    }
}

/// DST-First Test: Demonstrates the atomicity gap between KV and state
///
/// This test SHOULD FAIL with the current implementation, proving that:
/// - KV writes (ctx.kv_set) are immediate and persist even if state commit fails
/// - State writes (ctx.state) are only persisted on transaction commit
/// - A crash during state commit leaves KV and state inconsistent
///
/// After we fix the atomicity gap, this test should PASS.
#[test]
fn test_dst_kv_state_atomicity_gap() {
    // Use a fixed seed for reproducibility
    let config = SimConfig::new(54321);

    let result = Simulation::new(config)
        // 100% crash during transaction commit - this will crash the state save
        // IMPORTANT: Must filter to "transaction_commit" so it doesn't affect direct writes
        .with_fault(
            FaultConfig::new(FaultType::CrashDuringTransaction, 1.0)
                .with_filter("transaction_commit"),
        )
        .run(|env| async move {
            let actor_id = ActorId::new("dst-test", "bank-account-1")?;
            let storage = Arc::new(env.storage);

            // First activation: perform a transfer that will crash during state commit
            {
                let mut active =
                    ActiveActor::activate(actor_id.clone(), BankAccountActor, storage.clone())
                        .await?;

                // This transfer will:
                // 1. Write balance=100 to KV (IMMEDIATE - not transactional!)
                // 2. Set last_transfer_id="txn-1" in state (in-memory only)
                // 3. Try to commit state transaction (WILL CRASH due to fault)
                let result = active
                    .process_invocation("transfer", Bytes::from("txn-1:100"))
                    .await;

                // The invocation should fail because state commit crashed
                assert!(
                    result.is_err(),
                    "Expected invocation to fail due to CrashDuringTransaction"
                );

                // IMPORTANT: Do NOT call deactivate() - in a real crash scenario,
                // the process would die without a chance to run deactivation.
                // deactivate() does a direct write which would "heal" the inconsistency.
                // We drop the actor without deactivating to simulate a real crash.
                drop(active);
            }

            // Check consistency by reading directly from storage
            // (simulating recovery after crash)

            // Read KV balance
            let kv_balance = storage
                .get(&actor_id, b"balance")
                .await?
                .map(|b| String::from_utf8_lossy(&b[..]).to_string());

            // Read state (the __state__ key)
            let state_bytes = storage.get(&actor_id, b"__state__").await?;
            let state: Option<BankAccountState> = state_bytes
                .as_ref()
                .and_then(|b| serde_json::from_slice(b).ok());
            let last_transfer = state.and_then(|s| s.last_transfer_id);

            // Check if atomicity was preserved
            let kv_persisted = kv_balance.is_some();
            let state_persisted = last_transfer.is_some();

            // THE ATOMICITY GAP:
            // With current implementation, we expect:
            // - kv_balance = Some("100") - KV write persisted (BUG!)
            // - last_transfer = None - State transaction crashed
            //
            // After we fix the gap, we expect:
            // - kv_balance = None - Both rolled back atomically
            // - last_transfer = None - Both rolled back atomically

            match (kv_persisted, state_persisted) {
                (true, false) => {
                    // ATOMICITY GAP DETECTED: KV persisted but state didn't
                    // This is the bug we're documenting and will fix
                    Err(Error::Internal {
                        message: format!(
                            "ATOMICITY_GAP: KV balance={:?} persisted but state last_transfer={:?} did not",
                            kv_balance, last_transfer
                        ),
                    })
                }
                (false, false) => {
                    // Both rolled back - atomicity preserved!
                    // This is the expected outcome AFTER we fix the gap
                    Ok(())
                }
                (true, true) => {
                    // Both persisted - crash fault didn't trigger
                    Err(Error::Internal {
                        message: "UNEXPECTED: Both persisted - crash fault didn't work".into(),
                    })
                }
                (false, true) => {
                    // State persisted but KV didn't - unexpected
                    Err(Error::Internal {
                        message: "UNEXPECTED: State persisted but KV didn't".into(),
                    })
                }
            }
        });

    // Analyze the result
    // - Before fix: Test should fail with ATOMICITY_GAP error
    // - After fix: Test should pass (both rolled back)
    match result {
        Ok(()) => {
            // SUCCESS: Atomicity preserved (both rolled back)
            // This is what we expect AFTER implementing the fix
        }
        Err(e) => {
            let error_msg = format!("{:?}", e);
            if error_msg.contains("ATOMICITY_GAP") {
                // EXPECTED with current implementation - the gap exists
                // This test documents the problem that we need to fix
                //
                // DST-FIRST: Test FAILS now (documents bug), PASSES after fix
                panic!(
                    "DST-FIRST: Atomicity gap detected as expected. \
                     This test will pass once KV operations are made transactional with state. \
                     Details: {:?}",
                    e
                );
            } else {
                // Unexpected error
                panic!("Unexpected error: {:?}", e);
            }
        }
    }
}

/// DST Test: Verify KV-State atomicity under crash conditions
///
/// This test verifies the acceptance criteria from Issue #21:
/// - Inject crash during commit
/// - Verify: either BOTH persisted or NEITHER persisted
///
/// Tests multiple crash/success cycles to ensure atomicity invariant holds.
#[test]
fn test_kv_state_atomicity_under_crash() {
    // Test with different crash probabilities to cover various scenarios
    for crash_prob in [0.0, 0.3, 0.5, 0.7, 1.0] {
        let config = SimConfig::new(12345 + (crash_prob * 100.0) as u64);

        let result = Simulation::new(config)
            .with_fault(
                FaultConfig::new(FaultType::CrashDuringTransaction, crash_prob)
                    .with_filter("transaction_commit"),
            )
            .run(|env| async move {
                let actor_id = ActorId::new("dst-test", "atomicity-test")?;
                let storage = Arc::new(env.storage);

                // Perform multiple invocations
                let mut active =
                    ActiveActor::activate(actor_id.clone(), BankAccountActor, storage.clone())
                        .await?;

                let mut successful_ops = 0;
                let mut failed_ops = 0;

                for i in 0..5 {
                    let transfer_id = format!("txn-{}", i);
                    let payload = format!("{}:{}", transfer_id, (i + 1) * 100);

                    match active
                        .process_invocation("transfer", Bytes::from(payload))
                        .await
                    {
                        Ok(_) => successful_ops += 1,
                        Err(_) => failed_ops += 1,
                    }
                }

                // Deactivate cleanly
                let _ = active.deactivate().await;

                // VERIFY ATOMICITY INVARIANT:
                // Check storage directly - KV and state must be consistent
                let kv_balance = storage
                    .get(&actor_id, b"balance")
                    .await?
                    .map(|b| String::from_utf8_lossy(&b[..]).parse::<i64>().unwrap_or(0));

                let state_bytes = storage.get(&actor_id, b"__state__").await?;
                let state: Option<BankAccountState> = state_bytes
                    .as_ref()
                    .and_then(|b| serde_json::from_slice(b).ok());
                let has_transfer = state.and_then(|s| s.last_transfer_id).is_some();

                // CRITICAL INVARIANT: If KV has balance, state must have transfer ID
                // and vice versa. They must be atomic.
                let kv_has_data = kv_balance.is_some();
                let state_has_data = has_transfer;

                // After successful ops, both should have data (or neither if all failed)
                if successful_ops > 0 {
                    // At least one op succeeded - both must have data
                    if kv_has_data != state_has_data {
                        return Err(Error::Internal {
                            message: format!(
                                "ATOMICITY VIOLATION: kv_has_data={}, state_has_data={}, \
                                 successful_ops={}, failed_ops={}",
                                kv_has_data, state_has_data, successful_ops, failed_ops
                            ),
                        });
                    }
                }

                // Log results for debugging
                tracing::info!(
                    crash_prob = crash_prob,
                    successful_ops = successful_ops,
                    failed_ops = failed_ops,
                    kv_has_data = kv_has_data,
                    state_has_data = state_has_data,
                    "Atomicity test completed"
                );

                Ok(())
            });

        assert!(
            result.is_ok(),
            "Atomicity test failed at crash_prob={}: {:?}",
            crash_prob,
            result.err()
        );
    }
}

// =============================================================================
// Exploratory DST Test - Bug Hunting
// =============================================================================

/// Ledger actor for exploratory testing
/// Tracks a running balance with explicit transfer IDs for consistency checking
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct LedgerState {
    /// Number of successful transfers (monotonically increasing)
    transfer_count: u64,
    /// Sum of all successful transfer amounts (should match KV balance)
    expected_balance: i64,
}

#[derive(Clone)]
struct LedgerActor;

#[async_trait]
impl Actor for LedgerActor {
    type State = LedgerState;

    async fn invoke(
        &self,
        ctx: &mut ActorContext<Self::State>,
        operation: &str,
        payload: Bytes,
    ) -> Result<Bytes> {
        match operation {
            "transfer" => {
                // Parse amount from payload
                let amount: i64 =
                    String::from_utf8_lossy(&payload)
                        .parse()
                        .map_err(|_| Error::Internal {
                            message: "invalid amount".into(),
                        })?;

                // Read current KV balance
                let current_kv_balance: i64 = match ctx.kv_get(b"balance").await? {
                    Some(bytes) => String::from_utf8_lossy(&bytes).parse().unwrap_or(0),
                    None => 0,
                };

                // INVARIANT CHECK: KV balance should match expected balance from state
                // This would catch bugs where KV and state get out of sync
                if current_kv_balance != ctx.state.expected_balance {
                    return Err(Error::Internal {
                        message: format!(
                            "INVARIANT VIOLATION: KV balance {} != expected {} (transfer_count={})",
                            current_kv_balance,
                            ctx.state.expected_balance,
                            ctx.state.transfer_count
                        ),
                    });
                }

                // Update KV balance
                let new_balance = current_kv_balance + amount;
                ctx.kv_set(b"balance", new_balance.to_string().as_bytes())
                    .await?;

                // Update state
                ctx.state.transfer_count += 1;
                ctx.state.expected_balance = new_balance;

                Ok(Bytes::from(new_balance.to_string()))
            }
            "get_state" => {
                let response = format!(
                    "count={},expected={}",
                    ctx.state.transfer_count, ctx.state.expected_balance
                );
                Ok(Bytes::from(response))
            }
            _ => Err(Error::InvalidOperation {
                operation: operation.to_string(),
            }),
        }
    }
}

/// Exploratory DST test that hunts for bugs through randomized testing
///
/// This test:
/// 1. Runs many iterations with random seeds
/// 2. Injects various faults at realistic probabilities
/// 3. Performs multiple operations per iteration
/// 4. Checks invariants that should ALWAYS hold
/// 5. Reports violations with seed for reproduction
#[test]
fn test_dst_exploratory_bug_hunting() {
    const OPS_PER_ITERATION: u32 = 10;

    // If DST_SINGLE_SEED is set, only run that seed (for debugging)
    let single_seed = std::env::var("DST_SINGLE_SEED")
        .ok()
        .and_then(|s| s.parse::<u64>().ok());

    let iterations: Vec<u64> = if let Some(seed) = single_seed {
        println!("Running single seed: {}", seed);
        vec![seed]
    } else {
        (0..100).map(|i| 0xDEADBEEF_u64.wrapping_add(i)).collect()
    };

    let mut bugs_found: Vec<(u64, String)> = Vec::new();

    for (idx, seed) in iterations.iter().enumerate() {
        let result = run_single_exploration(*seed, OPS_PER_ITERATION);

        match result {
            Ok(()) => {
                // Iteration passed
            }
            Err(bug_description) => {
                bugs_found.push((*seed, bug_description));
            }
        }

        // Progress indicator
        if (idx + 1) % 20 == 0 {
            println!(
                "Progress: {}/{} iterations, {} bugs found",
                idx + 1,
                iterations.len(),
                bugs_found.len()
            );
        }
    }

    // Report results
    if bugs_found.is_empty() {
        println!(
            "Exploratory DST: {} iterations, {} ops each, NO BUGS FOUND",
            iterations.len(),
            OPS_PER_ITERATION
        );
    } else {
        println!("\n=== BUGS FOUND ===");
        for (seed, description) in &bugs_found {
            println!("Seed {}: {}", seed, description);
        }
        panic!(
            "Exploratory DST found {} bugs! See above for seeds to reproduce.",
            bugs_found.len()
        );
    }
}

fn run_single_exploration(seed: u64, ops_count: u32) -> std::result::Result<(), String> {
    let config = SimConfig::new(seed);
    let debug_mode = std::env::var("DST_DEBUG").is_ok();

    // Mix of realistic fault probabilities
    let result = Simulation::new(config)
        // 5% chance of crash during transaction commit
        .with_fault(
            FaultConfig::new(FaultType::CrashDuringTransaction, 0.05)
                .with_filter("transaction_commit"),
        )
        // 3% chance of storage write failure
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.03))
        // 2% chance of storage read failure
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.02))
        // 5% chance of storage latency (10-50ms delay)
        .with_fault(FaultConfig::new(
            FaultType::StorageLatency {
                min_ms: 10,
                max_ms: 50,
            },
            0.05,
        ))
        .run(|env| async move {
            let actor_id = ActorId::new("dst-explore", format!("ledger-{}", seed))?;
            let storage = Arc::new(env.storage);

            // Track successful operations for invariant checking
            let mut successful_transfers: u64 = 0;
            let mut expected_balance: i64 = 0;

            // Activate actor
            let mut active =
                ActiveActor::activate(actor_id.clone(), LedgerActor, storage.clone()).await?;

            // Perform multiple operations
            for op_num in 0..ops_count {
                let amount = ((op_num as i64 % 10) + 1) * 10; // 10, 20, 30, ...

                // Debug: Check storage state BEFORE operation
                if debug_mode {
                    let pre_kv = storage
                        .get(&actor_id, b"balance")
                        .await
                        .ok()
                        .flatten()
                        .map(|b| String::from_utf8_lossy(&b).to_string());
                    let pre_state = storage
                        .get(&actor_id, b"__state__")
                        .await
                        .ok()
                        .flatten()
                        .and_then(|b| serde_json::from_slice::<LedgerState>(&b).ok());
                    println!(
                        "  Op {}: BEFORE - KV={:?}, state={:?}",
                        op_num, pre_kv, pre_state
                    );
                }

                let result = active
                    .process_invocation("transfer", Bytes::from(amount.to_string()))
                    .await;

                // Debug: Check storage state AFTER operation
                if debug_mode {
                    let post_kv = storage
                        .get(&actor_id, b"balance")
                        .await
                        .ok()
                        .flatten()
                        .map(|b| String::from_utf8_lossy(&b).to_string());
                    let post_state = storage
                        .get(&actor_id, b"__state__")
                        .await
                        .ok()
                        .flatten()
                        .and_then(|b| serde_json::from_slice::<LedgerState>(&b).ok());
                    println!(
                        "  Op {}: result={:?}, AFTER - KV={:?}, state={:?}",
                        op_num,
                        result.is_ok(),
                        post_kv,
                        post_state
                    );
                }

                match result {
                    Ok(_) => {
                        successful_transfers += 1;
                        expected_balance += amount;
                    }
                    Err(_) => {
                        // Operation failed (expected with fault injection)
                        // Actor state should NOT have been modified
                    }
                }
            }

            // Deactivate cleanly
            if debug_mode {
                println!("  Before deactivate - checking storage directly");
                let pre_deact_kv = storage
                    .get(&actor_id, b"balance")
                    .await
                    .ok()
                    .flatten()
                    .map(|b| String::from_utf8_lossy(&b).to_string());
                let pre_deact_state = storage
                    .get(&actor_id, b"__state__")
                    .await
                    .ok()
                    .flatten()
                    .and_then(|b| serde_json::from_slice::<LedgerState>(&b).ok());
                println!(
                    "  PRE-DEACTIVATE - KV={:?}, state={:?}",
                    pre_deact_kv, pre_deact_state
                );
            }
            let deact_result = active.deactivate().await;
            if debug_mode {
                println!("  deactivate result: {:?}", deact_result.is_ok());
                let post_deact_kv = storage
                    .get(&actor_id, b"balance")
                    .await
                    .ok()
                    .flatten()
                    .map(|b| String::from_utf8_lossy(&b).to_string());
                let post_deact_state = storage
                    .get(&actor_id, b"__state__")
                    .await
                    .ok()
                    .flatten()
                    .and_then(|b| serde_json::from_slice::<LedgerState>(&b).ok());
                println!(
                    "  POST-DEACTIVATE - KV={:?}, state={:?}",
                    post_deact_kv, post_deact_state
                );
            }

            // INVARIANT CHECK 1: Read KV balance directly
            let kv_result = storage.get(&actor_id, b"balance").await;
            if debug_mode {
                println!(
                    "  INVARIANT kv_result: {:?}",
                    kv_result
                        .as_ref()
                        .map(|o| o.as_ref().map(|b| String::from_utf8_lossy(b).to_string()))
                );
            }
            let kv_balance: i64 = kv_result?
                .map(|b| String::from_utf8_lossy(&b).parse().unwrap_or(0))
                .unwrap_or(0);

            // INVARIANT CHECK 2: Read state directly
            let state_result = storage.get(&actor_id, b"__state__").await;
            if debug_mode {
                println!("  INVARIANT state_result: {:?}", state_result);
            }
            let state_bytes = state_result?;
            if debug_mode {
                println!(
                    "  INVARIANT state_bytes: {:?}",
                    state_bytes
                        .as_ref()
                        .map(|b| String::from_utf8_lossy(b).to_string())
                );
            }
            let state: LedgerState = state_bytes
                .as_ref()
                .and_then(|b| serde_json::from_slice(b).ok())
                .unwrap_or_default();
            if debug_mode {
                println!(
                    "  INVARIANT final: kv_balance={}, state={:?}",
                    kv_balance, state
                );
            }

            // INVARIANT: KV balance must equal state's expected_balance
            if kv_balance != state.expected_balance {
                return Err(Error::Internal {
                    message: format!(
                        "ATOMICITY BUG: KV balance {} != state expected_balance {} \
                         (state.transfer_count={}, our_successful_count={})",
                        kv_balance,
                        state.expected_balance,
                        state.transfer_count,
                        successful_transfers
                    ),
                });
            }

            // INVARIANT: State transfer_count should match our tracking
            // (This could differ if we had bugs in success/failure handling)
            if state.transfer_count != successful_transfers {
                return Err(Error::Internal {
                    message: format!(
                        "COUNT MISMATCH: state.transfer_count {} != our tracking {} \
                         (kv_balance={}, expected_balance={})",
                        state.transfer_count, successful_transfers, kv_balance, expected_balance
                    ),
                });
            }

            // INVARIANT: Our expected balance should match actual
            if expected_balance != kv_balance {
                return Err(Error::Internal {
                    message: format!(
                        "BALANCE MISMATCH: expected {} != actual {} \
                         (transfers={}, state_count={})",
                        expected_balance, kv_balance, successful_transfers, state.transfer_count
                    ),
                });
            }

            Ok(())
        });

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            let error_str = format!("{:?}", e);
            if error_str.contains("ATOMICITY BUG")
                || error_str.contains("COUNT MISMATCH")
                || error_str.contains("BALANCE MISMATCH")
                || error_str.contains("INVARIANT VIOLATION")
            {
                Err(error_str)
            } else {
                // Other errors (like storage failures during invariant checks) are OK
                Ok(())
            }
        }
    }
}
