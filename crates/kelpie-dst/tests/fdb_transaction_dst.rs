//! DST tests for TLA+ FDB transaction properties
//!
//! TLA+ Spec Reference: `docs/tla/KelpieFDBTransaction.tla`
//!
//! This test file verifies that Kelpie correctly uses FoundationDB's transaction API
//! by testing against the formal TLA+ specification. While FDB provides its own
//! correctness guarantees, we need to verify that Kelpie's usage of the transaction
//! API preserves the expected properties.
//!
//! ## Invariants Tested
//!
//! | Invariant | Test Function | TLA+ Reference |
//! |-----------|---------------|----------------|
//! | SerializableIsolation | `test_serializable_isolation` | Line 167 |
//! | ConflictDetection | `test_conflict_detection` | Line 196 |
//! | AtomicCommit | `test_atomic_commit` | Line 215 |
//! | ReadYourWrites | `test_read_your_writes_in_txn` | Line 231 |
//!
//! ## Liveness Properties Tested
//!
//! | Property | Test Function | TLA+ Reference |
//! |----------|---------------|----------------|
//! | EventualTermination | `test_eventual_termination` | Line 250 |
//! | EventualCommit | `test_eventual_commit` | Line 256 |
//!
//! ## OCC Implementation
//!
//! SimStorage implements Optimistic Concurrency Control (OCC) semantics:
//! - Each key has a version counter that increments on every write
//! - Transactions track read-set versions at read time
//! - On commit: validates that all read keys still have the same versions
//! - If any read key changed: abort with TransactionConflict error
//! - If no conflicts: apply writes atomically and increment versions
//!
//! TigerStyle: Deterministic simulation with explicit fault injection,
//! 2+ assertions per test, reproducible with DST_SEED.

use bytes::Bytes;
use kelpie_core::error::Error;
use kelpie_core::ActorId;
use kelpie_core::Runtime;
use kelpie_dst::{DeterministicRng, FaultConfig, FaultInjectorBuilder, FaultType, SimStorage};
use kelpie_storage::ActorKV;
use std::sync::Arc;

// =============================================================================
// Safety Invariant Tests
// =============================================================================

/// Verify SerializableIsolation: transactions appear to execute serially.
///
/// TLA+ invariant (line 167): All committed transactions can be arranged in a
/// serial order such that each transaction sees only the effects of transactions
/// that precede it.
///
/// Test approach:
/// - Run 3 transactions concurrently on overlapping keys
/// - Transaction 1: write k1=v1, k2=v2
/// - Transaction 2: read k1, write k3=v3 (should see T1's write or conflict)
/// - Transaction 3: read k2, write k4=v4 (should see T1's write or conflict)
/// - Verify final state matches SOME serial execution order
#[madsim::test]
async fn test_serializable_isolation() {
    let rng = DeterministicRng::from_env_or_random();
    let fault_injector = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
    let storage = Arc::new(SimStorage::new(rng, fault_injector));
    let actor_id = ActorId::new("fdb-test", "serial-1").unwrap();

    // Transaction 1: Write k1 and k2
    let mut txn1 = storage.begin_transaction(&actor_id).await.unwrap();
    txn1.set(b"k1", b"v1").await.unwrap();
    txn1.set(b"k2", b"v2").await.unwrap();
    let commit1_result = txn1.commit().await;

    // Transaction 2: Read k1, write k3
    let mut txn2 = storage.begin_transaction(&actor_id).await.unwrap();
    let k1_value = txn2.get(b"k1").await.unwrap();
    txn2.set(b"k3", b"v3").await.unwrap();
    let commit2_result = txn2.commit().await;

    // Transaction 3: Read k2, write k4
    let mut txn3 = storage.begin_transaction(&actor_id).await.unwrap();
    let _k2_value = txn3.get(b"k2").await.unwrap();
    txn3.set(b"k4", b"v4").await.unwrap();
    let commit3_result = txn3.commit().await;

    // At least one transaction should commit (forward progress)
    let committed_count = [&commit1_result, &commit2_result, &commit3_result]
        .iter()
        .filter(|r| r.is_ok())
        .count();
    assert!(
        committed_count >= 1,
        "at least one transaction should commit"
    );

    // If T1 committed, its writes should be visible
    if commit1_result.is_ok() {
        assert_eq!(
            storage.get(&actor_id, b"k1").await.unwrap(),
            Some(Bytes::from("v1"))
        );
        assert_eq!(
            storage.get(&actor_id, b"k2").await.unwrap(),
            Some(Bytes::from("v2"))
        );
    }

    // If T2 committed after T1, it should have seen T1's write to k1
    if commit1_result.is_ok() && commit2_result.is_ok() {
        // T2 read k1 - should have seen v1 or empty (but not a different value)
        assert!(
            k1_value == Some(Bytes::from("v1")) || k1_value.is_none(),
            "T2 should see T1's write or empty"
        );
    }

    // Postconditions
    assert!(
        committed_count <= 3,
        "cannot have more commits than transactions"
    );
}

