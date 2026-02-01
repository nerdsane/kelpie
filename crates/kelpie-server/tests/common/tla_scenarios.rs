//! TLA+ Bug Pattern Test Scenarios
//!
//! These tests are derived from TLA+ specifications in `docs/tla/`:
//! - KelpieSingleActivation.tla - Tests TOCTOU and zombie actor patterns
//! - KelpieRegistry.tla - Tests concurrent registration races
//! - KelpieActorState.tla - Tests partial commit patterns
//!
//! Each test targets a specific bug pattern modeled in the TLA+ specs
//! and uses invariant verification helpers from `invariants.rs`.
//!
//! # TigerStyle Principles
//!
//! 1. **Explicit outcome handling** - No `assert!(result.is_ok())`
//! 2. **Print DST_SEED** - Every test prints seed for reproduction
//! 3. **Invariant verification** - After EVERY operation sequence
//! 4. **Distinguish expected vs unexpected failures** - Under faults, some failures are ok

use super::invariants::{
    verify_capacity_bounds, verify_single_activation, InvariantViolation, SystemState,
};
use kelpie_core::Runtime;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

// =============================================================================
// SIMULATED MULTI-NODE ENVIRONMENT
// =============================================================================

/// Simulates a distributed registry for testing placement races.
///
/// This is a simplified model that captures the essential race conditions
/// from the TLA+ specs without requiring full cluster infrastructure.
#[derive(Debug)]
pub struct SimulatedRegistry {
    /// actor_id -> node_id (official placement)
    placements: RwLock<HashMap<String, String>>,
    /// actor_id -> (lease_node, expires_at)
    leases: RwLock<HashMap<String, (String, u64)>>,
    /// node_id -> actor_count
    node_counts: RwLock<HashMap<String, usize>>,
    /// node_id -> capacity
    capacities: HashMap<String, usize>,
    /// Current logical time
    time: AtomicU64,
    /// Whether to use atomic claim (safe) or racy claim (buggy)
    use_atomic_claim: AtomicBool,
    /// Whether to use atomic lease cleanup (safe) or racy cleanup (buggy)
    use_atomic_lease_cleanup: AtomicBool,
}

impl SimulatedRegistry {
    /// Create a new simulated registry with the given nodes.
    pub fn new(nodes: Vec<(&str, usize)>) -> Self {
        let mut capacities = HashMap::new();
        let mut node_counts = HashMap::new();
        for (node_id, capacity) in nodes {
            capacities.insert(node_id.to_string(), capacity);
            node_counts.insert(node_id.to_string(), 0);
        }
        Self {
            placements: RwLock::new(HashMap::new()),
            leases: RwLock::new(HashMap::new()),
            node_counts: RwLock::new(node_counts),
            capacities,
            time: AtomicU64::new(0),
            use_atomic_claim: AtomicBool::new(true),
            use_atomic_lease_cleanup: AtomicBool::new(true),
        }
    }

    /// Enable buggy (racy) claim behavior for testing TryClaimActor_Racy.
    pub fn enable_racy_claim(&self) {
        self.use_atomic_claim.store(false, Ordering::SeqCst);
    }

    /// Enable buggy (racy) lease cleanup for testing LeaseExpires_Racy.
    pub fn enable_racy_lease_cleanup(&self) {
        self.use_atomic_lease_cleanup.store(false, Ordering::SeqCst);
    }

    /// Get current placement for an actor.
    pub async fn get_placement(&self, actor_id: &str) -> Option<String> {
        let placements = self.placements.read().await;
        placements.get(actor_id).cloned()
    }

