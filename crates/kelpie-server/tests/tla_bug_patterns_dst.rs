//! TLA+ Bug Pattern DST Tests
//!
//! These tests verify that the invariant verification helpers correctly
//! detect bug patterns modeled in TLA+ specifications:
//!
//! - KelpieSingleActivation.tla - TryClaimActor_Racy, LeaseExpires_Racy
//! - KelpieRegistry.tla - RegisterActor_Racy
//! - KelpieActorState.tla - CommitTransaction_StateOnly
//!
//! Each test maps to a specific TLA+ bug pattern and verifies that:
//! 1. The buggy behavior produces invariant violations (test catches bugs)
//! 2. The safe behavior produces NO violations (correct implementation)
//!
//! # Why No Random Fault Injection
//!
//! NOTE: These tests intentionally do NOT include random fault injection.
//! Unlike resilience tests (which verify the system handles faults gracefully),
//! these tests verify that specific TLA+ bug patterns are correctly detected.
//!
//! Random faults would mask the scenarios being tested. For example, if a
//! TOCTOU race test fails due to an unrelated StorageWriteFail, it no longer
//! verifies that TOCTOU detection works correctly.
//!
//! For fault resilience testing, see:
//! - agent_actor_dst.rs
//! - agent_service_dst.rs
//! - real_adapter_simhttp_dst.rs
//! - and other *_dst.rs files in this directory
//!
//! # TigerStyle Principles
//!
//! 1. Print DST_SEED for every test (even these deterministic ones)
//! 2. Explicit outcome handling - no `assert!(result.is_ok())`
//! 3. Clear distinction between expected bugs vs implementation errors

mod common;

use common::tla_scenarios::{
    scenario_concurrent_registration_race, scenario_partial_commit, scenario_safe_concurrent_claim,
    scenario_toctou_race_dual_activation, scenario_zombie_actor_reclaim_race,
};
use common::InvariantViolation;

/// Helper to print test header with reproducibility info.
fn print_test_header(test_name: &str, tla_pattern: &str) {
    let seed = std::env::var("DST_SEED").unwrap_or_else(|_| "deterministic".to_string());
    println!("\n=== {} ===", test_name);
    println!("TLA+ Pattern: {}", tla_pattern);
    println!("DST_SEED: {}", seed);
    println!();
}

// =============================================================================
// BUG PATTERN TESTS (Expected Violations)
// =============================================================================

/// Test: TOCTOU race in TryClaimActor (TryClaimActor_Racy from TLA+)
///
/// From TLA+ KelpieSingleActivation.tla:
/// ```tla
/// TryClaimActor_Racy(actor, node) ==
///     /\ [actor |-> actor, node |-> node] \in pendingClaims
///     \* BUG: We don't re-check placements here!
///     /\ placements' = [placements EXCEPT ![actor] = node]
///     ...
/// ```
///
/// Expected outcome: SingleActivation violation detected.
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_toctou_race_dual_activation() {
    print_test_header(
        "test_toctou_race_dual_activation",
        "TryClaimActor_Racy (KelpieSingleActivation.tla)",
    );

    let (violations, description) = scenario_toctou_race_dual_activation().await;

    println!("Scenario result: {}", description);
    println!("Violations found: {}", violations.len());

    // TOCTOU race MUST produce SingleActivation violation
    // If it doesn't, either the test is broken or the bug model is wrong
    assert!(
        !violations.is_empty(),
        "TOCTOU race MUST produce SingleActivation violation.\n\
         This test models the bug where two nodes check placement as NULL,\n\
         then both claim the actor. If no violation was found, the test\n\
         scenario is not correctly modeling the race condition.\n\
         Scenario: {}",
        description
    );

    // Verify it's specifically a SingleActivation violation
    for (i, v) in violations.iter().enumerate() {
        println!("Violation {}: {}", i + 1, v);
        match v {
            InvariantViolation::SingleActivation {
                actor_id,
                active_on_nodes,
            } => {
                println!(
                    "  ✓ SingleActivation: actor '{}' on {} nodes",
                    actor_id,
                    active_on_nodes.len()
                );
                assert!(
                    active_on_nodes.len() > 1,
                    "SingleActivation violation must have > 1 node"
                );
            }
            _ => {
                panic!("Expected SingleActivation violation, got: {:?}", v);
            }
        }
    }

    println!("\n✓ TOCTOU race correctly detected SingleActivation violation");
}