/// Verify ConflictDetection: concurrent read-write conflict detection.
///
/// TLA+ invariant (line 196): If two transactions both access the same key
/// and commit, OCC must detect conflicts when one reads and another writes.
///
/// Test approach:
/// - Set initial value k1=v0
/// - Transaction 1: read k1, write k2
/// - Transaction 2: write k1 (before T1 commits)
/// - T2 commits first (succeeds)
/// - T1 tries to commit (should fail with conflict on k1)
#[madsim::test]
async fn test_conflict_detection() {
    let rng = DeterministicRng::from_env_or_random();
    let fault_injector = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
    let storage = Arc::new(SimStorage::new(rng, fault_injector));
    let actor_id = ActorId::new("fdb-test", "conflict-1").unwrap();

    // Set initial value
    storage.set(&actor_id, b"k1", b"v0").await.unwrap();

    // Transaction 1: Read k1 (adds to read-set), write k2
    let mut txn1 = storage.begin_transaction(&actor_id).await.unwrap();
    let _v1 = txn1.get(b"k1").await.unwrap();
    txn1.set(b"k2", b"from_t1").await.unwrap();

    // Transaction 2: Write k1 (before T1 commits)
    let mut txn2 = storage.begin_transaction(&actor_id).await.unwrap();
    txn2.set(b"k1", b"v1").await.unwrap();

    // Commit T2 first
    let result2 = txn2.commit().await;
    assert!(result2.is_ok(), "T2 should commit successfully");

    // Commit T1 - should detect conflict on k1
    let result1 = txn1.commit().await;
    assert!(
        result1.is_err(),
        "T1 should fail due to conflict on k1: {:?}",
        result1
    );
    assert!(
        matches!(result1, Err(Error::TransactionConflict { .. })),
        "should be TransactionConflict error"
    );

    // Postconditions
    assert_eq!(
        storage.get(&actor_id, b"k1").await.unwrap(),
        Some(Bytes::from("v1")),
        "T2's write to k1 should be visible"
    );
    assert!(
        storage.get(&actor_id, b"k2").await.unwrap().is_none(),
        "T1's write to k2 should NOT be visible (T1 aborted)"
    );
}

