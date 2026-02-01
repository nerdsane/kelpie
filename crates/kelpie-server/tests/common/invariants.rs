//! TLA+ Invariant Verification Helpers
//!
//! This module provides runtime verification of invariants defined in `docs/tla/`:
//! - KelpieSingleActivation.tla - Single activation guarantee
//! - KelpieRegistry.tla - Registry consistency
//! - KelpieActorState.tla - Transaction atomicity
//!
//! # Design Philosophy (TigerStyle)
//!
//! 1. **Explicit error types** - InvariantViolation captures exactly what went wrong
//! 2. **No silent failures** - Every check returns Result, never swallows errors
//! 3. **Evidence collection** - Violations include evidence for debugging
//! 4. **Composable** - verify_all_invariants() runs all applicable checks
//!
//! # Usage in DST Tests
//!
//! ```rust,ignore
//! #[tokio::test]
//! async fn test_concurrent_activation() {
//!     let config = SimConfig::from_env_or_random();
//!     println!("DST_SEED={}", config.seed);  // For reproduction
//!
//!     let result = Simulation::new(config)
//!         .with_fault(FaultConfig::new(FaultType::NetworkDelay, 0.5))
//!         .run_async(|env| async move {
//!             // ... test logic ...
//!
//!             // TLA+ invariant verification
//!             verify_single_activation(&registry).await?;
//!
//!             Ok(())
//!         }).await;
//!
//!     // TigerStyle: Explicit match, not just is_ok()
//!     match result {
//!         Ok(()) => println!("Test passed"),
//!         Err(e) => panic!("Test failed: {:?}", e),
//!     }
//! }
//! ```

use std::collections::{HashMap, HashSet};
use std::fmt;

/// Invariant violation with detailed evidence for debugging.
///
/// Following TigerStyle: explicit error types with full context.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Some variants are for future use
pub enum InvariantViolation {
    /// SingleActivation: More than one node has the same actor active.
    /// From TLA+: `Cardinality({n : actor \in localActors[n]}) <= 1`
    SingleActivation {
        actor_id: String,
        active_on_nodes: Vec<String>,
    },

    /// PlacementConsistency: Actor is active but placement points elsewhere.
    /// From TLA+: `actor \in localActors[node] => placements[actor] = node`
    PlacementInconsistency {
        actor_id: String,
        active_on: String,
        placement_points_to: Option<String>,
    },

    /// LeaseValidity: Actor is active but lease is expired or missing.
    /// From TLA+: `actor \in localActors[node] => leases[actor].expires > time`
    LeaseInvalid {
        actor_id: String,
        node_id: String,
        lease_expires_at: Option<u64>,
        current_time: u64,
    },

    /// CapacityBounds: Node has more actors than its capacity.
    /// From TLA+: `nodes[n].actor_count <= nodes[n].capacity`
    CapacityExceeded {
        node_id: String,
        actor_count: usize,
        capacity: usize,
    },

    /// CapacityConsistency: Placement count doesn't match actor_count.
    /// From TLA+: `Cardinality({a : placements[a] = n}) = nodes[n].actor_count`
    CapacityMismatch {
        node_id: String,
        placement_count: usize,
        reported_actor_count: usize,
    },

    /// TransactionAtomicity: Partial commit detected (state but not KV, or vice versa).
    PartialCommit {
        actor_id: String,
        state_committed: bool,
        kv_committed: bool,
        details: String,
    },

    /// CreateGetConsistency: Created entity not retrievable.
    CreateNotRetrievable {
        entity_type: String,
        entity_id: String,
        create_succeeded: bool,
        get_error: String,
    },

    /// DeleteGetConsistency: Deleted entity still retrievable.
    DeleteNotEffective {
        entity_type: String,
        entity_id: String,
        delete_succeeded: bool,
    },

    /// LeaseExclusivity: Valid lease but placement doesn't match.
    /// From TLA+: `leases[a].expires > time => placements[a] = leases[a].node`
    LeaseExclusivity {
        actor_id: String,
        lease_node: String,
        placement_node: Option<String>,
        lease_expires: u64,
        current_time: u64,
    },

    /// Generic invariant violation with custom message.
    Custom {
        invariant_name: String,
        message: String,
        evidence: Vec<(String, String)>,
    },
}