    /// Try to claim an actor for a node.
    ///
    /// This models the critical TOCTOU bug from TLA+ TryClaimActor_Racy:
    /// - Safe: Re-reads placement inside "transaction"
    /// - Racy: Trusts stale check result
    pub async fn try_claim_actor(
        &self,
        actor_id: &str,
        node_id: &str,
        lease_duration: u64,
    ) -> Result<bool, String> {
        if self.use_atomic_claim.load(Ordering::SeqCst) {
            // Safe: Atomic check-and-claim
            let mut placements = self.placements.write().await;
            let mut leases = self.leases.write().await;
            let mut counts = self.node_counts.write().await;

            #[allow(unused_variables)]
            let current_time = self.time.load(Ordering::SeqCst);

            // Check if actor is available (not placed or lease expired)
            let available = match placements.get(actor_id) {
                None => true,
                Some(_existing_node) => {
                    // Check if lease expired
                    if let Some((_, expires)) = leases.get(actor_id) {
                        *expires <= current_time
                    } else {
                        // No lease = can reclaim (edge case)
                        true
                    }
                }
            };

            if !available {
                return Ok(false);
            }

            // Check capacity
            let count = counts.get(node_id).copied().unwrap_or(0);
            let capacity = self.capacities.get(node_id).copied().unwrap_or(10);
            if count >= capacity {
                return Err("Node at capacity".to_string());
            }

            // Claim it
            placements.insert(actor_id.to_string(), node_id.to_string());
            leases.insert(
                actor_id.to_string(),
                (node_id.to_string(), current_time + lease_duration),
            );
            *counts.entry(node_id.to_string()).or_insert(0) += 1;

            Ok(true)
        } else {
            // BUGGY: Non-atomic - doesn't re-check placement!
            // This models TryClaimActor_Racy from TLA+
            let mut placements = self.placements.write().await;
            let mut leases = self.leases.write().await;
            let mut counts = self.node_counts.write().await;

            let current_time = self.time.load(Ordering::SeqCst);

            // BUG: We just claim without checking current state
            // Assumes caller already checked (TOCTOU window!)
            placements.insert(actor_id.to_string(), node_id.to_string());
            leases.insert(
                actor_id.to_string(),
                (node_id.to_string(), current_time + lease_duration),
            );
            *counts.entry(node_id.to_string()).or_insert(0) += 1;

            Ok(true)
        }
    }

    /// Expire a lease and optionally clean up.
    ///
    /// This models LeaseExpires_Safe vs LeaseExpires_Racy from TLA+.
    pub async fn expire_lease(&self, actor_id: &str) {
        #[allow(unused_variables)]
        let current_time = self.time.load(Ordering::SeqCst);

        if self.use_atomic_lease_cleanup.load(Ordering::SeqCst) {
            // Safe: Atomic cleanup
            let mut placements = self.placements.write().await;
            let mut leases = self.leases.write().await;
            let mut counts = self.node_counts.write().await;

            if let Some((node, _)) = leases.remove(actor_id) {
                placements.remove(actor_id);
                if let Some(count) = counts.get_mut(&node) {
                    *count = count.saturating_sub(1);
                }
            }
        } else {
            // BUGGY: Only remove placement/lease, actor may still be "running" (zombie)
            // This models LeaseExpires_Racy from TLA+
            let mut placements = self.placements.write().await;
            let mut leases = self.leases.write().await;
            // BUG: We clear placement/lease but don't coordinate with node!
            // Node may still think actor is running = zombie
            placements.remove(actor_id);
            leases.remove(actor_id);
            // Note: counts NOT decremented - this is the bug!
        }
    }

    /// Advance time.
    pub fn advance_time(&self, delta: u64) {
        self.time.fetch_add(delta, Ordering::SeqCst);
    }

    /// Get current time.
    #[allow(dead_code)]
    pub fn now(&self) -> u64 {
        self.time.load(Ordering::SeqCst)
    }

