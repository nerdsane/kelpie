//! Cluster Membership Types
//!
//! Types for distributed cluster membership protocol based on TLA+ specification
//! from `docs/tla/KelpieClusterMembership.tla`.
//!
//! TigerStyle: Explicit states matching TLA+, explicit term-based election.

use crate::node::NodeId;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// =============================================================================
// Constants
// =============================================================================

/// Maximum view number (bounds state space, matches TLA+ MaxViewNum)
pub const MEMBERSHIP_VIEW_NUMBER_MAX: u64 = 1_000_000;

/// Maximum primary term (bounds state space)
pub const PRIMARY_TERM_MAX: u64 = 1_000_000;

/// Heartbeat interval for failure detection in milliseconds
pub const HEARTBEAT_INTERVAL_MS: u64 = 1_000;

/// Number of missed heartbeats before marking node as Suspect
pub const HEARTBEAT_SUSPECT_THRESHOLD: u64 = 3;

/// Number of missed heartbeats before marking node as Failed
pub const HEARTBEAT_FAILURE_THRESHOLD: u64 = 5;

// =============================================================================
// NodeState (matches TLA+ exactly)
// =============================================================================

/// Node state in the cluster membership protocol.
///
/// States match TLA+ specification from KelpieClusterMembership.tla:
/// - Left: Node not in cluster
/// - Joining: Node is joining cluster
/// - Active: Node is active cluster member
/// - Leaving: Node is gracefully leaving
/// - Failed: Node detected as failed
///
/// State transitions:
/// ```text
/// Left ──join──> Joining ──complete──> Active ──leave──> Leaving ──complete──> Left
///                                         │                                      ▲
///                                         │ failure detected                     │
///                                         ▼                                      │
///                                      Failed ──recover─────────────────────────┘
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    /// Node not in cluster (initial and final state)
    #[default]
    Left,
    /// Node is joining the cluster
    Joining,
    /// Node is active cluster member
    Active,
    /// Node is gracefully leaving
    Leaving,
    /// Node detected as failed
    Failed,
}

impl NodeState {
    /// Check if this node can accept new actor activations
    pub fn can_accept_actors(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// Check if this node is considered healthy
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// Check if this node should be removed from membership view
    pub fn should_remove_from_view(&self) -> bool {
        matches!(self, Self::Failed | Self::Left | Self::Leaving)
    }

    /// Check if the transition from current state to new state is valid
    ///
    /// TLA+ valid transitions:
    /// - Left -> Joining (NodeJoin)
    /// - Left -> Active (first node join - NodeJoin)
    /// - Joining -> Active (NodeJoinComplete)
    /// - Active -> Leaving (NodeLeave)
    /// - Active -> Failed (MarkNodeFailed)
    /// - Leaving -> Left (NodeLeaveComplete)
    /// - Failed -> Left (NodeRecover)
    pub fn can_transition_to(&self, new_state: NodeState) -> bool {
        matches!(
            (self, new_state),
            // Normal join flow
            (NodeState::Left, NodeState::Joining)
                | (NodeState::Left, NodeState::Active) // first node
                | (NodeState::Joining, NodeState::Active)
                // Normal leave flow
                | (NodeState::Active, NodeState::Leaving)
                | (NodeState::Leaving, NodeState::Left)
                // Failure flow
                | (NodeState::Active, NodeState::Failed)
                | (NodeState::Failed, NodeState::Left)
                // Joining node can also fail before becoming active
                | (NodeState::Joining, NodeState::Failed)
        )
    }

    /// Validate and perform a state transition
    ///
    /// # Panics
    /// Panics if the transition is invalid (TigerStyle: fail fast on invariant violation)
    pub fn transition_to(&mut self, new_state: NodeState) {
        assert!(
            self.can_transition_to(new_state),
            "invalid state transition from {:?} to {:?}",
            self,
            new_state
        );
        *self = new_state;
    }
}

impl std::fmt::Display for NodeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Left => write!(f, "left"),
            Self::Joining => write!(f, "joining"),
            Self::Active => write!(f, "active"),
            Self::Leaving => write!(f, "leaving"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

// =============================================================================
// PrimaryInfo
// =============================================================================

/// Information about the cluster primary.
///
/// The primary coordinates cluster operations. Uses Raft-style monotonically
/// increasing terms for conflict resolution.
///
/// TigerStyle: Explicit term for ordering, explicit timestamps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrimaryInfo {
    /// Node ID of the primary
    pub node_id: NodeId,
    /// Primary term (epoch number, monotonically increasing)
    pub term: u64,
    /// When this node became primary (Unix timestamp ms)
    pub elected_at_ms: u64,
}

impl PrimaryInfo {
    /// Create new primary info
    pub fn new(node_id: NodeId, term: u64, elected_at_ms: u64) -> Self {
        assert!(term > 0, "primary term must be positive");
        assert!(term <= PRIMARY_TERM_MAX, "primary term exceeds maximum");

        Self {
            node_id,
            term,
            elected_at_ms,
        }
    }

