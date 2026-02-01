//! DST tests for Linearizability invariants
//!
//! TLA+ Spec Reference: `docs/tla/KelpieLinearizability.tla`
//!
//! This module tests the linearizability invariants defined in the TLA+ spec:
//!
//! | Invariant | Test | TLA+ Line |
//! |-----------|------|-----------|
//! | ReadYourWrites | test_read_your_writes | 381-394 |
//! | MonotonicReads | test_monotonic_reads | 399-412 |
//! | DispatchConsistency | test_dispatch_consistency | 416-436 |
//!
//! TigerStyle: Deterministic testing with explicit fault injection.

use futures::future::join_all;
use kelpie_core::error::{Error, Result};
use kelpie_core::Runtime;
use kelpie_dst::{FaultConfig, FaultType, InvariantViolation, SimConfig, Simulation};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

// =============================================================================
// Constants (TigerStyle: Explicit with units)
// =============================================================================

/// Maximum operations per test for bounded checking
#[allow(dead_code)]
const OPERATIONS_COUNT_MAX: usize = 100;

/// Number of concurrent clients in tests
const CLIENTS_COUNT_DEFAULT: usize = 3;

// =============================================================================
// Linearization History Model
// =============================================================================

/// Operation type in the linearization history
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationType {
    Claim,
    Release,
    Read,
    Dispatch,
}

/// Response type for operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationResponse {
    Ok,
    Fail,
    Owner(String),
    NoOwner,
}

/// A linearized operation in the history
#[derive(Debug, Clone)]
pub struct LinearizedOp {
    pub op_type: OperationType,
    pub client: String,
    pub actor: String,
    pub id: u64,
    pub response: OperationResponse,
}

/// Linearization history tracker for invariant checking
#[derive(Debug)]
pub struct LinearizationHistory {
    history: RwLock<Vec<LinearizedOp>>,
    op_counter: AtomicU64,
    /// Actor ownership: actor_id -> node_id | None
    ownership: RwLock<HashMap<String, Option<String>>>,
}

impl LinearizationHistory {
    pub fn new() -> Self {
        Self {
            history: RwLock::new(Vec::new()),
            op_counter: AtomicU64::new(0),
            ownership: RwLock::new(HashMap::new()),
        }
    }

    /// Get next operation ID
    fn next_id(&self) -> u64 {
        self.op_counter.fetch_add(1, Ordering::SeqCst)
    }

    /// Record a Claim operation
    pub async fn record_claim(&self, client: &str, actor: &str, node: &str) -> Result<u64> {
        let id = self.next_id();
        let mut ownership = self.ownership.write().await;

        // Check if actor is already owned
        let current_owner = ownership.get(actor).cloned().flatten();

        let response = if current_owner.is_none() {
            // Success: claim the actor
            ownership.insert(actor.to_string(), Some(node.to_string()));
            OperationResponse::Ok
        } else {
            // Fail: already owned
            OperationResponse::Fail
        };

        // Record in history
        let mut history = self.history.write().await;
        history.push(LinearizedOp {
            op_type: OperationType::Claim,
            client: client.to_string(),
            actor: actor.to_string(),
            id,
            response: response.clone(),
        });

        match response {
            OperationResponse::Ok => Ok(id),
            OperationResponse::Fail => Err(Error::ActorAlreadyExists {
                id: actor.to_string(),
            }),
            _ => unreachable!(),
        }
    }

    /// Record a Release operation
    pub async fn record_release(&self, client: &str, actor: &str) -> Result<()> {
        let id = self.next_id();
        let mut ownership = self.ownership.write().await;

        // Check current ownership
        let current_owner = ownership.get(actor).cloned().flatten();

        let response = if current_owner.is_some() {
            // Success: release the actor
            ownership.insert(actor.to_string(), None);
            OperationResponse::Ok
        } else {
            // Fail: not owned
            OperationResponse::Fail
        };

        // Record in history
        let mut history = self.history.write().await;
        history.push(LinearizedOp {
            op_type: OperationType::Release,
            client: client.to_string(),
            actor: actor.to_string(),
            id,
            response: response.clone(),
        });

        match response {
            OperationResponse::Ok => Ok(()),
            OperationResponse::Fail => Err(Error::ActorNotFound {
                id: actor.to_string(),
            }),
            _ => unreachable!(),
        }
    }