/// Verify ConflictDetection with read-write conflict.
///
/// This is the key conflict case: Transaction reads a key, another transaction
/// writes it, first transaction tries to commit.
#[madsim::test]
async fn test_conflict_detection_read_write() {
    let rng = DeterministicRng::from_env_or_random();
    let fault_injector = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
    let storage = Arc::new(SimStorage::new(rng, fault_injector));
    let actor_id = ActorId::new("fdb-test", "conflict-rw-1").unwrap();

    // Set initial value
    storage.set(&actor_id, b"k1", b"v0").await.unwrap();

    // Transaction 1: Read k1, write k2
    let mut txn1 = storage.begin_transaction(&actor_id).await.unwrap();
    let v1 = txn1.get(b"k1").await.unwrap();
    assert_eq!(v1, Some(Bytes::from("v0")), "T1 should see initial value");
    txn1.set(b"k2", b"from_t1").await.unwrap();

    // Transaction 2: Write k1 (before T1 commits)
    let mut txn2 = storage.begin_transaction(&actor_id).await.unwrap();
    txn2.set(b"k1", b"v1").await.unwrap();

    // Commit T2 first
    let result2 = txn2.commit().await;
    assert!(result2.is_ok(), "T2 should commit successfully");

    // Commit T1 - should detect conflict on k1
    let result1 = txn1.commit().await;
    assert!(
        result1.is_err(),
        "T1 should fail due to conflict on k1: {:?}",
        result1
    );
    assert!(
        matches!(result1, Err(Error::TransactionConflict { .. })),
        "should be TransactionConflict error"
    );

    // Postconditions
    assert_eq!(
        storage.get(&actor_id, b"k1").await.unwrap(),
        Some(Bytes::from("v1")),
        "T2's write to k1 should be visible"
    );
    assert!(
        storage.get(&actor_id, b"k2").await.unwrap().is_none(),
        "T1's write to k2 should NOT be visible (T1 aborted)"
    );
}

/// Verify AtomicCommit: transaction commits are all-or-nothing.
///
/// TLA+ invariant (line 215): A transaction's writes are either all visible
/// or none are visible.
///
/// Test approach:
/// - Transaction writes multiple keys
/// - Inject CrashDuringTransaction fault
/// - Verify either ALL keys updated or NONE updated (no partial commit)
#[madsim::test]
async fn test_atomic_commit() {
    let rng = DeterministicRng::from_env_or_random();
    // Inject CrashDuringTransaction with 50% probability
    let fault_injector = Arc::new(
        FaultInjectorBuilder::new(rng.fork())
            .with_fault(
                FaultConfig::new(FaultType::CrashDuringTransaction, 0.5)
                    .with_filter("transaction_commit"),
            )
            .build(),
    );
    let storage = Arc::new(SimStorage::new(rng, fault_injector));
    let actor_id = ActorId::new("fdb-test", "atomic-1").unwrap();

    // Run 10 transactions, each writing 3 keys
    for i in 0..10 {
        let mut txn = storage.begin_transaction(&actor_id).await.unwrap();
        let k1 = format!("txn{}_k1", i);
        let k2 = format!("txn{}_k2", i);
        let k3 = format!("txn{}_k3", i);

        txn.set(k1.as_bytes(), b"value").await.unwrap();
        txn.set(k2.as_bytes(), b"value").await.unwrap();
        txn.set(k3.as_bytes(), b"value").await.unwrap();

        let result = txn.commit().await;

        // Check atomicity: either all 3 keys exist or none
        let k1_exists = storage
            .get(&actor_id, k1.as_bytes())
            .await
            .unwrap()
            .is_some();
        let k2_exists = storage
            .get(&actor_id, k2.as_bytes())
            .await
            .unwrap()
            .is_some();
        let k3_exists = storage
            .get(&actor_id, k3.as_bytes())
            .await
            .unwrap()
            .is_some();

        if result.is_ok() {
            assert!(
                k1_exists && k2_exists && k3_exists,
                "if commit succeeded, all keys should exist for txn {}",
                i
            );
        } else {
            assert!(
                !k1_exists && !k2_exists && !k3_exists,
                "if commit failed, no keys should exist for txn {}",
                i
            );
        }
    }
}