    /// Check if this primary has a higher term than another
    pub fn has_higher_term_than(&self, other: &PrimaryInfo) -> bool {
        self.term > other.term
    }

    /// Check if this primary is the same node
    pub fn is_same_node(&self, node_id: &NodeId) -> bool {
        &self.node_id == node_id
    }
}

// =============================================================================
// MembershipView
// =============================================================================

/// View of active cluster members.
///
/// Each node maintains its own view of which nodes are active.
/// Views are synchronized via FDB with view numbers for consistency.
///
/// TLA+ invariant: Active nodes with the same view number have the same membership view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipView {
    /// Set of active node IDs in this view
    pub active_nodes: HashSet<NodeId>,
    /// View number (monotonically increasing)
    pub view_number: u64,
    /// When this view was created (Unix timestamp ms)
    pub created_at_ms: u64,
}

impl MembershipView {
    /// Create a new membership view
    pub fn new(active_nodes: HashSet<NodeId>, view_number: u64, created_at_ms: u64) -> Self {
        assert!(
            view_number <= MEMBERSHIP_VIEW_NUMBER_MAX,
            "view number exceeds maximum"
        );

        Self {
            active_nodes,
            view_number,
            created_at_ms,
        }
    }

    /// Create an empty view (for new nodes)
    pub fn empty() -> Self {
        Self {
            active_nodes: HashSet::new(),
            view_number: 0,
            created_at_ms: 0,
        }
    }

    /// Check if this view has a higher view number
    pub fn has_higher_view_than(&self, other: &MembershipView) -> bool {
        self.view_number > other.view_number
    }

    /// Check if a node is in this view
    pub fn contains(&self, node_id: &NodeId) -> bool {
        self.active_nodes.contains(node_id)
    }

    /// Get the number of nodes in the view
    pub fn size(&self) -> usize {
        self.active_nodes.len()
    }

    /// Calculate quorum size (strict majority)
    pub fn quorum_size(&self) -> usize {
        (self.active_nodes.len() / 2) + 1
    }

    /// Check if the given count constitutes a quorum
    pub fn is_quorum(&self, count: usize) -> bool {
        count >= self.quorum_size()
    }

    /// Add a node to the view (creates new view with incremented number)
    pub fn with_node_added(&self, node_id: NodeId, now_ms: u64) -> Self {
        let mut new_nodes = self.active_nodes.clone();
        new_nodes.insert(node_id);

        Self {
            active_nodes: new_nodes,
            view_number: self.view_number + 1,
            created_at_ms: now_ms,
        }
    }

    /// Remove a node from the view (creates new view with incremented number)
    pub fn with_node_removed(&self, node_id: &NodeId, now_ms: u64) -> Self {
        let mut new_nodes = self.active_nodes.clone();
        new_nodes.remove(node_id);

        Self {
            active_nodes: new_nodes,
            view_number: self.view_number + 1,
            created_at_ms: now_ms,
        }
    }

    /// Merge two views (for partition healing)
    ///
    /// Takes union of nodes and higher view number + 1
    pub fn merge(&self, other: &MembershipView, now_ms: u64) -> Self {
        let merged_nodes: HashSet<NodeId> = self
            .active_nodes
            .union(&other.active_nodes)
            .cloned()
            .collect();
        let new_view_number = std::cmp::max(self.view_number, other.view_number) + 1;

        Self {
            active_nodes: merged_nodes,
            view_number: new_view_number,
            created_at_ms: now_ms,
        }
    }
}