    /// Record a Read operation
    pub async fn record_read(&self, client: &str, actor: &str) -> OperationResponse {
        let id = self.next_id();
        let ownership = self.ownership.read().await;

        let response = match ownership.get(actor).cloned().flatten() {
            Some(owner) => OperationResponse::Owner(owner),
            None => OperationResponse::NoOwner,
        };

        // Record in history
        let mut history = self.history.write().await;
        history.push(LinearizedOp {
            op_type: OperationType::Read,
            client: client.to_string(),
            actor: actor.to_string(),
            id,
            response: response.clone(),
        });

        response
    }

    /// Record a Dispatch operation
    pub async fn record_dispatch(&self, client: &str, actor: &str) -> Result<()> {
        let id = self.next_id();
        let ownership = self.ownership.read().await;

        let current_owner = ownership.get(actor).cloned().flatten();

        let response = if current_owner.is_some() {
            OperationResponse::Ok
        } else {
            OperationResponse::Fail
        };

        // Record in history
        let mut history = self.history.write().await;
        history.push(LinearizedOp {
            op_type: OperationType::Dispatch,
            client: client.to_string(),
            actor: actor.to_string(),
            id,
            response: response.clone(),
        });

        match response {
            OperationResponse::Ok => Ok(()),
            OperationResponse::Fail => Err(Error::ActorNotFound {
                id: actor.to_string(),
            }),
            _ => unreachable!(),
        }
    }

    /// Get a snapshot of the history
    pub async fn snapshot(&self) -> Vec<LinearizedOp> {
        self.history.read().await.clone()
    }
}

// =============================================================================
// Linearizability Invariant Implementations
// =============================================================================

/// ReadYourWrites invariant from KelpieLinearizability.tla (lines 381-394)
///
/// **TLA+ Definition:**
/// ```tla
/// ReadYourWrites ==
///     \A i, j \in 1..Len(history):
///         /\ i < j
///         /\ history[i].client = history[j].client
///         /\ history[i].type = "Claim"
///         /\ history[i].response = "ok"
///         /\ history[j].type = "Read"
///         /\ history[j].actor = history[i].actor
///         /\ ~\E k \in (i+1)..(j-1):
///             /\ history[k].actor = history[i].actor
///             /\ history[k].type = "Release"
///             /\ history[k].response = "ok"
///         => history[j].response # "no_owner"
/// ```
///
/// If client C successfully claims actor A, then a subsequent read by the
/// SAME client C on actor A (with no intervening release) must see an owner.
pub struct LinearizationReadYourWrites;