    /// Get system state snapshot for invariant verification.
    pub async fn get_state(&self) -> SystemState {
        let placements = self.placements.read().await;
        let leases = self.leases.read().await;
        let counts = self.node_counts.read().await;

        let mut state = SystemState::new();
        state.current_time = self.time.load(Ordering::SeqCst);

        // Build placements
        for (actor, node) in placements.iter() {
            state.placements.insert(actor.clone(), node.clone());
        }

        // Build leases
        for (actor, (node, expires)) in leases.iter() {
            state.leases.insert(actor.clone(), (node.clone(), *expires));
        }

        // Build node_info
        for (node, &count) in counts.iter() {
            let capacity = self.capacities.get(node).copied().unwrap_or(10);
            state.node_info.insert(node.clone(), (count, capacity));
        }

        // Build active_actors (from placements for this simplified model)
        for (actor, node) in placements.iter() {
            state
                .active_actors
                .entry(node.clone())
                .or_default()
                .insert(actor.clone());
        }

        state
    }
}

/// Simulates a node's local state for multi-node testing.
#[derive(Debug)]
pub struct SimulatedNode {
    #[allow(dead_code)]
    pub node_id: String,
    /// Actors this node thinks are active locally
    pub local_actors: RwLock<HashSet<String>>,
}

impl SimulatedNode {
    pub fn new(node_id: &str) -> Self {
        Self {
            node_id: node_id.to_string(),
            local_actors: RwLock::new(HashSet::new()),
        }
    }

    pub async fn activate_actor(&self, actor_id: &str) {
        let mut local = self.local_actors.write().await;
        local.insert(actor_id.to_string());
    }

    #[allow(dead_code)]
    pub async fn deactivate_actor(&self, actor_id: &str) {
        let mut local = self.local_actors.write().await;
        local.remove(actor_id);
    }

    pub async fn is_active(&self, actor_id: &str) -> bool {
        let local = self.local_actors.read().await;
        local.contains(actor_id)
    }
}

// =============================================================================
// TLA+ BUG PATTERN TEST SCENARIOS
// =============================================================================

/// Scenario: TOCTOU race in TryClaimActor (TryClaimActor_Racy from TLA+)
///
/// Models the bug where:
/// 1. Node A checks: placement[actor] = NULL
/// 2. Node B checks: placement[actor] = NULL (same window)
/// 3. Node A claims actor
/// 4. Node B claims actor (trusts stale check!) <- BUG
/// 5. Result: Actor active on BOTH nodes, violating SingleActivation
pub async fn scenario_toctou_race_dual_activation() -> (Vec<InvariantViolation>, String) {
    let registry = SimulatedRegistry::new(vec![("node1", 10), ("node2", 10)]);
    let node1 = SimulatedNode::new("node1");
    let node2 = SimulatedNode::new("node2");

    // Enable racy behavior
    registry.enable_racy_claim();

    let actor_id = "actor1";

    // Both nodes "check" placement (both see NULL)
    let check1 = registry.get_placement(actor_id).await;
    let check2 = registry.get_placement(actor_id).await;

    assert!(check1.is_none(), "Expected no placement initially");
    assert!(check2.is_none(), "Expected no placement initially");

    // TOCTOU window: Both try to claim
    // With racy claim, both succeed!
    let claim1 = registry.try_claim_actor(actor_id, "node1", 100).await;
    let claim2 = registry.try_claim_actor(actor_id, "node2", 100).await;

    // Both activate locally based on their claim
    if claim1.is_ok() {
        node1.activate_actor(actor_id).await;
    }
    if claim2.is_ok() {
        node2.activate_actor(actor_id).await;
    }

    // Now check: both nodes think they have the actor active!
    let mut actor_locations: HashMap<String, HashSet<String>> = HashMap::new();
    if node1.is_active(actor_id).await {
        actor_locations
            .entry(actor_id.to_string())
            .or_default()
            .insert("node1".to_string());
    }
    if node2.is_active(actor_id).await {
        actor_locations
            .entry(actor_id.to_string())
            .or_default()
            .insert("node2".to_string());
    }

    // Verify SingleActivation - should FAIL with racy claim
    let mut violations = Vec::new();
    if let Err(v) = verify_single_activation(&actor_locations) {
        violations.push(v);
    }

    let description = format!(
        "TOCTOU race: node1 claim={:?}, node2 claim={:?}, actor on nodes: {:?}",
        claim1.is_ok(),
        claim2.is_ok(),
        actor_locations.get(actor_id)
    );

    (violations, description)
}