impl Default for MembershipView {
    fn default() -> Self {
        Self::empty()
    }
}

// =============================================================================
// ClusterState
// =============================================================================

/// Full cluster state for a node (used for DST invariant checking)
#[derive(Debug, Clone)]
pub struct ClusterState {
    /// This node's ID
    pub node_id: NodeId,
    /// This node's state
    pub state: NodeState,
    /// This node's membership view
    pub view: MembershipView,
    /// Whether this node believes it's the primary
    pub believes_primary: bool,
    /// Primary term if believes_primary is true
    pub primary_term: u64,
    /// Current primary info (if any)
    pub primary_info: Option<PrimaryInfo>,
}

impl ClusterState {
    /// Check if this node has a valid primary claim
    ///
    /// TLA+ HasValidPrimaryClaim:
    /// - believesPrimary is true
    /// - Node is Active
    /// - Can reach majority (checked externally)
    pub fn has_valid_primary_claim(&self, can_reach_majority: bool) -> bool {
        self.believes_primary && self.state == NodeState::Active && can_reach_majority
    }

    /// Check if this node can become primary
    ///
    /// TLA+ CanBecomePrimary (safe version):
    /// - Node is Active
    /// - Can reach majority of ALL nodes in cluster
    /// - No valid primary exists
    pub fn can_become_primary(
        &self,
        cluster_size: usize,
        reachable_active: usize,
        any_valid_primary: bool,
    ) -> bool {
        self.state == NodeState::Active && 2 * reachable_active > cluster_size && !any_valid_primary
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_state_valid_transitions() {
        // Left -> Joining
        let mut state = NodeState::Left;
        assert!(state.can_transition_to(NodeState::Joining));
        state.transition_to(NodeState::Joining);
        assert_eq!(state, NodeState::Joining);

        // Joining -> Active
        assert!(state.can_transition_to(NodeState::Active));
        state.transition_to(NodeState::Active);
        assert_eq!(state, NodeState::Active);

        // Active -> Leaving
        assert!(state.can_transition_to(NodeState::Leaving));
        state.transition_to(NodeState::Leaving);
        assert_eq!(state, NodeState::Leaving);

        // Leaving -> Left
        assert!(state.can_transition_to(NodeState::Left));
        state.transition_to(NodeState::Left);
        assert_eq!(state, NodeState::Left);
    }

    #[test]
    fn test_node_state_failure_transitions() {
        // Active -> Failed
        let mut state = NodeState::Active;
        assert!(state.can_transition_to(NodeState::Failed));
        state.transition_to(NodeState::Failed);
        assert_eq!(state, NodeState::Failed);

        // Failed -> Left
        assert!(state.can_transition_to(NodeState::Left));
        state.transition_to(NodeState::Left);
        assert_eq!(state, NodeState::Left);
    }

    #[test]
    fn test_node_state_first_node_join() {
        // First node: Left -> Active directly
        let mut state = NodeState::Left;
        assert!(state.can_transition_to(NodeState::Active));
        state.transition_to(NodeState::Active);
        assert_eq!(state, NodeState::Active);
    }

    #[test]
    fn test_node_state_invalid_transitions() {
        let state = NodeState::Left;

        // Left cannot go to Leaving
        assert!(!state.can_transition_to(NodeState::Leaving));

        // Left cannot go to Failed
        assert!(!state.can_transition_to(NodeState::Failed));
    }

    #[test]
    #[should_panic(expected = "invalid state transition")]
    fn test_node_state_transition_panics_on_invalid() {
        let mut state = NodeState::Left;
        state.transition_to(NodeState::Leaving); // Should panic
    }

    #[test]
    fn test_primary_info() {
        let node_id = NodeId::new("node-1").unwrap();
        let primary = PrimaryInfo::new(node_id.clone(), 1, 1000);

        assert_eq!(primary.term, 1);
        assert!(primary.is_same_node(&node_id));

        let other_node = NodeId::new("node-2").unwrap();
        assert!(!primary.is_same_node(&other_node));
    }

    #[test]
    fn test_primary_term_comparison() {
        let node1 = NodeId::new("node-1").unwrap();
        let node2 = NodeId::new("node-2").unwrap();

        let primary1 = PrimaryInfo::new(node1, 1, 1000);
        let primary2 = PrimaryInfo::new(node2, 2, 2000);

        assert!(!primary1.has_higher_term_than(&primary2));
        assert!(primary2.has_higher_term_than(&primary1));
    }

    #[test]
    fn test_membership_view() {
        let mut nodes = HashSet::new();
        nodes.insert(NodeId::new("node-1").unwrap());
        nodes.insert(NodeId::new("node-2").unwrap());
        nodes.insert(NodeId::new("node-3").unwrap());

        let view = MembershipView::new(nodes, 1, 1000);

        assert_eq!(view.size(), 3);
        assert_eq!(view.quorum_size(), 2); // 3/2 + 1 = 2
        assert!(view.is_quorum(2));
        assert!(view.is_quorum(3));
        assert!(!view.is_quorum(1));

        assert!(view.contains(&NodeId::new("node-1").unwrap()));
        assert!(!view.contains(&NodeId::new("node-4").unwrap()));
    }

    #[test]
    fn test_membership_view_add_remove() {
        let mut nodes = HashSet::new();
        nodes.insert(NodeId::new("node-1").unwrap());

        let view = MembershipView::new(nodes, 1, 1000);

        // Add node
        let view2 = view.with_node_added(NodeId::new("node-2").unwrap(), 2000);
        assert_eq!(view2.size(), 2);
        assert_eq!(view2.view_number, 2);

        // Remove node
        let view3 = view2.with_node_removed(&NodeId::new("node-1").unwrap(), 3000);
        assert_eq!(view3.size(), 1);
        assert_eq!(view3.view_number, 3);
        assert!(!view3.contains(&NodeId::new("node-1").unwrap()));
        assert!(view3.contains(&NodeId::new("node-2").unwrap()));
    }

    #[test]
    fn test_membership_view_merge() {
        let mut nodes1 = HashSet::new();
        nodes1.insert(NodeId::new("node-1").unwrap());
        nodes1.insert(NodeId::new("node-2").unwrap());
        let view1 = MembershipView::new(nodes1, 5, 1000);

        let mut nodes2 = HashSet::new();
        nodes2.insert(NodeId::new("node-2").unwrap());
        nodes2.insert(NodeId::new("node-3").unwrap());
        let view2 = MembershipView::new(nodes2, 3, 2000);

        let merged = view1.merge(&view2, 3000);

        assert_eq!(merged.size(), 3); // Union
        assert_eq!(merged.view_number, 6); // max(5,3) + 1
        assert!(merged.contains(&NodeId::new("node-1").unwrap()));
        assert!(merged.contains(&NodeId::new("node-2").unwrap()));
        assert!(merged.contains(&NodeId::new("node-3").unwrap()));
    }

    #[test]
    fn test_cluster_state_can_become_primary() {
        let node_id = NodeId::new("node-1").unwrap();
        let state = ClusterState {
            node_id,
            state: NodeState::Active,
            view: MembershipView::empty(),
            believes_primary: false,
            primary_term: 0,
            primary_info: None,
        };

        // Can become primary: Active, majority reachable, no valid primary
        assert!(state.can_become_primary(5, 3, false)); // 3 > 5/2

        // Cannot become primary: no majority
        assert!(!state.can_become_primary(5, 2, false)); // 2 <= 5/2

        // Cannot become primary: valid primary exists
        assert!(!state.can_become_primary(5, 3, true));

        // Cannot become primary: not Active
        let joining_state = ClusterState {
            node_id: NodeId::new("node-2").unwrap(),
            state: NodeState::Joining,
            view: MembershipView::empty(),
            believes_primary: false,
            primary_term: 0,
            primary_info: None,
        };
        assert!(!joining_state.can_become_primary(5, 3, false));
    }
}