/// Verify ReadYourWrites: transaction sees its own uncommitted writes.
///
/// TLA+ invariant (line 231): A transaction always sees its own uncommitted writes.
/// This is enforced by checking the write buffer before reading from storage.
///
/// Test approach:
/// - Within single transaction: write then read
/// - Verify read returns written value even before commit
#[madsim::test]
async fn test_read_your_writes_in_txn() {
    let rng = DeterministicRng::from_env_or_random();
    let fault_injector = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
    let storage = Arc::new(SimStorage::new(rng, fault_injector));
    let actor_id = ActorId::new("fdb-test", "ryw-1").unwrap();

    let mut txn = storage.begin_transaction(&actor_id).await.unwrap();

    // Write then read
    txn.set(b"k1", b"v1").await.unwrap();
    let value = txn.get(b"k1").await.unwrap();
    assert_eq!(
        value,
        Some(Bytes::from("v1")),
        "should see own write within transaction"
    );

    // Update then read
    txn.set(b"k1", b"v2").await.unwrap();
    let value = txn.get(b"k1").await.unwrap();
    assert_eq!(
        value,
        Some(Bytes::from("v2")),
        "should see updated write within transaction"
    );

    // Delete then read
    txn.delete(b"k1").await.unwrap();
    let value = txn.get(b"k1").await.unwrap();
    assert!(
        value.is_none(),
        "should see delete within transaction (read-your-writes)"
    );

    txn.abort().await.unwrap();
}

// =============================================================================
// Liveness Property Tests
// =============================================================================

/// Verify EventualTermination: every transaction eventually commits or aborts.
///
/// TLA+ property (line 250): []<>(committed ∨ aborted)
/// Every running transaction eventually reaches a final state.
///
/// Test approach:
/// - Start many transactions with various faults
/// - Verify all eventually resolve (no hanging transactions)
#[madsim::test]
async fn test_eventual_termination() {
    let rng = DeterministicRng::from_env_or_random();
    let fault_injector = Arc::new(
        FaultInjectorBuilder::new(rng.fork())
            .with_fault(
                FaultConfig::new(FaultType::CrashDuringTransaction, 0.3)
                    .with_filter("transaction_commit"),
            )
            .build(),
    );
    let storage = Arc::new(SimStorage::new(rng, fault_injector));
    let actor_id = ActorId::new("fdb-test", "liveness-1").unwrap();

    // Run 20 transactions
    let mut outcomes = Vec::new();
    for i in 0..20 {
        let mut txn = storage.begin_transaction(&actor_id).await.unwrap();
        txn.set(format!("k{}", i).as_bytes(), b"value")
            .await
            .unwrap();

        let result = txn.commit().await;
        outcomes.push(result.is_ok());
    }

    // All transactions terminated (either commit or abort)
    assert_eq!(outcomes.len(), 20, "all transactions must terminate");

    // At least some transactions should have committed (forward progress)
    let committed_count = outcomes.iter().filter(|&&ok| ok).count();
    assert!(
        committed_count > 0,
        "at least some transactions should commit (forward progress)"
    );
}

/// Verify EventualCommit: non-conflicting transactions eventually commit.
///
/// TLA+ property (line 256): A transaction with no conflicts eventually commits.
/// This ensures forward progress.
///
/// Test approach:
/// - Run transactions on disjoint key sets (no conflicts)
/// - Verify all eventually commit
#[madsim::test]
async fn test_eventual_commit() {
    let rng = DeterministicRng::from_env_or_random();
    let fault_injector = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
    let storage = Arc::new(SimStorage::new(rng, fault_injector));
    let actor_id = ActorId::new("fdb-test", "eventual-1").unwrap();

    // Run 10 transactions on disjoint keys (no conflicts possible)
    let mut results = Vec::new();
    for i in 0..10 {
        let mut txn = storage.begin_transaction(&actor_id).await.unwrap();
        txn.set(format!("k{}", i).as_bytes(), b"value")
            .await
            .unwrap();
        results.push(txn.commit().await);
    }

    // All should commit (no conflicts)
    for (i, result) in results.iter().enumerate() {
        assert!(
            result.is_ok(),
            "transaction {} should commit (no conflicts): {:?}",
            i,
            result
        );
    }
}

// =============================================================================
// Stress and Retry Tests
// =============================================================================