/// Test: Zombie actor race (LeaseExpires_Racy from TLA+)
///
/// From TLA+ KelpieSingleActivation.tla:
/// ```tla
/// LeaseExpires_Racy(actor) ==
///     /\ leases[actor] # NULL
///     /\ leases[actor].expires <= time
///     /\ leases' = [leases EXCEPT ![actor] = NULL]
///     /\ placements' = [placements EXCEPT ![actor] = NULL]
///     \* BUG: Actor may still be in localActors (zombie until detected)
///     /\ UNCHANGED <<localActors, activating, time, pendingClaims>>
/// ```
///
/// Expected outcome: SingleActivation violation (zombie + new activation).
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_zombie_actor_reclaim_race() {
    print_test_header(
        "test_zombie_actor_reclaim_race",
        "LeaseExpires_Racy (KelpieSingleActivation.tla)",
    );

    let (violations, description) = scenario_zombie_actor_reclaim_race().await;

    println!("Scenario result: {}", description);
    println!("Violations found: {}", violations.len());

    // Zombie race MUST produce SingleActivation violation
    assert!(
        !violations.is_empty(),
        "Zombie race MUST produce SingleActivation violation.\n\
         This test models the bug where lease expires, placement is cleared,\n\
         but the original node still thinks it has the actor (zombie).\n\
         A new node claims the actor, creating dual activation.\n\
         Scenario: {}",
        description
    );

    for (i, v) in violations.iter().enumerate() {
        println!("Violation {}: {}", i + 1, v);
        assert!(
            matches!(v, InvariantViolation::SingleActivation { .. }),
            "Expected SingleActivation violation from zombie race"
        );
    }

    println!("\n✓ Zombie race correctly detected SingleActivation violation");
}

/// Test: Partial commit detection (CommitTransaction_StateOnly from TLA+)
///
/// From TLA+ KelpieActorState.tla:
/// ```tla
/// CommitTransaction_StateOnly(inv) ==
///     \* BUG: Only commit state, not KV
///     /\ actorState' = [actorState EXCEPT ![inv.actor] = inv.newState]
///     \* KV NOT updated!
///     /\ UNCHANGED <<actorKV, ...>>
/// ```
///
/// Expected outcome: PartialCommit violation detected.
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_partial_commit_detected() {
    print_test_header(
        "test_partial_commit_detected",
        "CommitTransaction_StateOnly (KelpieActorState.tla)",
    );

    let (violations, description) = scenario_partial_commit().await;

    println!("Scenario result: {}", description);
    println!("Violations found: {}", violations.len());

    // Partial commit MUST produce TransactionAtomicity violation
    assert!(
        !violations.is_empty(),
        "Partial commit MUST produce PartialCommit violation.\n\
         This test models the bug where state is written but KV is not,\n\
         violating transaction atomicity (all-or-nothing).\n\
         Scenario: {}",
        description
    );

    for (i, v) in violations.iter().enumerate() {
        println!("Violation {}: {}", i + 1, v);
        match v {
            InvariantViolation::PartialCommit {
                state_committed,
                kv_committed,
                ..
            } => {
                println!(
                    "  ✓ PartialCommit: state={}, kv={}",
                    state_committed, kv_committed
                );
                assert!(
                    state_committed != kv_committed,
                    "Partial commit means state != kv committed status"
                );
            }
            _ => {
                panic!("Expected PartialCommit violation, got: {:?}", v);
            }
        }
    }

    println!("\n✓ Partial commit correctly detected TransactionAtomicity violation");
}

// =============================================================================
// SAFE BEHAVIOR TESTS (No Violations Expected)
// =============================================================================

/// Test: Concurrent registration with atomic claims respects capacity.
///
/// From TLA+ KelpieRegistry.tla, this tests the SAFE behavior:
/// ```tla
/// TryClaimActor_Atomic(actor, node) ==
///     /\ placements[actor] = NULL  \* Re-check inside "transaction"
///     /\ nodes[node].actor_count < nodes[node].capacity
///     ...
/// ```
///
/// Expected outcome: NO capacity violation.
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_concurrent_registration_respects_capacity() {
    print_test_header(
        "test_concurrent_registration_respects_capacity",
        "TryClaimActor_Atomic capacity check (KelpieRegistry.tla)",
    );

    let (violations, description) = scenario_concurrent_registration_race().await;

    println!("Scenario result: {}", description);
    println!("Violations found: {}", violations.len());

    // Atomic claims should respect capacity
    if !violations.is_empty() {
        for (i, v) in violations.iter().enumerate() {
            eprintln!("Unexpected violation {}: {}", i + 1, v);
        }
        panic!(
            "Atomic claims MUST respect capacity bounds.\n\
             Found {} violations when expecting none.\n\
             Scenario: {}",
            violations.len(),
            description
        );
    }

    println!("\n✓ Atomic claims correctly respect capacity bounds");
}

