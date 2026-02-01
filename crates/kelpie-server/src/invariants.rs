//! System Invariants for Kelpie
//!
//! These invariants are derived from TLA+ specifications in `docs/tla/`:
//! - KelpieSingleActivation.tla - Actor placement and lifecycle
//! - KelpieRegistry.tla - Registry and capacity management
//! - KelpieActorState.tla - Transaction and state consistency
//!
//! Each invariant can be verified in tests using the helpers in `tests/common/invariants.rs`.

/// Invariant: At most one active instance per ActorId across all nodes.
///
/// From TLA+ spec: `SingleActivation == \A actor \in ActorIds: Cardinality({n : actor \in localActors[n]}) <= 1`
///
/// This is THE core guarantee of Kelpie's virtual actor model.
/// Violations indicate TOCTOU race in placement or zombie cleanup failure.
pub const SINGLE_ACTIVATION: &str = "SINGLE_ACTIVATION";

/// Invariant: If an actor is active, its placement points to the correct node.
///
/// From TLA+ spec: `actor \in localActors[node] => placements[actor] = node`
///
/// Note: Temporarily violated during lease expiry (zombie state) until cleanup.
pub const PLACEMENT_CONSISTENCY: &str = "PLACEMENT_CONSISTENCY";

/// Invariant: Active actors have valid (non-expired) leases.
///
/// From TLA+ spec: `actor \in localActors[node] => leases[actor].expires > time`
///
/// Lease renewal must happen before expiry to maintain this invariant.
pub const LEASE_VALIDITY: &str = "LEASE_VALIDITY";

/// Invariant: If create returns Ok(entity), get(entity.id) must succeed.
///
/// This is a fundamental consistency guarantee. If violated, indicates
/// partial write or transaction atomicity failure.
pub const CREATE_GET_CONSISTENCY: &str = "CREATE_GET_CONSISTENCY";

/// Invariant: If delete returns Ok, get must return NotFound.
///
/// Complementary to CREATE_GET_CONSISTENCY. Violations indicate
/// incomplete deletion or orphaned data.
pub const DELETE_GET_CONSISTENCY: &str = "DELETE_GET_CONSISTENCY";

/// Invariant: At most one invocation per actor at any time.
///
/// From TLA+ spec: `Cardinality({inv : inv.actor = a /\ inv.phase # "Done"}) <= 1`
///
/// Enforced by single-threaded actor mailbox processing.
pub const SINGLE_INVOCATION: &str = "SINGLE_INVOCATION";

/// Invariant: Commit is all-or-nothing (state + KV writes).
///
/// From TLA+ spec: `TransactionAtomicity` - either all changes visible or none.
///
/// Violations indicate partial writes or transaction boundary issues.
pub const TRANSACTION_ATOMICITY: &str = "TRANSACTION_ATOMICITY";

/// Invariant: Deactivating actors don't accept new invocations.
///
/// From TLA+ spec: `inv.phase \in {"PreSnapshot", "Executing"} => actorStatus[inv.actor] = "Active"`
///
/// Prevents orphaned state writes during shutdown.
pub const NO_ORPHANED_WRITES: &str = "NO_ORPHANED_WRITES";

/// Invariant: Actor count never exceeds node capacity.
///
/// From TLA+ spec: `nodes[n].actor_count <= nodes[n].capacity`
///
/// Registry must enforce capacity limits during placement.
pub const CAPACITY_BOUNDS: &str = "CAPACITY_BOUNDS";

/// Invariant: Sum of placements on node equals actor_count.
///
/// From TLA+ spec: `Cardinality({a : placements[a] = n}) = nodes[n].actor_count`
///
/// Violations indicate counter drift or placement tracking bug.
pub const CAPACITY_CONSISTENCY: &str = "CAPACITY_CONSISTENCY";

/// Invariant: Valid lease implies placement matches lease.node.
///
/// From TLA+ spec: `leases[a].expires > time => placements[a] = leases[a].node`
///
/// Ensures lease and placement are always in sync.
pub const LEASE_EXCLUSIVITY: &str = "LEASE_EXCLUSIVITY";

/// All core invariants that should be checked in comprehensive tests.
pub const CORE_INVARIANTS: &[&str] = &[
    SINGLE_ACTIVATION,
    CREATE_GET_CONSISTENCY,
    DELETE_GET_CONSISTENCY,
    TRANSACTION_ATOMICITY,
    CAPACITY_BOUNDS,
];

/// Invariants that may be temporarily violated during normal operation.
/// These should be checked after operations complete, not during.
pub const EVENTUALLY_CONSISTENT_INVARIANTS: &[&str] = &[
    PLACEMENT_CONSISTENCY, // Violated during zombie cleanup
    LEASE_VALIDITY,        // Violated between expiry and cleanup
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invariant_names_are_unique() {
        let all = [
            SINGLE_ACTIVATION,
            PLACEMENT_CONSISTENCY,
            LEASE_VALIDITY,
            CREATE_GET_CONSISTENCY,
            DELETE_GET_CONSISTENCY,
            SINGLE_INVOCATION,
            TRANSACTION_ATOMICITY,
            NO_ORPHANED_WRITES,
            CAPACITY_BOUNDS,
            CAPACITY_CONSISTENCY,
            LEASE_EXCLUSIVITY,
        ];

        let mut seen = std::collections::HashSet::new();
        for inv in all {
            assert!(seen.insert(inv), "Duplicate invariant name: {}", inv);
        }
    }

    #[test]
    fn core_invariants_are_defined() {
        assert!(!CORE_INVARIANTS.is_empty());
        assert!(CORE_INVARIANTS.contains(&SINGLE_ACTIVATION));
        assert!(CORE_INVARIANTS.contains(&TRANSACTION_ATOMICITY));
    }
}