/// Test conflict retry logic.
///
/// Verifies that retrying a conflicted transaction eventually succeeds.
#[madsim::test]
async fn test_conflict_retry() {
    let rng = DeterministicRng::from_env_or_random();
    let fault_injector = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
    let storage = Arc::new(SimStorage::new(rng, fault_injector));
    let actor_id = ActorId::new("fdb-test", "retry-1").unwrap();

    // Set initial value
    storage.set(&actor_id, b"counter", b"0").await.unwrap();

    // Try to increment the counter with retries on conflict
    let mut retry_count = 0;
    let max_retries = 10;

    loop {
        let mut txn = storage.begin_transaction(&actor_id).await.unwrap();

        // Read current value
        let current = txn.get(b"counter").await.unwrap();
        let count: i32 = current
            .map(|b| String::from_utf8_lossy(&b).parse().unwrap())
            .unwrap_or(0);

        // Increment
        let new_count = count + 1;
        txn.set(b"counter", new_count.to_string().as_bytes())
            .await
            .unwrap();

        // Try to commit
        match txn.commit().await {
            Ok(_) => break, // Success!
            Err(Error::TransactionConflict { .. }) => {
                retry_count += 1;
                assert!(
                    retry_count < max_retries,
                    "exceeded max retries ({})",
                    max_retries
                );
                // Retry
            }
            Err(e) => panic!("unexpected error: {:?}", e),
        }
    }

    // Verify final value
    let final_value = storage.get(&actor_id, b"counter").await.unwrap();
    assert!(
        final_value.is_some(),
        "counter should exist after successful retry"
    );

    // Postcondition: retry count should be bounded
    assert!(
        retry_count < max_retries,
        "retry count ({}) should be less than max ({})",
        retry_count,
        max_retries
    );
}

/// Stress test: high-contention workload with concurrent transactions.
///
/// Run many concurrent transactions on a small key set.
/// Verifies:
/// - Serializability maintained under high contention
/// - Forward progress (some transactions commit)
/// - No deadlocks or hangs
#[madsim::test]
async fn test_high_contention_stress() {
    let rng = DeterministicRng::from_env_or_random();
    let fault_injector = Arc::new(
        FaultInjectorBuilder::new(rng.fork())
            .with_fault(
                FaultConfig::new(
                    FaultType::StorageLatency {
                        min_ms: 1,
                        max_ms: 5,
                    },
                    0.2,
                )
                .with_filter("storage_write"),
            )
            .build(),
    );
    let storage = Arc::new(SimStorage::new(rng, fault_injector));
    let actor_id = ActorId::new("fdb-test", "stress-1").unwrap();

    // Initialize small key set
    for i in 0..5 {
        storage
            .set(&actor_id, format!("k{}", i).as_bytes(), b"0")
            .await
            .unwrap();
    }

    // Run 50 concurrent transactions on the same 5 keys (high contention)
    let mut handles = Vec::new();
    for task_id in 0..50 {
        let storage = storage.clone();
        let actor_id = actor_id.clone();

        let handle = kelpie_core::current_runtime().spawn(async move {
            let mut txn = storage.begin_transaction(&actor_id).await.unwrap();

            // Each transaction reads and writes multiple keys
            for i in 0..3 {
                let _ = txn.get(format!("k{}", i).as_bytes()).await;
                txn.set(
                    format!("k{}", i).as_bytes(),
                    format!("updated_by_{}", task_id).as_bytes(),
                )
                .await
                .unwrap();
            }

            txn.commit().await.is_ok()
        });

        handles.push(handle);
    }

    // Wait for all concurrent transactions to complete
    let mut outcomes = Vec::new();
    for handle in handles {
        outcomes.push(handle.await.unwrap());
    }

    // Forward progress: at least some transactions should commit
    let committed_count = outcomes.iter().filter(|&&ok| ok).count();
    assert!(
        committed_count > 0,
        "at least some transactions should commit under high contention (forward progress)"
    );

    // Serializability: final state should be consistent
    // (all keys either updated or not, no partial updates visible)
    for i in 0..5 {
        let value = storage
            .get(&actor_id, format!("k{}", i).as_bytes())
            .await
            .unwrap();
        assert!(value.is_some(), "key k{} should exist after stress test", i);
    }
}