/// Test: Safe concurrent claims prevent SingleActivation violation.
///
/// This tests the SAFE behavior from TLA+ TryClaimActor_Atomic.
///
/// Expected outcome: Exactly one node gets the actor, NO violations.
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_safe_concurrent_claim_no_violations() {
    print_test_header(
        "test_safe_concurrent_claim_no_violations",
        "TryClaimActor_Atomic (KelpieSingleActivation.tla)",
    );

    let (violations, description) = scenario_safe_concurrent_claim().await;

    println!("Scenario result: {}", description);
    println!("Violations found: {}", violations.len());

    // Safe claims should produce no violations
    if !violations.is_empty() {
        for (i, v) in violations.iter().enumerate() {
            eprintln!("Unexpected violation {}: {}", i + 1, v);
        }
        panic!(
            "Atomic claims MUST NOT produce SingleActivation violations.\n\
             Found {} violations when expecting none.\n\
             Scenario: {}",
            violations.len(),
            description
        );
    }

    println!("\n✓ Atomic claims correctly prevent SingleActivation violation");
}

// =============================================================================
// INTEGRATION TEST
// =============================================================================

/// Integration test: Run all TLA+ bug patterns in sequence.
///
/// This provides a quick sanity check that all patterns are working.
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_all_tla_bug_patterns_integration() {
    print_test_header(
        "test_all_tla_bug_patterns_integration",
        "All TLA+ patterns (KelpieSingleActivation, KelpieRegistry, KelpieActorState)",
    );

    let mut total_expected_violations = 0;
    let mut total_unexpected_violations = 0;

    // 1. TOCTOU race (expected violation)
    println!("1/5 Testing TOCTOU race...");
    let (v, _) = scenario_toctou_race_dual_activation().await;
    if v.is_empty() {
        eprintln!("   ✗ FAILED: Expected violation, got none");
        total_unexpected_violations += 1;
    } else {
        println!("   ✓ PASSED: {} violations detected", v.len());
        total_expected_violations += v.len();
    }

    // 2. Zombie race (expected violation)
    println!("2/5 Testing zombie race...");
    let (v, _) = scenario_zombie_actor_reclaim_race().await;
    if v.is_empty() {
        eprintln!("   ✗ FAILED: Expected violation, got none");
        total_unexpected_violations += 1;
    } else {
        println!("   ✓ PASSED: {} violations detected", v.len());
        total_expected_violations += v.len();
    }

    // 3. Partial commit (expected violation)
    println!("3/5 Testing partial commit...");
    let (v, _) = scenario_partial_commit().await;
    if v.is_empty() {
        eprintln!("   ✗ FAILED: Expected violation, got none");
        total_unexpected_violations += 1;
    } else {
        println!("   ✓ PASSED: {} violations detected", v.len());
        total_expected_violations += v.len();
    }

    // 4. Concurrent registration (NO violation expected)
    println!("4/5 Testing concurrent registration (safe)...");
    let (v, _) = scenario_concurrent_registration_race().await;
    if !v.is_empty() {
        eprintln!("   ✗ FAILED: Expected no violations, got {}", v.len());
        total_unexpected_violations += v.len();
    } else {
        println!("   ✓ PASSED: No violations (as expected)");
    }

    // 5. Safe concurrent claims (NO violation expected)
    println!("5/5 Testing safe concurrent claims...");
    let (v, _) = scenario_safe_concurrent_claim().await;
    if !v.is_empty() {
        eprintln!("   ✗ FAILED: Expected no violations, got {}", v.len());
        total_unexpected_violations += v.len();
    } else {
        println!("   ✓ PASSED: No violations (as expected)");
    }

    println!("\n=== Summary ===");
    println!(
        "Expected violations detected: {}",
        total_expected_violations
    );
    println!("Unexpected violations: {}", total_unexpected_violations);

    assert_eq!(
        total_unexpected_violations, 0,
        "Integration test failed with {} unexpected violations",
        total_unexpected_violations
    );

    assert!(
        total_expected_violations >= 3,
        "Expected at least 3 violations from bug patterns, got {}",
        total_expected_violations
    );

    println!("\n✓ All TLA+ bug pattern tests passed");
}