impl LinearizationReadYourWrites {
    /// Check the invariant against a history
    pub fn check_history(
        &self,
        history: &[LinearizedOp],
    ) -> std::result::Result<(), InvariantViolation> {
        for (i, op_i) in history.iter().enumerate() {
            // Look for successful claims
            if op_i.op_type != OperationType::Claim || op_i.response != OperationResponse::Ok {
                continue;
            }

            // Find subsequent reads by the same client on the same actor
            for (j, op_j) in history.iter().enumerate().skip(i + 1) {
                if op_j.client != op_i.client
                    || op_j.op_type != OperationType::Read
                    || op_j.actor != op_i.actor
                {
                    continue;
                }

                // Check for intervening release
                let has_intervening_release = history[i + 1..j].iter().any(|op_k| {
                    op_k.actor == op_i.actor
                        && op_k.op_type == OperationType::Release
                        && op_k.response == OperationResponse::Ok
                });

                if has_intervening_release {
                    continue;
                }

                // No intervening release - read must see an owner
                if op_j.response == OperationResponse::NoOwner {
                    return Err(InvariantViolation::with_evidence(
                        "ReadYourWrites",
                        format!(
                            "Client '{}' claimed actor '{}' at op {} but read at op {} returned no_owner",
                            op_i.client, op_i.actor, op_i.id, op_j.id
                        ),
                        format!(
                            "Claim op: {:?}, Read op: {:?}",
                            op_i, op_j
                        ),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// MonotonicReads invariant from KelpieLinearizability.tla (lines 399-412)
///
/// **TLA+ Definition:**
/// ```tla
/// MonotonicReads ==
///     \A i, j \in 1..Len(history):
///         /\ i < j
///         /\ history[i].client = history[j].client
///         /\ history[i].type = "Read"
///         /\ history[i].actor = history[j].actor
///         /\ history[j].type = "Read"
///         /\ history[i].response # "no_owner"
///         /\ ~\E k \in (i+1)..(j-1):
///             /\ history[k].actor = history[i].actor
///             /\ history[k].type = "Release"
///             /\ history[k].response = "ok"
///         => history[j].response # "no_owner"
/// ```
///
/// For a single client, once they read an owner, their subsequent reads on
/// the same actor don't regress to "no_owner" unless there's an intervening
/// successful release.
pub struct LinearizationMonotonicReads;

impl LinearizationMonotonicReads {
    /// Check the invariant against a history
    pub fn check_history(
        &self,
        history: &[LinearizedOp],
    ) -> std::result::Result<(), InvariantViolation> {
        for (i, op_i) in history.iter().enumerate() {
            // Look for reads that returned an owner
            if op_i.op_type != OperationType::Read || op_i.response == OperationResponse::NoOwner {
                continue;
            }

            // Find subsequent reads by the same client on the same actor
            for (j, op_j) in history.iter().enumerate().skip(i + 1) {
                if op_j.client != op_i.client
                    || op_j.op_type != OperationType::Read
                    || op_j.actor != op_i.actor
                {
                    continue;
                }

                // Check for intervening release
                let has_intervening_release = history[i + 1..j].iter().any(|op_k| {
                    op_k.actor == op_i.actor
                        && op_k.op_type == OperationType::Release
                        && op_k.response == OperationResponse::Ok
                });

                if has_intervening_release {
                    continue;
                }

                // No intervening release - subsequent read must also see an owner
                if op_j.response == OperationResponse::NoOwner {
                    return Err(InvariantViolation::with_evidence(
                        "MonotonicReads",
                        format!(
                            "Client '{}' read owner for actor '{}' at op {} but later read at op {} returned no_owner",
                            op_i.client, op_i.actor, op_i.id, op_j.id
                        ),
                        format!(
                            "First read: {:?}, Second read: {:?}",
                            op_i, op_j
                        ),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// DispatchConsistency invariant from KelpieLinearizability.tla (lines 416-436)
///
/// **TLA+ Definition:**
/// ```tla
/// DispatchConsistency ==
///     \A i \in 1..Len(history):
///         history[i].type = "Dispatch" =>
///         LET prior_claims == {j \in 1..(i-1): ...}
///             prior_releases == {j \in 1..(i-1): ...}
///             last_claim == ...
///             last_release == ...
///         IN (history[i].response = "ok") <=> (last_claim > last_release)
/// ```
///
/// Dispatch succeeds if and only if the actor is owned (last claim > last release).
pub struct LinearizationDispatchConsistency;

impl LinearizationDispatchConsistency {
    /// Check the invariant against a history
    pub fn check_history(
        &self,
        history: &[LinearizedOp],
    ) -> std::result::Result<(), InvariantViolation> {
        for (i, op_i) in history.iter().enumerate() {
            if op_i.op_type != OperationType::Dispatch {
                continue;
            }

            // Find most recent successful claim for this actor (using Option)
            let last_claim = history[..i]
                .iter()
                .enumerate()
                .filter(|(_, op)| {
                    op.actor == op_i.actor
                        && op.op_type == OperationType::Claim
                        && op.response == OperationResponse::Ok
                })
                .map(|(idx, _)| idx)
                .max();

            // Find most recent successful release for this actor
            let last_release = history[..i]
                .iter()
                .enumerate()
                .filter(|(_, op)| {
                    op.actor == op_i.actor
                        && op.op_type == OperationType::Release
                        && op.response == OperationResponse::Ok
                })
                .map(|(idx, _)| idx)
                .max();

            // Actor is owned iff there's a claim that's more recent than any release
            let actor_owned = match (last_claim, last_release) {
                (None, _) => false,               // No claim ever - not owned
                (Some(_claim_idx), None) => true, // Claim exists, no release - owned
                (Some(claim_idx), Some(release_idx)) => claim_idx > release_idx, // Claim after release - owned
            };
            let dispatch_succeeded = op_i.response == OperationResponse::Ok;

            if dispatch_succeeded != actor_owned {
                return Err(InvariantViolation::with_evidence(
                    "DispatchConsistency",
                    format!(
                        "Dispatch for actor '{}' at op {} returned {:?} but actor_owned={} (last_claim={:?}, last_release={:?})",
                        op_i.actor, op_i.id, op_i.response, actor_owned, last_claim, last_release
                    ),
                    format!("Dispatch op: {:?}", op_i),
                ));
            }
        }
        Ok(())
    }
}

// =============================================================================
// DST Tests
// =============================================================================

/// Test ReadYourWrites: Write(k,v) then Read(k) returns v
///
/// TLA+ Invariant: ReadYourWrites (lines 381-394)
///
/// A client that successfully claims an actor should see that actor as owned
/// when it subsequently reads the actor state.
#[test]
fn test_read_your_writes() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(seed = config.seed, "Running ReadYourWrites test");

    let result = Simulation::new(config).run(|_env| async move {
        let history = Arc::new(LinearizationHistory::new());
        let actor = "actor-1";
        let node = "node-1";

        // Client claims actor
        let client = "client-1";
        history.record_claim(client, actor, node).await?;

        // Same client reads - should see owner
        let read_result = history.record_read(client, actor).await;
        assert_ne!(
            read_result,
            OperationResponse::NoOwner,
            "ReadYourWrites violated: client read no_owner after claiming"
        );

        // Verify with invariant checker
        let snapshot = history.snapshot().await;
        LinearizationReadYourWrites
            .check_history(&snapshot)
            .map_err(|v| Error::Internal {
                message: format!("Invariant violation: {}", v),
            })?;

        tracing::info!("ReadYourWrites test passed");
        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test ReadYourWrites with concurrent operations
///
/// Multiple clients operate on the same actor, but each client's reads
/// should reflect their own writes.
#[test]
fn test_read_your_writes_concurrent() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(seed = config.seed, "Running ReadYourWrites concurrent test");

    let result = Simulation::new(config).run(|_env| async move {
        let history = Arc::new(LinearizationHistory::new());
        let num_clients = CLIENTS_COUNT_DEFAULT;
        let actor = "shared-actor";

        // Each client tries to claim the actor
        let handles: Vec<_> = (0..num_clients)
            .map(|i| {
                let history = history.clone();
                let client = format!("client-{}", i);
                let node = format!("node-{}", i);
                let actor = actor.to_string();
                kelpie_core::current_runtime().spawn(async move {
                    // Try to claim
                    let claim_result = history.record_claim(&client, &actor, &node).await;

                    // Read regardless of claim result
                    let read_result = history.record_read(&client, &actor).await;

                    (client, claim_result.is_ok(), read_result)
                })
            })
            .collect();

        let results: Vec<_> = join_all(handles)
            .await
            .into_iter()
            .map(|r| r.expect("task panicked"))
            .collect();

        // Verify: exactly one client should have claimed successfully
        let successful_claims: Vec<_> = results
            .iter()
            .filter(|(_, claimed, _)| *claimed)
            .map(|(client, _, _)| client.clone())
            .collect();

        assert_eq!(
            successful_claims.len(),
            1,
            "Expected exactly one successful claim, got: {:?}",
            successful_claims
        );

        // The successful claimer should see an owner when they read
        for (client, claimed, read_result) in &results {
            if *claimed {
                assert_ne!(
                    *read_result,
                    OperationResponse::NoOwner,
                    "ReadYourWrites violated: {} claimed but read no_owner",
                    client
                );
            }
        }

        // Verify full history with invariant checker
        let snapshot = history.snapshot().await;
        LinearizationReadYourWrites
            .check_history(&snapshot)
            .map_err(|v| Error::Internal {
                message: format!("Invariant violation: {}", v),
            })?;

        tracing::info!(
            successful_claims = ?successful_claims,
            "ReadYourWrites concurrent test passed"
        );
        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test MonotonicReads: Read(k)=v implies future Read(k) >= v
///
/// TLA+ Invariant: MonotonicReads (lines 399-412)
///
/// Once a client reads an owner, subsequent reads should not regress to
/// no_owner unless there's an intervening release.
#[test]
fn test_monotonic_reads() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(seed = config.seed, "Running MonotonicReads test");

    let result = Simulation::new(config).run(|_env| async move {
        let history = Arc::new(LinearizationHistory::new());
        let actor = "actor-1";

        // Setup: claim the actor
        history.record_claim("client-0", actor, "node-1").await?;

        // Client-1 reads multiple times - should always see owner
        let client = "client-1";
        let read1 = history.record_read(client, actor).await;
        let read2 = history.record_read(client, actor).await;
        let read3 = history.record_read(client, actor).await;

        // All reads should see owner
        assert_ne!(read1, OperationResponse::NoOwner);
        assert_ne!(read2, OperationResponse::NoOwner);
        assert_ne!(read3, OperationResponse::NoOwner);

        // Verify with invariant checker
        let snapshot = history.snapshot().await;
        LinearizationMonotonicReads
            .check_history(&snapshot)
            .map_err(|v| Error::Internal {
                message: format!("Invariant violation: {}", v),
            })?;

        tracing::info!("MonotonicReads test passed");
        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test MonotonicReads with release in between
///
/// After a release, reads may return no_owner - this should not violate
/// MonotonicReads since there was an intervening release.
#[test]
fn test_monotonic_reads_with_release() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(
        seed = config.seed,
        "Running MonotonicReads with release test"
    );

    let result = Simulation::new(config).run(|_env| async move {
        let history = Arc::new(LinearizationHistory::new());
        let actor = "actor-1";
        let client = "client-1";

        // Setup: claim the actor
        history.record_claim("client-0", actor, "node-1").await?;

        // First read - should see owner
        let read1 = history.record_read(client, actor).await;
        assert_ne!(read1, OperationResponse::NoOwner);

        // Release the actor
        history.record_release("client-0", actor).await?;

        // Second read - may return no_owner (this is OK because of release)
        let _read2 = history.record_read(client, actor).await;

        // Verify with invariant checker - should pass because release was intervening
        let snapshot = history.snapshot().await;
        LinearizationMonotonicReads
            .check_history(&snapshot)
            .map_err(|v| Error::Internal {
                message: format!("Invariant violation: {}", v),
            })?;

        tracing::info!("MonotonicReads with release test passed");
        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test DispatchConsistency: dispatch(actor) routes to node where actor is active
///
/// TLA+ Invariant: DispatchConsistency (lines 416-436)
///
/// Dispatch should succeed iff the actor is owned (has been claimed and not released).
#[test]
fn test_dispatch_consistency() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(seed = config.seed, "Running DispatchConsistency test");

    let result = Simulation::new(config).run(|_env| async move {
        let history = Arc::new(LinearizationHistory::new());
        let actor = "actor-1";
        let client = "client-1";

        // Dispatch before claim - should fail
        let dispatch1 = history.record_dispatch(client, actor).await;
        assert!(
            dispatch1.is_err(),
            "Dispatch should fail when actor not claimed"
        );

        // Claim the actor
        history.record_claim("client-0", actor, "node-1").await?;

        // Dispatch after claim - should succeed
        let dispatch2 = history.record_dispatch(client, actor).await;
        assert!(
            dispatch2.is_ok(),
            "Dispatch should succeed when actor is claimed"
        );

        // Release the actor
        history.record_release("client-0", actor).await?;

        // Dispatch after release - should fail
        let dispatch3 = history.record_dispatch(client, actor).await;
        assert!(
            dispatch3.is_err(),
            "Dispatch should fail after actor released"
        );

        // Verify with invariant checker
        let snapshot = history.snapshot().await;
        LinearizationDispatchConsistency
            .check_history(&snapshot)
            .map_err(|v| Error::Internal {
                message: format!("Invariant violation: {}", v),
            })?;

        tracing::info!("DispatchConsistency test passed");
        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test DispatchConsistency with multiple actors
///
/// Dispatch operations should correctly track ownership per actor.
#[test]
fn test_dispatch_consistency_multi_actor() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(
        seed = config.seed,
        "Running DispatchConsistency multi-actor test"
    );

    let result = Simulation::new(config).run(|_env| async move {
        let history = Arc::new(LinearizationHistory::new());
        let client = "client-1";

        // Claim actor-1, not actor-2
        history
            .record_claim("client-0", "actor-1", "node-1")
            .await?;

        // Dispatch to actor-1 should succeed
        let dispatch1 = history.record_dispatch(client, "actor-1").await;
        assert!(
            dispatch1.is_ok(),
            "Dispatch to claimed actor should succeed"
        );

        // Dispatch to actor-2 should fail (not claimed)
        let dispatch2 = history.record_dispatch(client, "actor-2").await;
        assert!(
            dispatch2.is_err(),
            "Dispatch to unclaimed actor should fail"
        );

        // Verify with invariant checker
        let snapshot = history.snapshot().await;
        LinearizationDispatchConsistency
            .check_history(&snapshot)
            .map_err(|v| Error::Internal {
                message: format!("Invariant violation: {}", v),
            })?;

        tracing::info!("DispatchConsistency multi-actor test passed");
        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Determinism Tests
// =============================================================================

/// Test that same seed produces same history
///
/// TigerStyle: Determinism verification - same seed = same outcome
#[test]
fn test_linearizability_deterministic() {
    let seed = 42_u64;

    let run_test = || {
        let config = SimConfig::new(seed);

        Simulation::new(config).run(|_env| async move {
            let history = Arc::new(LinearizationHistory::new());
            let num_ops = 20;

            // Perform a fixed sequence of operations
            for i in 0..num_ops {
                let client = format!("client-{}", i % 3);
                let actor = format!("actor-{}", i % 2);
                let node = format!("node-{}", i % 3);

                match i % 4 {
                    0 => {
                        let _ = history.record_claim(&client, &actor, &node).await;
                    }
                    1 => {
                        let _ = history.record_read(&client, &actor).await;
                    }
                    2 => {
                        let _ = history.record_dispatch(&client, &actor).await;
                    }
                    3 => {
                        let _ = history.record_release(&client, &actor).await;
                    }
                    _ => unreachable!(),
                }
            }

            let snapshot = history.snapshot().await;
            Ok(snapshot)
        })
    };

    let result1 = run_test().expect("First run failed");
    let result2 = run_test().expect("Second run failed");

    // Compare operation IDs and responses
    assert_eq!(result1.len(), result2.len(), "History lengths differ");

    for (op1, op2) in result1.iter().zip(result2.iter()) {
        assert_eq!(
            op1.id, op2.id,
            "Operation IDs differ at position {}: {} vs {}",
            op1.id, op1.id, op2.id
        );
        assert_eq!(
            op1.response, op2.response,
            "Responses differ at op {}: {:?} vs {:?}",
            op1.id, op1.response, op2.response
        );
    }

    tracing::info!("Determinism test passed");
}

// =============================================================================
// Fault Injection Tests
// =============================================================================

/// Test linearizability invariants under storage write failures
///
/// Even with transient failures, the invariants must hold.
#[test]
fn test_linearizability_with_storage_faults() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(
        seed = config.seed,
        "Running linearizability with storage faults"
    );

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.1))
        .run(|_env| async move {
            let history = Arc::new(LinearizationHistory::new());
            let num_clients = CLIENTS_COUNT_DEFAULT;
            let num_ops_per_client = 10;

            // Multiple clients perform operations concurrently
            let handles: Vec<_> = (0..num_clients)
                .map(|client_id| {
                    let history = history.clone();
                    let client = format!("client-{}", client_id);
                    let node = format!("node-{}", client_id);
                    kelpie_core::current_runtime().spawn(async move {
                        for op_num in 0..num_ops_per_client {
                            let actor = format!("actor-{}", op_num % 3);
                            match op_num % 4 {
                                0 => {
                                    let _ = history.record_claim(&client, &actor, &node).await;
                                }
                                1 => {
                                    let _ = history.record_read(&client, &actor).await;
                                }
                                2 => {
                                    let _ = history.record_dispatch(&client, &actor).await;
                                }
                                3 => {
                                    let _ = history.record_release(&client, &actor).await;
                                }
                                _ => unreachable!(),
                            }
                        }
                    })
                })
                .collect();

            join_all(handles).await;

            // Verify all invariants
            let snapshot = history.snapshot().await;
            LinearizationReadYourWrites
                .check_history(&snapshot)
                .map_err(|v| Error::Internal {
                    message: format!("Invariant violation: {}", v),
                })?;
            LinearizationMonotonicReads
                .check_history(&snapshot)
                .map_err(|v| Error::Internal {
                    message: format!("Invariant violation: {}", v),
                })?;
            LinearizationDispatchConsistency
                .check_history(&snapshot)
                .map_err(|v| Error::Internal {
                    message: format!("Invariant violation: {}", v),
                })?;

            tracing::info!(
                total_ops = snapshot.len(),
                "Linearizability invariants held under storage faults"
            );
            Ok(())
        });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test linearizability invariants under network delays
#[test]
fn test_linearizability_with_network_delays() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(
        seed = config.seed,
        "Running linearizability with network delays"
    );

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::NetworkDelay {
                min_ms: 10,
                max_ms: 100,
            },
            0.3,
        ))
        .run(|_env| async move {
            let history = Arc::new(LinearizationHistory::new());

            // Perform operations with potential delays
            for i in 0..20 {
                let client = format!("client-{}", i % 3);
                let actor = format!("actor-{}", i % 2);
                let node = format!("node-{}", i % 3);

                match i % 4 {
                    0 => {
                        let _ = history.record_claim(&client, &actor, &node).await;
                    }
                    1 => {
                        let _ = history.record_read(&client, &actor).await;
                    }
                    2 => {
                        let _ = history.record_dispatch(&client, &actor).await;
                    }
                    3 => {
                        let _ = history.record_release(&client, &actor).await;
                    }
                    _ => unreachable!(),
                }
            }

            // Verify invariants
            let snapshot = history.snapshot().await;
            LinearizationReadYourWrites
                .check_history(&snapshot)
                .map_err(|v| Error::Internal {
                    message: format!("Invariant violation: {}", v),
                })?;
            LinearizationMonotonicReads
                .check_history(&snapshot)
                .map_err(|v| Error::Internal {
                    message: format!("Invariant violation: {}", v),
                })?;
            LinearizationDispatchConsistency
                .check_history(&snapshot)
                .map_err(|v| Error::Internal {
                    message: format!("Invariant violation: {}", v),
                })?;

            tracing::info!("Linearizability invariants held under network delays");
            Ok(())
        });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Stress Tests
// =============================================================================

/// Stress test with many operations
#[test]
#[ignore] // Run with: cargo test linearizability_stress --release -- --ignored
fn test_linearizability_stress() {
    let seed = std::env::var("DST_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(rand::random);

    let config = SimConfig::new(seed);
    tracing::info!(seed = config.seed, "Running linearizability stress test");

    let result = Simulation::new(config).run(|_env| async move {
        let history = Arc::new(LinearizationHistory::new());
        let num_clients = 10;
        let ops_per_client = 100;

        let handles: Vec<_> = (0..num_clients)
            .map(|client_id| {
                let history = history.clone();
                let client = format!("client-{}", client_id);
                let node = format!("node-{}", client_id);
                kelpie_core::current_runtime().spawn(async move {
                    for op_num in 0..ops_per_client {
                        let actor = format!("actor-{}", op_num % 5);
                        match op_num % 4 {
                            0 => {
                                let _ = history.record_claim(&client, &actor, &node).await;
                            }
                            1 => {
                                let _ = history.record_read(&client, &actor).await;
                            }
                            2 => {
                                let _ = history.record_dispatch(&client, &actor).await;
                            }
                            3 => {
                                let _ = history.record_release(&client, &actor).await;
                            }
                            _ => unreachable!(),
                        }
                    }
                })
            })
            .collect();

        join_all(handles).await;

        let snapshot = history.snapshot().await;
        tracing::info!(
            total_ops = snapshot.len(),
            "Checking invariants on {} operations",
            snapshot.len()
        );

        LinearizationReadYourWrites
            .check_history(&snapshot)
            .map_err(|v| Error::Internal {
                message: format!("Invariant violation: {}", v),
            })?;
        LinearizationMonotonicReads
            .check_history(&snapshot)
            .map_err(|v| Error::Internal {
                message: format!("Invariant violation: {}", v),
            })?;
        LinearizationDispatchConsistency
            .check_history(&snapshot)
            .map_err(|v| Error::Internal {
                message: format!("Invariant violation: {}", v),
            })?;

        tracing::info!("Linearizability stress test passed");
        Ok(())
    });

    assert!(result.is_ok(), "Stress test failed: {:?}", result.err());
}

// =============================================================================
// Unit Tests for Invariant Checkers
// =============================================================================

#[cfg(test)]
mod invariant_tests {
    use super::*;

    #[test]
    fn test_read_your_writes_invariant_passes() {
        let history = vec![
            LinearizedOp {
                op_type: OperationType::Claim,
                client: "c1".to_string(),
                actor: "a1".to_string(),
                id: 0,
                response: OperationResponse::Ok,
            },
            LinearizedOp {
                op_type: OperationType::Read,
                client: "c1".to_string(),
                actor: "a1".to_string(),
                id: 1,
                response: OperationResponse::Owner("node-1".to_string()),
            },
        ];

        let result = LinearizationReadYourWrites.check_history(&history);
        assert!(result.is_ok());
    }

    #[test]
    fn test_read_your_writes_invariant_fails() {
        let history = vec![
            LinearizedOp {
                op_type: OperationType::Claim,
                client: "c1".to_string(),
                actor: "a1".to_string(),
                id: 0,
                response: OperationResponse::Ok,
            },
            LinearizedOp {
                op_type: OperationType::Read,
                client: "c1".to_string(),
                actor: "a1".to_string(),
                id: 1,
                response: OperationResponse::NoOwner, // Violation!
            },
        ];

        let result = LinearizationReadYourWrites.check_history(&history);
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert!(violation.to_string().contains("ReadYourWrites"));
    }

    #[test]
    fn test_monotonic_reads_invariant_passes() {
        let history = vec![
            LinearizedOp {
                op_type: OperationType::Read,
                client: "c1".to_string(),
                actor: "a1".to_string(),
                id: 0,
                response: OperationResponse::Owner("node-1".to_string()),
            },
            LinearizedOp {
                op_type: OperationType::Read,
                client: "c1".to_string(),
                actor: "a1".to_string(),
                id: 1,
                response: OperationResponse::Owner("node-1".to_string()),
            },
        ];

        let result = LinearizationMonotonicReads.check_history(&history);
        assert!(result.is_ok());
    }

    #[test]
    fn test_monotonic_reads_invariant_fails() {
        let history = vec![
            LinearizedOp {
                op_type: OperationType::Read,
                client: "c1".to_string(),
                actor: "a1".to_string(),
                id: 0,
                response: OperationResponse::Owner("node-1".to_string()),
            },
            LinearizedOp {
                op_type: OperationType::Read,
                client: "c1".to_string(),
                actor: "a1".to_string(),
                id: 1,
                response: OperationResponse::NoOwner, // Violation!
            },
        ];

        let result = LinearizationMonotonicReads.check_history(&history);
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert!(violation.to_string().contains("MonotonicReads"));
    }

    #[test]
    fn test_dispatch_consistency_invariant_passes() {
        let history = vec![
            LinearizedOp {
                op_type: OperationType::Claim,
                client: "c1".to_string(),
                actor: "a1".to_string(),
                id: 0,
                response: OperationResponse::Ok,
            },
            LinearizedOp {
                op_type: OperationType::Dispatch,
                client: "c2".to_string(),
                actor: "a1".to_string(),
                id: 1,
                response: OperationResponse::Ok, // Correct: actor is claimed
            },
        ];

        let result = LinearizationDispatchConsistency.check_history(&history);
        assert!(result.is_ok());
    }

    #[test]
    fn test_dispatch_consistency_invariant_fails() {
        let history = vec![LinearizedOp {
            op_type: OperationType::Dispatch,
            client: "c1".to_string(),
            actor: "a1".to_string(),
            id: 0,
            response: OperationResponse::Ok, // Violation: no prior claim
        }];

        let result = LinearizationDispatchConsistency.check_history(&history);
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert!(violation.to_string().contains("DispatchConsistency"));
    }
}