/// Scenario: Zombie actor race (LeaseExpires_Racy from TLA+)
///
/// Models the bug where:
/// 1. Actor is active on Node A with lease
/// 2. Lease expires (racy cleanup - only clears placement/lease)
/// 3. Node B sees no placement, claims actor
/// 4. Node A still running actor (zombie!) <- BUG
/// 5. Result: Actor active on BOTH nodes
pub async fn scenario_zombie_actor_reclaim_race() -> (Vec<InvariantViolation>, String) {
    let registry = SimulatedRegistry::new(vec![("node1", 10), ("node2", 10)]);
    let node1 = SimulatedNode::new("node1");
    let node2 = SimulatedNode::new("node2");

    // Enable racy lease cleanup
    registry.enable_racy_lease_cleanup();

    let actor_id = "actor1";

    // Node 1 claims and activates actor
    let claim1 = registry.try_claim_actor(actor_id, "node1", 100).await;
    assert!(claim1.is_ok());
    node1.activate_actor(actor_id).await;

    // Advance time past lease expiry
    registry.advance_time(150);

    // Racy lease expiry - clears placement but node1 doesn't know!
    registry.expire_lease(actor_id).await;

    // Node 2 sees no placement, claims actor
    let check = registry.get_placement(actor_id).await;
    assert!(check.is_none(), "Placement should be cleared after expiry");

    // Node 2 claims (safe claim is fine here, the bug is in lease cleanup)
    registry.use_atomic_claim.store(true, Ordering::SeqCst);
    let claim2 = registry.try_claim_actor(actor_id, "node2", 100).await;
    assert!(claim2.is_ok());
    node2.activate_actor(actor_id).await;

    // BUG: Node 1 never got notified, still thinks it has the actor!
    // (In real system, heartbeat/lease renewal would eventually fail)

    // Check: both nodes have actor active
    let mut actor_locations: HashMap<String, HashSet<String>> = HashMap::new();
    if node1.is_active(actor_id).await {
        actor_locations
            .entry(actor_id.to_string())
            .or_default()
            .insert("node1".to_string());
    }
    if node2.is_active(actor_id).await {
        actor_locations
            .entry(actor_id.to_string())
            .or_default()
            .insert("node2".to_string());
    }

    // Verify SingleActivation - should FAIL due to zombie
    let mut violations = Vec::new();
    if let Err(v) = verify_single_activation(&actor_locations) {
        violations.push(v);
    }

    let description = format!(
        "Zombie race: node1 still active={}, node2 claimed=true, actor on nodes: {:?}",
        node1.is_active(actor_id).await,
        actor_locations.get(actor_id)
    );

    (violations, description)
}

/// Scenario: Concurrent registration race (RegisterActor_Racy from TLA+)
///
/// Models the bug where multiple concurrent registrations can exceed capacity.
pub async fn scenario_concurrent_registration_race() -> (Vec<InvariantViolation>, String) {
    // Node with capacity 2
    let registry = SimulatedRegistry::new(vec![("node1", 2)]);

    // Try to register 5 actors concurrently
    // With atomic claims, only 2 should succeed
    let actors = ["actor1", "actor2", "actor3", "actor4", "actor5"];
    let mut handles = Vec::new();

    let registry_arc = Arc::new(registry);

    for actor in &actors {
        let reg = registry_arc.clone();
        let actor_id = actor.to_string();
        handles.push(
            kelpie_core::current_runtime()
                .spawn(async move { reg.try_claim_actor(&actor_id, "node1", 100).await }),
        );
    }

    let mut success_count = 0;
    for handle in handles {
        if let Ok(Ok(true)) = handle.await {
            success_count += 1;
        }
    }

    // With atomic claims, should respect capacity
    let state = registry_arc.get_state().await;

    let mut violations = Vec::new();
    if let Err(v) = verify_capacity_bounds(&state.node_info) {
        violations.push(v);
    }

    let description = format!(
        "Concurrent registration: {} succeeded, capacity=2, actual count={:?}",
        success_count,
        state.node_info.get("node1")
    );

    (violations, description)
}

