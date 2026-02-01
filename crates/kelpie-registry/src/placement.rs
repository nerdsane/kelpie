//! Actor placement tracking
//!
//! TigerStyle: Explicit placement with single activation guarantee.

use crate::error::{RegistryError, RegistryResult};
use crate::node::NodeId;
use kelpie_core::actor::ActorId;
use kelpie_core::io::{TimeProvider, WallClockTime};
use serde::{Deserialize, Serialize};

/// Information about where an actor is placed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorPlacement {
    /// The actor's unique identifier
    pub actor_id: ActorId,
    /// The node where the actor is currently active
    pub node_id: NodeId,
    /// When the actor was activated (Unix timestamp ms)
    pub activated_at_ms: u64,
    /// When this placement was last updated (Unix timestamp ms)
    pub updated_at_ms: u64,
    /// Generation/version for detecting stale placements
    pub generation: u64,
}

impl ActorPlacement {
    /// Create a new placement record using production wall clock
    ///
    /// For DST, use `with_timestamp` or `new_with_time`.
    pub fn new(actor_id: ActorId, node_id: NodeId) -> Self {
        Self::new_with_time(actor_id, node_id, &WallClockTime::new())
    }

    /// Create a new placement record with injected time provider (for DST)
    pub fn new_with_time(actor_id: ActorId, node_id: NodeId, time: &dyn TimeProvider) -> Self {
        Self::with_timestamp(actor_id, node_id, time.now_ms())
    }

    /// Create a placement with a specific timestamp (for testing/simulation)
    pub fn with_timestamp(actor_id: ActorId, node_id: NodeId, timestamp_ms: u64) -> Self {
        Self {
            actor_id,
            node_id,
            activated_at_ms: timestamp_ms,
            updated_at_ms: timestamp_ms,
            generation: 1,
        }
    }

    /// Update the node for this placement (for migration)
    pub fn migrate_to(&mut self, new_node: NodeId, timestamp_ms: u64) {
        debug_assert!(timestamp_ms >= self.updated_at_ms);
        self.node_id = new_node;
        self.updated_at_ms = timestamp_ms;
        self.generation = self.generation.saturating_add(1);
    }

    /// Check if this placement is stale compared to another
    pub fn is_stale(&self, other: &ActorPlacement) -> bool {
        debug_assert_eq!(self.actor_id, other.actor_id);
        self.generation < other.generation
    }

    /// Touch the placement to update the timestamp
    pub fn touch(&mut self, timestamp_ms: u64) {
        debug_assert!(timestamp_ms >= self.updated_at_ms);
        self.updated_at_ms = timestamp_ms;
    }
}

/// Strategy for selecting a node for actor placement
#[derive(Debug, Clone, Copy, Default)]
pub enum PlacementStrategy {
    /// Place on node with least actors (load balancing)
    #[default]
    LeastLoaded,
    /// Place on random available node
    Random,
    /// Place on specific node (for affinity)
    Affinity,
    /// Round-robin across nodes
    RoundRobin,
}

/// Context for making placement decisions
#[derive(Debug, Clone)]
pub struct PlacementContext {
    /// The actor to place
    pub actor_id: ActorId,
    /// Preferred node (for affinity)
    pub preferred_node: Option<NodeId>,
    /// Strategy to use
    pub strategy: PlacementStrategy,
}

impl PlacementContext {
    /// Create a new placement context
    pub fn new(actor_id: ActorId) -> Self {
        Self {
            actor_id,
            preferred_node: None,
            strategy: PlacementStrategy::default(),
        }
    }

    /// Set preferred node
    pub fn with_preferred_node(mut self, node_id: NodeId) -> Self {
        self.preferred_node = Some(node_id);
        self.strategy = PlacementStrategy::Affinity;
        self
    }

    /// Set placement strategy
    pub fn with_strategy(mut self, strategy: PlacementStrategy) -> Self {
        self.strategy = strategy;
        self
    }
}

/// Result of a placement decision
#[derive(Debug, Clone)]
pub enum PlacementDecision {
    /// Actor is already placed on a node
    Existing(ActorPlacement),
    /// Actor should be placed on this node
    New(NodeId),
    /// No suitable node available for placement
    NoCapacity,
}

/// Validation for placement operations
pub fn validate_placement(
    actor_id: &ActorId,
    existing: Option<&ActorPlacement>,
    requested_node: &NodeId,
) -> RegistryResult<()> {
    // TigerStyle: Explicit validation with 2+ assertions
    debug_assert!(!actor_id.id().is_empty());
    debug_assert!(!requested_node.as_str().is_empty());

    // Check for conflict
    if let Some(existing) = existing {
        if existing.node_id != *requested_node {
            return Err(RegistryError::actor_already_registered(
                actor_id,
                existing.node_id.as_str(),
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_actor_id() -> ActorId {
        ActorId::new("test", "actor-1").unwrap()
    }

    fn test_node_id() -> NodeId {
        NodeId::new("node-1").unwrap()
    }

    #[test]
    fn test_actor_placement_new() {
        let placement = ActorPlacement::new(test_actor_id(), test_node_id());

        assert_eq!(placement.actor_id, test_actor_id());
        assert_eq!(placement.node_id, test_node_id());
        assert_eq!(placement.generation, 1);
    }

    #[test]
    fn test_actor_placement_migrate() {
        let mut placement = ActorPlacement::with_timestamp(test_actor_id(), test_node_id(), 1000);

        let new_node = NodeId::new("node-2").unwrap();
        placement.migrate_to(new_node.clone(), 2000);

        assert_eq!(placement.node_id, new_node);
        assert_eq!(placement.updated_at_ms, 2000);
        assert_eq!(placement.generation, 2);
    }

    #[test]
    fn test_actor_placement_stale() {
        let p1 = ActorPlacement::with_timestamp(test_actor_id(), test_node_id(), 1000);
        let mut p2 = ActorPlacement::with_timestamp(test_actor_id(), test_node_id(), 1000);
        p2.generation = 2;

        assert!(p1.is_stale(&p2));
        assert!(!p2.is_stale(&p1));
    }

    #[test]
    fn test_placement_context() {
        let ctx = PlacementContext::new(test_actor_id());
        assert!(ctx.preferred_node.is_none());
        assert!(matches!(ctx.strategy, PlacementStrategy::LeastLoaded));

        let ctx = ctx.with_preferred_node(test_node_id());
        assert!(ctx.preferred_node.is_some());
        assert!(matches!(ctx.strategy, PlacementStrategy::Affinity));
    }

    #[test]
    fn test_validate_placement_no_conflict() {
        let result = validate_placement(&test_actor_id(), None, &test_node_id());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_placement_same_node() {
        let placement = ActorPlacement::new(test_actor_id(), test_node_id());
        let result = validate_placement(&test_actor_id(), Some(&placement), &test_node_id());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_placement_conflict() {
        let placement = ActorPlacement::new(test_actor_id(), test_node_id());
        let other_node = NodeId::new("node-2").unwrap();
        let result = validate_placement(&test_actor_id(), Some(&placement), &other_node);
        assert!(matches!(
            result,
            Err(RegistryError::ActorAlreadyRegistered { .. })
        ));
    }
}