impl fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SingleActivation {
                actor_id,
                active_on_nodes,
            } => {
                write!(
                    f,
                    "SINGLE_ACTIVATION violated: actor '{}' active on {} nodes: {:?}",
                    actor_id,
                    active_on_nodes.len(),
                    active_on_nodes
                )
            }
            Self::PlacementInconsistency {
                actor_id,
                active_on,
                placement_points_to,
            } => {
                write!(
                    f,
                    "PLACEMENT_CONSISTENCY violated: actor '{}' active on '{}' but placement points to {:?}",
                    actor_id, active_on, placement_points_to
                )
            }
            Self::LeaseInvalid {
                actor_id,
                node_id,
                lease_expires_at,
                current_time,
            } => {
                write!(
                    f,
                    "LEASE_VALIDITY violated: actor '{}' active on '{}' but lease {:?} <= current time {}",
                    actor_id, node_id, lease_expires_at, current_time
                )
            }
            Self::CapacityExceeded {
                node_id,
                actor_count,
                capacity,
            } => {
                write!(
                    f,
                    "CAPACITY_BOUNDS violated: node '{}' has {} actors but capacity is {}",
                    node_id, actor_count, capacity
                )
            }
            Self::CapacityMismatch {
                node_id,
                placement_count,
                reported_actor_count,
            } => {
                write!(
                    f,
                    "CAPACITY_CONSISTENCY violated: node '{}' has {} placements but reports {} actor_count",
                    node_id, placement_count, reported_actor_count
                )
            }
            Self::PartialCommit {
                actor_id,
                state_committed,
                kv_committed,
                details,
            } => {
                write!(
                    f,
                    "TRANSACTION_ATOMICITY violated: actor '{}' state_committed={} kv_committed={}: {}",
                    actor_id, state_committed, kv_committed, details
                )
            }
            Self::CreateNotRetrievable {
                entity_type,
                entity_id,
                create_succeeded,
                get_error,
            } => {
                write!(
                    f,
                    "CREATE_GET_CONSISTENCY violated: {} '{}' create_succeeded={} but get failed: {}",
                    entity_type, entity_id, create_succeeded, get_error
                )
            }
            Self::DeleteNotEffective {
                entity_type,
                entity_id,
                delete_succeeded,
            } => {
                write!(
                    f,
                    "DELETE_GET_CONSISTENCY violated: {} '{}' delete_succeeded={} but still retrievable",
                    entity_type, entity_id, delete_succeeded
                )
            }
            Self::LeaseExclusivity {
                actor_id,
                lease_node,
                placement_node,
                lease_expires,
                current_time,
            } => {
                write!(
                    f,
                    "LEASE_EXCLUSIVITY violated: actor '{}' has valid lease (expires {} > {}) on '{}' but placement is {:?}",
                    actor_id, lease_expires, current_time, lease_node, placement_node
                )
            }
            Self::Custom {
                invariant_name,
                message,
                evidence,
            } => {
                write!(
                    f,
                    "{} violated: {} evidence={:?}",
                    invariant_name, message, evidence
                )
            }
        }
    }
}

impl std::error::Error for InvariantViolation {}

/// Result type for invariant verification.
pub type InvariantResult<T> = Result<T, InvariantViolation>;

// =============================================================================
// VERIFICATION HELPERS
// =============================================================================

/// Verify SingleActivation invariant.
///
/// From TLA+ KelpieSingleActivation.tla:
/// ```tla
/// SingleActivation ==
///     \A actor \in ActorIds:
///         Cardinality({node \in NodeIds : actor \in localActors[node]}) <= 1
/// ```
///
/// # Arguments
/// * `actor_locations` - Map of actor_id -> set of node_ids where it's active
///
/// # Returns
/// * `Ok(())` if invariant holds
/// * `Err(InvariantViolation::SingleActivation)` if any actor is active on multiple nodes
pub fn verify_single_activation(
    actor_locations: &HashMap<String, HashSet<String>>,
) -> InvariantResult<()> {
    for (actor_id, nodes) in actor_locations {
        if nodes.len() > 1 {
            return Err(InvariantViolation::SingleActivation {
                actor_id: actor_id.clone(),
                active_on_nodes: nodes.iter().cloned().collect(),
            });
        }
    }
    Ok(())
}