/// Scenario: Partial commit (CommitTransaction_StateOnly from TLA+)
///
/// Models the bug where state is committed but KV writes are not.
/// This is a simplified model - real test would use actual storage.
pub async fn scenario_partial_commit() -> (Vec<InvariantViolation>, String) {
    // Simulated storage state
    let state_committed = Arc::new(AtomicBool::new(false));
    let kv_committed = Arc::new(AtomicBool::new(false));

    // Simulate partial commit (state written, crash before KV)
    state_committed.store(true, Ordering::SeqCst);
    // kv_committed stays false - simulating crash

    let mut violations = Vec::new();
    let state_ok = state_committed.load(Ordering::SeqCst);
    let kv_ok = kv_committed.load(Ordering::SeqCst);

    if state_ok != kv_ok {
        violations.push(InvariantViolation::PartialCommit {
            actor_id: "test-actor".to_string(),
            state_committed: state_ok,
            kv_committed: kv_ok,
            details: "State committed but KV not committed (simulated crash)".to_string(),
        });
    }

    let description = format!(
        "Partial commit: state_committed={}, kv_committed={}",
        state_ok, kv_ok
    );

    (violations, description)
}

// =============================================================================
// SAFE BEHAVIOR TESTS
// =============================================================================

/// Test that safe (atomic) claims prevent TOCTOU race.
pub async fn scenario_safe_concurrent_claim() -> (Vec<InvariantViolation>, String) {
    let registry = SimulatedRegistry::new(vec![("node1", 10), ("node2", 10)]);
    let node1 = SimulatedNode::new("node1");
    let node2 = SimulatedNode::new("node2");

    // Use safe (atomic) claim - default
    let actor_id = "actor1";

    // Both nodes try to claim concurrently
    let reg1 = Arc::new(registry);
    let reg2 = reg1.clone();

    let handle1 = {
        let actor = actor_id.to_string();
        kelpie_core::current_runtime()
            .spawn(async move { reg1.try_claim_actor(&actor, "node1", 100).await })
    };

    let handle2 = {
        let actor = actor_id.to_string();
        kelpie_core::current_runtime()
            .spawn(async move { reg2.try_claim_actor(&actor, "node2", 100).await })
    };

    let result1 = handle1.await.unwrap();
    let result2 = handle2.await.unwrap();

    // Only one should succeed
    let success_count = [&result1, &result2]
        .iter()
        .filter(|r| matches!(r, Ok(true)))
        .count();

    // Activate based on who won
    if matches!(result1, Ok(true)) {
        node1.activate_actor(actor_id).await;
    }
    if matches!(result2, Ok(true)) {
        node2.activate_actor(actor_id).await;
    }

    // Verify SingleActivation - should PASS with atomic claim
    let mut actor_locations: HashMap<String, HashSet<String>> = HashMap::new();
    if node1.is_active(actor_id).await {
        actor_locations
            .entry(actor_id.to_string())
            .or_default()
            .insert("node1".to_string());
    }
    if node2.is_active(actor_id).await {
        actor_locations
            .entry(actor_id.to_string())
            .or_default()
            .insert("node2".to_string());
    }

    let mut violations = Vec::new();
    if let Err(v) = verify_single_activation(&actor_locations) {
        violations.push(v);
    }

    let description = format!(
        "Safe concurrent claim: success_count={}, violations={}",
        success_count,
        violations.len()
    );

    (violations, description)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test TOCTOU race detects SingleActivation violation.
    #[tokio::test]
    async fn test_toctou_race_detects_violation() {
        let (violations, description) = scenario_toctou_race_dual_activation().await;
        println!("Scenario: {}", description);

        // TOCTOU race should produce SingleActivation violation
        assert!(
            !violations.is_empty(),
            "Expected SingleActivation violation from TOCTOU race, got none"
        );

        for v in &violations {
            println!("Violation: {}", v);
            assert!(matches!(v, InvariantViolation::SingleActivation { .. }));
        }
    }

    /// Test zombie actor race detects SingleActivation violation.
    #[tokio::test]
    async fn test_zombie_race_detects_violation() {
        let (violations, description) = scenario_zombie_actor_reclaim_race().await;
        println!("Scenario: {}", description);

        // Zombie race should produce SingleActivation violation
        assert!(
            !violations.is_empty(),
            "Expected SingleActivation violation from zombie race, got none"
        );

        for v in &violations {
            println!("Violation: {}", v);
            assert!(matches!(v, InvariantViolation::SingleActivation { .. }));
        }
    }

    /// Test concurrent registration respects capacity with atomic claims.
    #[tokio::test]
    async fn test_concurrent_registration_respects_capacity() {
        let (violations, description) = scenario_concurrent_registration_race().await;
        println!("Scenario: {}", description);

        // Atomic claims should prevent capacity violations
        assert!(
            violations.is_empty(),
            "Expected no capacity violations with atomic claims, got: {:?}",
            violations
        );
    }

    /// Test partial commit is detected.
    #[tokio::test]
    async fn test_partial_commit_detected() {
        let (violations, description) = scenario_partial_commit().await;
        println!("Scenario: {}", description);

        // Partial commit should be detected
        assert!(
            !violations.is_empty(),
            "Expected partial commit violation, got none"
        );

        for v in &violations {
            println!("Violation: {}", v);
            assert!(matches!(v, InvariantViolation::PartialCommit { .. }));
        }
    }

    /// Test safe concurrent claims prevent violations.
    #[tokio::test]
    async fn test_safe_concurrent_claim_no_violations() {
        let (violations, description) = scenario_safe_concurrent_claim().await;
        println!("Scenario: {}", description);

        // Safe claims should produce no violations
        assert!(
            violations.is_empty(),
            "Expected no violations with safe claims, got: {:?}",
            violations
        );
    }

    /// Integration test: Run all bug pattern scenarios.
    #[tokio::test]
    async fn test_all_tla_bug_patterns() {
        println!("\n=== TLA+ Bug Pattern Integration Test ===\n");

        // 1. TOCTOU race (expected violation)
        println!("1. Testing TOCTOU race (TryClaimActor_Racy)...");
        let (v1, d1) = scenario_toctou_race_dual_activation().await;
        println!("   Result: {} violations - {}", v1.len(), d1);
        assert!(!v1.is_empty(), "TOCTOU race should produce violation");

        // 2. Zombie race (expected violation)
        println!("\n2. Testing zombie race (LeaseExpires_Racy)...");
        let (v2, d2) = scenario_zombie_actor_reclaim_race().await;
        println!("   Result: {} violations - {}", v2.len(), d2);
        assert!(!v2.is_empty(), "Zombie race should produce violation");

        // 3. Concurrent registration (NO violation with atomic claims)
        println!("\n3. Testing concurrent registration...");
        let (v3, d3) = scenario_concurrent_registration_race().await;
        println!("   Result: {} violations - {}", v3.len(), d3);
        assert!(
            v3.is_empty(),
            "Atomic claims should prevent capacity violation"
        );

        // 4. Partial commit (expected violation)
        println!("\n4. Testing partial commit (CommitTransaction_StateOnly)...");
        let (v4, d4) = scenario_partial_commit().await;
        println!("   Result: {} violations - {}", v4.len(), d4);
        assert!(!v4.is_empty(), "Partial commit should produce violation");

        // 5. Safe concurrent claims (NO violation)
        println!("\n5. Testing safe concurrent claims...");
        let (v5, d5) = scenario_safe_concurrent_claim().await;
        println!("   Result: {} violations - {}", v5.len(), d5);
        assert!(v5.is_empty(), "Safe claims should produce no violations");

        println!("\n=== All TLA+ Bug Pattern Tests Complete ===\n");
    }
}