/// Verify PlacementConsistency invariant.
///
/// From TLA+ KelpieSingleActivation.tla:
/// ```tla
/// PlacementConsistency ==
///     \A actor \in ActorIds:
///         \A node \in NodeIds:
///             actor \in localActors[node] => placements[actor] = node
/// ```
///
/// # Arguments
/// * `active_actors` - Map of node_id -> set of actor_ids active on that node
/// * `placements` - Map of actor_id -> node_id (the official placement)
pub fn verify_placement_consistency(
    active_actors: &HashMap<String, HashSet<String>>,
    placements: &HashMap<String, String>,
) -> InvariantResult<()> {
    for (node_id, actors) in active_actors {
        for actor_id in actors {
            let placement = placements.get(actor_id);
            if placement != Some(node_id) {
                return Err(InvariantViolation::PlacementInconsistency {
                    actor_id: actor_id.clone(),
                    active_on: node_id.clone(),
                    placement_points_to: placement.cloned(),
                });
            }
        }
    }
    Ok(())
}

/// Verify LeaseValidity invariant.
///
/// From TLA+ KelpieSingleActivation.tla:
/// ```tla
/// LeaseValidityIfActive ==
///     \A actor \in ActorIds:
///         \A node \in NodeIds:
///             actor \in localActors[node] =>
///                 /\ leases[actor] # NULL
///                 /\ leases[actor].node = node
///                 /\ leases[actor].expires > time
/// ```
///
/// # Arguments
/// * `active_actors` - Map of node_id -> set of actor_ids active on that node
/// * `leases` - Map of actor_id -> (lease_node, expires_at)
/// * `current_time` - Current logical time
pub fn verify_lease_validity(
    active_actors: &HashMap<String, HashSet<String>>,
    leases: &HashMap<String, (String, u64)>,
    current_time: u64,
) -> InvariantResult<()> {
    for (node_id, actors) in active_actors {
        for actor_id in actors {
            match leases.get(actor_id) {
                None => {
                    return Err(InvariantViolation::LeaseInvalid {
                        actor_id: actor_id.clone(),
                        node_id: node_id.clone(),
                        lease_expires_at: None,
                        current_time,
                    });
                }
                Some((lease_node, expires_at)) => {
                    // Check lease node matches
                    if lease_node != node_id {
                        return Err(InvariantViolation::LeaseInvalid {
                            actor_id: actor_id.clone(),
                            node_id: node_id.clone(),
                            lease_expires_at: Some(*expires_at),
                            current_time,
                        });
                    }
                    // Check not expired
                    if *expires_at <= current_time {
                        return Err(InvariantViolation::LeaseInvalid {
                            actor_id: actor_id.clone(),
                            node_id: node_id.clone(),
                            lease_expires_at: Some(*expires_at),
                            current_time,
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

/// Verify CapacityBounds invariant.
///
/// From TLA+ KelpieRegistry.tla:
/// ```tla
/// CapacityBounds ==
///     \A n \in NodeIds:
///         nodes[n] # NULL =>
///             /\ nodes[n].actor_count >= 0
///             /\ nodes[n].actor_count <= nodes[n].capacity
/// ```
///
/// # Arguments
/// * `node_info` - Map of node_id -> (actor_count, capacity)
pub fn verify_capacity_bounds(node_info: &HashMap<String, (usize, usize)>) -> InvariantResult<()> {
    for (node_id, (actor_count, capacity)) in node_info {
        if *actor_count > *capacity {
            return Err(InvariantViolation::CapacityExceeded {
                node_id: node_id.clone(),
                actor_count: *actor_count,
                capacity: *capacity,
            });
        }
    }
    Ok(())
}

/// Verify CapacityConsistency invariant.
///
/// From TLA+ KelpieRegistry.tla:
/// ```tla
/// CapacityConsistency ==
///     \A n \in NodeIds:
///         nodes[n] # NULL =>
///             Cardinality({a \in ActorIds : placements[a] = n}) = nodes[n].actor_count
/// ```
///
/// # Arguments
/// * `placements` - Map of actor_id -> node_id
/// * `node_actor_counts` - Map of node_id -> reported actor_count
pub fn verify_capacity_consistency(
    placements: &HashMap<String, String>,
    node_actor_counts: &HashMap<String, usize>,
) -> InvariantResult<()> {
    // Count placements per node
    let mut placement_counts: HashMap<String, usize> = HashMap::new();
    for node_id in placements.values() {
        *placement_counts.entry(node_id.clone()).or_insert(0) += 1;
    }

    // Verify counts match
    for (node_id, reported_count) in node_actor_counts {
        let actual_count = placement_counts.get(node_id).copied().unwrap_or(0);
        if actual_count != *reported_count {
            return Err(InvariantViolation::CapacityMismatch {
                node_id: node_id.clone(),
                placement_count: actual_count,
                reported_actor_count: *reported_count,
            });
        }
    }

    Ok(())
}

/// Verify LeaseExclusivity invariant.
///
/// From TLA+ KelpieRegistry.tla:
/// ```tla
/// LeaseExclusivity ==
///     \A a \in ActorIds:
///         leases[a] # NULL /\ leases[a].expires > time =>
///             placements[a] = leases[a].node
/// ```
///
/// # Arguments
/// * `leases` - Map of actor_id -> (lease_node, expires_at)
/// * `placements` - Map of actor_id -> node_id
/// * `current_time` - Current logical time
pub fn verify_lease_exclusivity(
    leases: &HashMap<String, (String, u64)>,
    placements: &HashMap<String, String>,
    current_time: u64,
) -> InvariantResult<()> {
    for (actor_id, (lease_node, expires_at)) in leases {
        // Only check valid (non-expired) leases
        if *expires_at > current_time {
            let placement = placements.get(actor_id);
            if placement != Some(lease_node) {
                return Err(InvariantViolation::LeaseExclusivity {
                    actor_id: actor_id.clone(),
                    lease_node: lease_node.clone(),
                    placement_node: placement.cloned(),
                    lease_expires: *expires_at,
                    current_time,
                });
            }
        }
    }
    Ok(())
}

// =============================================================================
// COMPOSITE VERIFICATION
// =============================================================================

/// State snapshot for comprehensive invariant verification.
#[derive(Debug, Clone, Default)]
pub struct SystemState {
    /// node_id -> set of actor_ids active on that node
    pub active_actors: HashMap<String, HashSet<String>>,
    /// actor_id -> node_id (official placement)
    pub placements: HashMap<String, String>,
    /// actor_id -> (lease_node, expires_at)
    pub leases: HashMap<String, (String, u64)>,
    /// node_id -> (actor_count, capacity)
    pub node_info: HashMap<String, (usize, usize)>,
    /// Current logical time
    pub current_time: u64,
}

impl SystemState {
    /// Create a new empty system state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Derive actor_locations from active_actors for SingleActivation check.
    pub fn actor_locations(&self) -> HashMap<String, HashSet<String>> {
        let mut locations: HashMap<String, HashSet<String>> = HashMap::new();
        for (node_id, actors) in &self.active_actors {
            for actor_id in actors {
                locations
                    .entry(actor_id.clone())
                    .or_default()
                    .insert(node_id.clone());
            }
        }
        locations
    }
}

/// Verify all core invariants against a system state snapshot.
///
/// Returns the first violation found, or Ok(()) if all pass.
#[allow(dead_code)]
pub fn verify_core_invariants(state: &SystemState) -> InvariantResult<()> {
    // 1. SingleActivation
    let actor_locations = state.actor_locations();
    verify_single_activation(&actor_locations)?;

    // 2. PlacementConsistency
    verify_placement_consistency(&state.active_actors, &state.placements)?;

    // 3. LeaseValidity (if leases are tracked)
    if !state.leases.is_empty() {
        verify_lease_validity(&state.active_actors, &state.leases, state.current_time)?;
    }

    // 4. CapacityBounds
    verify_capacity_bounds(&state.node_info)?;

    // 5. CapacityConsistency
    let node_counts: HashMap<String, usize> = state
        .node_info
        .iter()
        .map(|(k, (count, _))| (k.clone(), *count))
        .collect();
    verify_capacity_consistency(&state.placements, &node_counts)?;

    // 6. LeaseExclusivity
    if !state.leases.is_empty() {
        verify_lease_exclusivity(&state.leases, &state.placements, state.current_time)?;
    }

    Ok(())
}

/// Verify all invariants and collect all violations (not just the first).
pub fn verify_all_invariants(state: &SystemState) -> Vec<InvariantViolation> {
    let mut violations = Vec::new();

    // 1. SingleActivation
    let actor_locations = state.actor_locations();
    if let Err(v) = verify_single_activation(&actor_locations) {
        violations.push(v);
    }

    // 2. PlacementConsistency
    if let Err(v) = verify_placement_consistency(&state.active_actors, &state.placements) {
        violations.push(v);
    }

    // 3. LeaseValidity
    if !state.leases.is_empty() {
        if let Err(v) =
            verify_lease_validity(&state.active_actors, &state.leases, state.current_time)
        {
            violations.push(v);
        }
    }

    // 4. CapacityBounds
    if let Err(v) = verify_capacity_bounds(&state.node_info) {
        violations.push(v);
    }

    // 5. CapacityConsistency
    let node_counts: HashMap<String, usize> = state
        .node_info
        .iter()
        .map(|(k, (count, _))| (k.clone(), *count))
        .collect();
    if let Err(v) = verify_capacity_consistency(&state.placements, &node_counts) {
        violations.push(v);
    }

    // 6. LeaseExclusivity
    if !state.leases.is_empty() {
        if let Err(v) =
            verify_lease_exclusivity(&state.leases, &state.placements, state.current_time)
        {
            violations.push(v);
        }
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_activation_passes_when_unique() {
        let mut locations = HashMap::new();
        locations.insert("actor1".to_string(), HashSet::from(["node1".to_string()]));
        locations.insert("actor2".to_string(), HashSet::from(["node2".to_string()]));

        assert!(verify_single_activation(&locations).is_ok());
    }

    #[test]
    fn test_single_activation_fails_when_duplicate() {
        let mut locations = HashMap::new();
        locations.insert(
            "actor1".to_string(),
            HashSet::from(["node1".to_string(), "node2".to_string()]),
        );

        let result = verify_single_activation(&locations);
        assert!(matches!(
            result,
            Err(InvariantViolation::SingleActivation { .. })
        ));
    }

    #[test]
    fn test_capacity_bounds_passes() {
        let mut info = HashMap::new();
        info.insert("node1".to_string(), (5, 10)); // 5 actors, capacity 10
        info.insert("node2".to_string(), (10, 10)); // At capacity

        assert!(verify_capacity_bounds(&info).is_ok());
    }

    #[test]
    fn test_capacity_bounds_fails_when_exceeded() {
        let mut info = HashMap::new();
        info.insert("node1".to_string(), (11, 10)); // Over capacity!

        let result = verify_capacity_bounds(&info);
        assert!(matches!(
            result,
            Err(InvariantViolation::CapacityExceeded { .. })
        ));
    }

    #[test]
    fn test_lease_validity_fails_when_expired() {
        let mut active = HashMap::new();
        active.insert("node1".to_string(), HashSet::from(["actor1".to_string()]));

        let mut leases = HashMap::new();
        leases.insert("actor1".to_string(), ("node1".to_string(), 100)); // Expires at 100

        // Current time is 150 - lease expired
        let result = verify_lease_validity(&active, &leases, 150);
        assert!(matches!(
            result,
            Err(InvariantViolation::LeaseInvalid { .. })
        ));
    }

    #[test]
    fn test_verify_all_invariants_collects_multiple_violations() {
        let mut state = SystemState::new();

        // Add actor on two nodes (SingleActivation violation)
        state
            .active_actors
            .insert("node1".to_string(), HashSet::from(["actor1".to_string()]));
        state
            .active_actors
            .insert("node2".to_string(), HashSet::from(["actor1".to_string()]));

        // Node over capacity (CapacityBounds violation)
        state.node_info.insert("node1".to_string(), (11, 10));
        state.node_info.insert("node2".to_string(), (5, 10));

        let violations = verify_all_invariants(&state);
        assert!(violations.len() >= 2, "Expected multiple violations");
    }
}
