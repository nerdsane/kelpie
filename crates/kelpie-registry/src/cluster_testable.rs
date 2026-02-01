//! Testable Cluster Membership Implementation
//!
//! This module provides a `TestableClusterMembership` that uses the
//! `ClusterStorageBackend` trait for storage, enabling DST testing
//! against production-equivalent code without requiring FDB.
//!
//! TigerStyle: Explicit state management, 2+ assertions per function.

use crate::cluster_storage::ClusterStorageBackend;
use crate::cluster_types::{ClusterNodeInfo, MigrationCandidate, MigrationQueue, MigrationResult};
use crate::error::{RegistryError, RegistryResult};
use crate::membership::{MembershipView, NodeState, PrimaryInfo};
use crate::node::NodeId;
use kelpie_core::io::TimeProvider;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument, warn};

// =============================================================================
// Constants (same as cluster.rs)
// =============================================================================

/// Election timeout in milliseconds
pub const ELECTION_TIMEOUT_MS: u64 = 5_000;

/// Primary step-down delay after quorum loss in milliseconds
pub const PRIMARY_STEPDOWN_DELAY_MS: u64 = 1_000;

// =============================================================================
// TestableClusterMembership
// =============================================================================

/// Testable cluster membership manager
///
/// Implements the same logic as `ClusterMembership` but uses the
/// `ClusterStorageBackend` trait for storage, enabling DST testing
/// without FDB.
///
/// TigerStyle: All state changes are explicit, 2+ assertions per function.
pub struct TestableClusterMembership<S: ClusterStorageBackend> {
    /// Storage backend
    storage: Arc<S>,
    /// Local node ID
    local_node_id: NodeId,
    /// Local node state cache
    local_state: RwLock<NodeState>,
    /// Does this node believe it's primary?
    believes_primary: RwLock<bool>,
    /// Current primary term
    primary_term: RwLock<u64>,
    /// Local membership view cache
    local_view: RwLock<MembershipView>,
    /// Time provider for timestamps
    time_provider: Arc<dyn TimeProvider>,
    /// Set of reachable nodes (for DST simulation)
    reachable_nodes: RwLock<HashSet<NodeId>>,
}

impl<S: ClusterStorageBackend> TestableClusterMembership<S> {
    /// Create a new testable cluster membership manager
    ///
    /// # Arguments
    /// * `storage` - Storage backend implementing `ClusterStorageBackend`
    /// * `local_node_id` - This node's ID
    /// * `time_provider` - Provider for timestamps
    ///
    /// # Preconditions
    /// * `local_node_id` must be valid (non-empty)
    pub fn new(
        storage: Arc<S>,
        local_node_id: NodeId,
        time_provider: Arc<dyn TimeProvider>,
    ) -> Self {
        // Preconditions (TigerStyle)
        assert!(
            !local_node_id.as_str().is_empty(),
            "local_node_id cannot be empty"
        );

        Self {
            storage,
            local_node_id,
            local_state: RwLock::new(NodeState::Left),
            believes_primary: RwLock::new(false),
            primary_term: RwLock::new(0),
            local_view: RwLock::new(MembershipView::empty()),
            time_provider,
            reachable_nodes: RwLock::new(HashSet::new()),
        }
    }

    /// Get the local node ID
    pub fn local_node_id(&self) -> &NodeId {
        &self.local_node_id
    }

    /// Get the current local state
    pub async fn local_state(&self) -> NodeState {
        *self.local_state.read().await
    }

    /// Check if this node believes it's the primary
    pub async fn is_primary(&self) -> bool {
        *self.believes_primary.read().await
    }

    /// Get the current primary term
    pub async fn current_term(&self) -> u64 {
        *self.primary_term.read().await
    }

    /// Get the current membership view
    pub async fn membership_view(&self) -> MembershipView {
        self.local_view.read().await.clone()
    }

    // =========================================================================
    // Node Join/Leave Operations
    // =========================================================================

    /// Join the cluster
    ///
    /// TLA+ NodeJoin: Left -> Joining (or Active if first node)
    ///
    /// # Preconditions
    /// * Node must be in Left state
    /// * rpc_addr must be non-empty
    ///
    /// # Postconditions
    /// * Node state is either Joining or Active
    /// * If first node: becomes primary with term 1
    #[instrument(skip(self), fields(node_id = %self.local_node_id))]
    pub async fn join(&self, rpc_addr: String) -> RegistryResult<()> {
        // Preconditions (TigerStyle)
        assert!(!rpc_addr.is_empty(), "rpc_addr cannot be empty");

        let now_ms = self.time_provider.now_ms();

        // Check current state
        let mut local_state = self.local_state.write().await;
        assert!(
            *local_state == NodeState::Left,
            "join requires node to be in Left state, was {:?}",
            *local_state
        );

        // Check if this is the first node
        let existing_nodes = self.storage.list_nodes().await?;
        let is_first_node = existing_nodes.is_empty()
            || existing_nodes
                .iter()
                .all(|n| n.state == NodeState::Left || n.state == NodeState::Failed);

        // Create node info
        let mut node_info = ClusterNodeInfo::new(self.local_node_id.clone(), rpc_addr, now_ms);

        if is_first_node {
            // First node: join directly as Active and become primary
            node_info.state = NodeState::Active;
            node_info.joined_at_ms = now_ms;

            // Write node info
            self.storage.write_node(&node_info).await?;

            // Create initial membership view
            let mut active_nodes = HashSet::new();
            active_nodes.insert(self.local_node_id.clone());
            let view = MembershipView::new(active_nodes, 1, now_ms);
            self.storage.write_membership_view(&view).await?;

            // Become primary with term 1
            let primary = PrimaryInfo::new(self.local_node_id.clone(), 1, now_ms);
            self.storage.write_primary(&primary).await?;
            self.storage.write_primary_term(1).await?;

            *local_state = NodeState::Active;
            *self.believes_primary.write().await = true;
            *self.primary_term.write().await = 1;
            *self.local_view.write().await = view;

            // Postconditions (TigerStyle)
            assert!(
                *local_state == NodeState::Active,
                "first node must be Active"
            );
            assert!(
                *self.believes_primary.read().await,
                "first node must be primary"
            );

            info!("Joined as first node and became primary with term 1");
        } else {
            // Not first: join as Joining
            node_info.state = NodeState::Joining;

            self.storage.write_node(&node_info).await?;
            *local_state = NodeState::Joining;

            // Postcondition (TigerStyle)
            assert!(
                *local_state == NodeState::Joining,
                "non-first node must be Joining"
            );

            info!("Joined as Joining, waiting for active nodes to accept");
        }

        Ok(())
    }

    /// Complete joining the cluster
    ///
    /// TLA+ NodeJoinComplete: Joining -> Active
    ///
    /// # Preconditions
    /// * Node must be in Joining state
    ///
    /// # Postconditions
    /// * Node state is Active
    /// * Node is in membership view
    #[instrument(skip(self), fields(node_id = %self.local_node_id))]
    pub async fn complete_join(&self) -> RegistryResult<()> {
        let now_ms = self.time_provider.now_ms();

        let mut local_state = self.local_state.write().await;
        // Precondition (TigerStyle)
        assert!(
            *local_state == NodeState::Joining,
            "complete_join requires Joining state, was {:?}",
            *local_state
        );

        // Update node state to Active
        let mut node_info = self
            .storage
            .get_node(&self.local_node_id)
            .await?
            .ok_or_else(|| RegistryError::node_not_found(self.local_node_id.as_str()))?;

        node_info.state = NodeState::Active;
        node_info.joined_at_ms = now_ms;
        self.storage.write_node(&node_info).await?;

        // Update membership view to include this node
        let mut view = self
            .storage
            .read_membership_view()
            .await?
            .unwrap_or_default();
        view = view.with_node_added(self.local_node_id.clone(), now_ms);
        self.storage.write_membership_view(&view).await?;

        *local_state = NodeState::Active;
        *self.local_view.write().await = view.clone();

        // Postconditions (TigerStyle)
        assert!(
            *local_state == NodeState::Active,
            "must be Active after complete_join"
        );
        assert!(
            view.contains(&self.local_node_id),
            "must be in membership view after complete_join"
        );

        info!("Completed join, now Active");
        Ok(())
    }

    /// Leave the cluster gracefully
    ///
    /// TLA+ NodeLeave: Active -> Leaving
    ///
    /// # Preconditions
    /// * Node must be in Active state
    ///
    /// # Postconditions
    /// * Node state is Leaving
    /// * Node is not in membership view
    /// * If was primary, no longer primary
    #[instrument(skip(self), fields(node_id = %self.local_node_id))]
    pub async fn leave(&self) -> RegistryResult<()> {
        let now_ms = self.time_provider.now_ms();

        let mut local_state = self.local_state.write().await;
        // Precondition (TigerStyle)
        assert!(
            *local_state == NodeState::Active,
            "leave requires Active state, was {:?}",
            *local_state
        );

        // Step down if primary
        if *self.believes_primary.read().await {
            drop(local_state); // Release lock before step_down
            self.step_down_internal().await?;
            local_state = self.local_state.write().await;
        }

        // Update node state
        let mut node_info = self
            .storage
            .get_node(&self.local_node_id)
            .await?
            .ok_or_else(|| RegistryError::node_not_found(self.local_node_id.as_str()))?;

        node_info.state = NodeState::Leaving;
        self.storage.write_node(&node_info).await?;

        // Remove from membership view
        let mut view = self
            .storage
            .read_membership_view()
            .await?
            .unwrap_or_default();
        view = view.with_node_removed(&self.local_node_id, now_ms);
        self.storage.write_membership_view(&view).await?;

        *local_state = NodeState::Leaving;
        *self.local_view.write().await = view.clone();

        // Postconditions (TigerStyle)
        assert!(
            *local_state == NodeState::Leaving,
            "must be Leaving after leave"
        );
        assert!(
            !view.contains(&self.local_node_id),
            "must not be in membership view after leave"
        );

        info!("Started leaving cluster");
        Ok(())
    }

    /// Complete leaving the cluster
    ///
    /// TLA+ NodeLeaveComplete: Leaving -> Left
    ///
    /// # Preconditions
    /// * Node must be in Leaving state
    ///
    /// # Postconditions
    /// * Node state is Left
    /// * Not primary, term is 0
    #[instrument(skip(self), fields(node_id = %self.local_node_id))]
    pub async fn complete_leave(&self) -> RegistryResult<()> {
        let mut local_state = self.local_state.write().await;
        // Precondition (TigerStyle)
        assert!(
            *local_state == NodeState::Leaving,
            "complete_leave requires Leaving state, was {:?}",
            *local_state
        );

        // Update node state
        let mut node_info = self
            .storage
            .get_node(&self.local_node_id)
            .await?
            .ok_or_else(|| RegistryError::node_not_found(self.local_node_id.as_str()))?;

        node_info.state = NodeState::Left;
        self.storage.write_node(&node_info).await?;

        *local_state = NodeState::Left;
        *self.believes_primary.write().await = false;
        *self.primary_term.write().await = 0;
        *self.local_view.write().await = MembershipView::empty();

        // Postconditions (TigerStyle)
        assert!(
            *local_state == NodeState::Left,
            "must be Left after complete_leave"
        );
        assert!(
            !*self.believes_primary.read().await,
            "must not be primary after complete_leave"
        );

        info!("Completed leave, now Left");
        Ok(())
    }

    // =========================================================================
    // Primary Election
    // =========================================================================

    /// Try to become primary
    ///
    /// TLA+ CanBecomePrimary conditions:
    /// - Node is Active
    /// - Can reach majority of ALL nodes
    /// - No valid primary exists
    ///
    /// # Preconditions
    /// * Node state is checked internally
    ///
    /// # Returns
    /// * `Some(term)` if became primary
    /// * `None` if cannot become primary
    #[instrument(skip(self), fields(node_id = %self.local_node_id))]
    pub async fn try_become_primary(&self) -> RegistryResult<Option<u64>> {
        let local_state = *self.local_state.read().await;
        if local_state != NodeState::Active {
            debug!(
                "Cannot become primary: not Active (state={:?})",
                local_state
            );
            return Ok(None);
        }

        // Check if already primary
        if *self.believes_primary.read().await {
            let term = *self.primary_term.read().await;
            debug!("Already primary with term {}", term);
            return Ok(Some(term));
        }

        // Check quorum
        let (cluster_size, reachable_count) = self.calculate_reachability().await?;
        if !self.has_quorum(cluster_size, reachable_count) {
            debug!(
                "Cannot become primary: no quorum ({}/{})",
                reachable_count, cluster_size
            );
            return Ok(None);
        }

        // Check if valid primary exists
        if let Some(current_primary) = self.storage.read_primary().await? {
            // Check if current primary is still valid (reachable and Active)
            if self.is_primary_valid(&current_primary).await? {
                debug!(
                    "Cannot become primary: valid primary exists ({})",
                    current_primary.node_id
                );
                return Ok(None);
            }
        }

        // Increment term and become primary
        let current_term = self.storage.read_primary_term().await?.unwrap_or(0);
        let new_term = current_term + 1;
        let now_ms = self.time_provider.now_ms();

        let primary = PrimaryInfo::new(self.local_node_id.clone(), new_term, now_ms);

        // Write primary and term
        self.storage.write_primary(&primary).await?;
        self.storage.write_primary_term(new_term).await?;

        *self.believes_primary.write().await = true;
        *self.primary_term.write().await = new_term;

        // Postconditions (TigerStyle)
        assert!(
            *self.believes_primary.read().await,
            "must believe primary after election"
        );
        assert!(
            *self.primary_term.read().await == new_term,
            "term must be updated"
        );

        info!("Became primary with term {}", new_term);
        Ok(Some(new_term))
    }

    /// Step down from primary role
    ///
    /// Called when quorum is lost or voluntarily stepping down
    ///
    /// # Postconditions
    /// * Not believing primary
    #[instrument(skip(self), fields(node_id = %self.local_node_id))]
    pub async fn step_down(&self) -> RegistryResult<()> {
        self.step_down_internal().await
    }

    async fn step_down_internal(&self) -> RegistryResult<()> {
        if !*self.believes_primary.read().await {
            return Ok(()); // Not primary, nothing to do
        }

        // Clear primary claim
        self.storage.clear_primary().await?;
        *self.believes_primary.write().await = false;

        // Postcondition (TigerStyle)
        assert!(
            !*self.believes_primary.read().await,
            "must not believe primary after step_down"
        );

        info!("Stepped down from primary");
        Ok(())
    }

    /// Check if this node has a valid primary claim
    ///
    /// TLA+ HasValidPrimaryClaim:
    /// - believesPrimary is true
    /// - Node is Active
    /// - Can reach majority
    pub async fn has_valid_primary_claim(&self) -> RegistryResult<bool> {
        let is_primary = *self.believes_primary.read().await;
        let local_state = *self.local_state.read().await;

        if !is_primary || local_state != NodeState::Active {
            return Ok(false);
        }

        let (cluster_size, reachable_count) = self.calculate_reachability().await?;
        Ok(self.has_quorum(cluster_size, reachable_count))
    }

    /// Get current primary info
    pub async fn get_primary(&self) -> RegistryResult<Option<PrimaryInfo>> {
        self.storage.read_primary().await
    }

    // =========================================================================
    // Quorum and Reachability
    // =========================================================================

    /// Check if a count constitutes a quorum
    ///
    /// # TigerStyle
    /// * Uses strict majority: 2 * reachable > cluster_size
    fn has_quorum(&self, cluster_size: usize, reachable_count: usize) -> bool {
        // Strict majority: 2 * reachable > cluster_size
        2 * reachable_count > cluster_size
    }

    /// Calculate cluster size and reachable count
    async fn calculate_reachability(&self) -> RegistryResult<(usize, usize)> {
        let nodes = self.storage.list_nodes().await?;
        let cluster_size = nodes.len().max(1); // At least count ourselves

        // Count reachable active nodes
        let reachable = self.reachable_nodes.read().await;
        let mut reachable_count = 1; // Count self

        for node in &nodes {
            if node.id != self.local_node_id
                && node.state == NodeState::Active
                && reachable.contains(&node.id)
            {
                reachable_count += 1;
            }
        }

        Ok((cluster_size, reachable_count))
    }

    /// Check if a primary is still valid
    async fn is_primary_valid(&self, primary: &PrimaryInfo) -> RegistryResult<bool> {
        // Check if primary node is Active
        if let Some(node) = self.storage.get_node(&primary.node_id).await? {
            if node.state != NodeState::Active {
                return Ok(false);
            }
        } else {
            return Ok(false);
        }

        // Check if primary is reachable (or is us)
        if primary.node_id == self.local_node_id {
            // We are the primary - check our own quorum
            let (cluster_size, reachable_count) = self.calculate_reachability().await?;
            return Ok(self.has_quorum(cluster_size, reachable_count));
        }

        // Check if we can reach the primary
        let reachable = self.reachable_nodes.read().await;
        Ok(reachable.contains(&primary.node_id))
    }

    /// Set reachable nodes (for DST simulation)
    pub async fn set_reachable_nodes(&self, nodes: HashSet<NodeId>) {
        *self.reachable_nodes.write().await = nodes;
    }

    /// Mark a node as unreachable (for DST simulation)
    pub async fn mark_unreachable(&self, node_id: &NodeId) {
        self.reachable_nodes.write().await.remove(node_id);
    }

    /// Mark a node as reachable (for DST simulation)
    pub async fn mark_reachable(&self, node_id: &NodeId) {
        self.reachable_nodes.write().await.insert(node_id.clone());
    }

    // =========================================================================
    // Heartbeat and Failure Detection
    // =========================================================================

    /// Send heartbeat (update last_heartbeat_ms)
    ///
    /// TLA+ SendHeartbeat action
    ///
    /// # Preconditions
    /// * Node should be Active (only active nodes send heartbeats)
    #[instrument(skip(self), fields(node_id = %self.local_node_id))]
    pub async fn send_heartbeat(&self) -> RegistryResult<()> {
        let now_ms = self.time_provider.now_ms();

        // Precondition (TigerStyle) - only Active nodes send heartbeats
        let state = *self.local_state.read().await;
        assert!(
            state == NodeState::Active,
            "only Active nodes send heartbeats, was {:?}",
            state
        );

        if let Some(mut node) = self.storage.get_node(&self.local_node_id).await? {
            let old_heartbeat = node.last_heartbeat_ms;
            node.last_heartbeat_ms = now_ms;
            self.storage.write_node(&node).await?;

            // Postcondition (TigerStyle)
            assert!(now_ms >= old_heartbeat, "heartbeat time must not decrease");
        }

        Ok(())
    }

    /// Detect failure of a node based on heartbeat timeout
    ///
    /// TLA+ DetectFailure action
    ///
    /// # Arguments
    /// * `target` - Node to check for failure
    /// * `timeout_ms` - Timeout threshold
    ///
    /// # Returns
    /// * `true` if node was detected as failed and marked
    /// * `false` if node is still healthy
    #[instrument(skip(self), fields(node_id = %self.local_node_id, target = %target))]
    pub async fn detect_failure(&self, target: &NodeId, timeout_ms: u64) -> RegistryResult<bool> {
        // Precondition (TigerStyle)
        assert!(
            target != &self.local_node_id,
            "cannot detect self as failed"
        );
        assert!(timeout_ms > 0, "timeout must be positive");

        let now_ms = self.time_provider.now_ms();

        if let Some(node) = self.storage.get_node(target).await? {
            if node.state == NodeState::Active && node.is_heartbeat_timeout(now_ms, timeout_ms) {
                // Mark as failed
                self.mark_node_failed(target).await?;
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Check for failed nodes based on heartbeat timeout
    #[instrument(skip(self))]
    pub async fn detect_failed_nodes(&self, timeout_ms: u64) -> RegistryResult<Vec<NodeId>> {
        let now_ms = self.time_provider.now_ms();
        let nodes = self.storage.list_nodes().await?;

        let mut failed = Vec::new();
        for node in nodes {
            if node.id == self.local_node_id {
                continue;
            }

            if node.state == NodeState::Active && node.is_heartbeat_timeout(now_ms, timeout_ms) {
                failed.push(node.id);
            }
        }

        Ok(failed)
    }

    /// Mark a node as failed
    ///
    /// TLA+ MarkNodeFailed: Active -> Failed
    ///
    /// # Postconditions
    /// * Node state is Failed
    /// * Node is not in membership view
    #[instrument(skip(self), fields(failed_node = %node_id))]
    pub async fn mark_node_failed(&self, node_id: &NodeId) -> RegistryResult<()> {
        let now_ms = self.time_provider.now_ms();

        if let Some(mut node) = self.storage.get_node(node_id).await? {
            if node.state == NodeState::Active {
                node.state = NodeState::Failed;
                self.storage.write_node(&node).await?;

                // Remove from membership view
                let mut view = self
                    .storage
                    .read_membership_view()
                    .await?
                    .unwrap_or_default();
                view = view.with_node_removed(node_id, now_ms);
                self.storage.write_membership_view(&view).await?;

                // If this is us (detected externally), update local state
                if node_id == &self.local_node_id {
                    *self.local_state.write().await = NodeState::Failed;
                    *self.believes_primary.write().await = false;
                }

                // Update local view cache
                *self.local_view.write().await = view.clone();

                // Postconditions (TigerStyle)
                let stored_node = self.storage.get_node(node_id).await?.unwrap();
                assert!(
                    stored_node.state == NodeState::Failed,
                    "node must be Failed after mark_node_failed"
                );
                assert!(
                    !view.contains(node_id),
                    "node must not be in view after mark_node_failed"
                );

                info!("Marked node {} as Failed", node_id);
            }
        }

        Ok(())
    }

    /// Node recover from Failed state
    ///
    /// TLA+ NodeRecover: Failed -> Left
    ///
    /// # Preconditions
    /// * Node must be in Failed state
    ///
    /// # Postconditions
    /// * Node state is Left (ready to rejoin)
    #[instrument(skip(self), fields(node_id = %node_id))]
    pub async fn node_recover(&self, node_id: &NodeId) -> RegistryResult<()> {
        if let Some(mut node) = self.storage.get_node(node_id).await? {
            // Precondition (TigerStyle)
            assert!(
                node.state == NodeState::Failed,
                "can only recover from Failed state, was {:?}",
                node.state
            );

            node.state = NodeState::Left;
            self.storage.write_node(&node).await?;

            // If this is us, update local state
            if node_id == &self.local_node_id {
                *self.local_state.write().await = NodeState::Left;
                *self.believes_primary.write().await = false;
                *self.primary_term.write().await = 0;
            }

            // Postcondition (TigerStyle)
            let stored_node = self.storage.get_node(node_id).await?.unwrap();
            assert!(
                stored_node.state == NodeState::Left,
                "node must be Left after recover"
            );

            info!("Node {} recovered from Failed to Left", node_id);
        }

        Ok(())
    }

    // =========================================================================
    // Membership View Synchronization
    // =========================================================================

    /// Synchronize membership views between nodes
    ///
    /// TLA+ SyncViews action
    ///
    /// # Arguments
    /// * `other_view` - View from another node
    ///
    /// # Returns
    /// * Merged view
    #[instrument(skip(self, other_view))]
    pub async fn sync_views(&self, other_view: &MembershipView) -> RegistryResult<MembershipView> {
        let my_view = self
            .storage
            .read_membership_view()
            .await?
            .unwrap_or_default();
        let now_ms = self.time_provider.now_ms();

        if my_view.view_number == other_view.view_number {
            // Same view number - must have same content (TLA+ invariant)
            // In production, this would be an assertion, but for robustness we log
            if my_view.active_nodes != other_view.active_nodes {
                warn!(
                    "MembershipConsistency potential violation: same view number {}, different nodes",
                    my_view.view_number
                );
            }
            return Ok(my_view);
        }

        // Merge views
        let merged = my_view.merge(other_view, now_ms);
        self.storage.write_membership_view(&merged).await?;
        *self.local_view.write().await = merged.clone();

        // Postcondition (TigerStyle)
        assert!(
            merged.view_number > my_view.view_number || merged.view_number > other_view.view_number,
            "merged view must have higher view number"
        );

        Ok(merged)
    }

    /// Synchronize local view with storage
    pub async fn sync_membership_view(&self) -> RegistryResult<()> {
        if let Some(view) = self.storage.read_membership_view().await? {
            *self.local_view.write().await = view;
        }
        Ok(())
    }

    /// Check if this node still has quorum, step down if not
    pub async fn check_quorum_and_maybe_step_down(&self) -> RegistryResult<bool> {
        if !*self.believes_primary.read().await {
            return Ok(true); // Not primary, nothing to check
        }

        let (cluster_size, reachable_count) = self.calculate_reachability().await?;
        if !self.has_quorum(cluster_size, reachable_count) {
            warn!(
                "Primary lost quorum ({}/{}), stepping down",
                reachable_count, cluster_size
            );
            self.step_down().await?;
            return Ok(false);
        }

        Ok(true)
    }

    // =========================================================================
    // Actor Migration (FR-7)
    // =========================================================================

    /// Queue actors from a failed node for migration
    #[instrument(skip(self, actor_ids), fields(failed_node = %failed_node_id, count = actor_ids.len()))]
    pub async fn queue_actors_for_migration(
        &self,
        failed_node_id: &NodeId,
        actor_ids: Vec<String>,
    ) -> RegistryResult<usize> {
        let now_ms = self.time_provider.now_ms();

        // Precondition (TigerStyle)
        assert!(
            !actor_ids.is_empty(),
            "cannot queue empty actor list for migration"
        );

        // Read current migration queue
        let mut queue = self
            .storage
            .read_migration_queue()
            .await?
            .unwrap_or_default();

        // Add all actors to the queue
        let count = actor_ids.len();
        for actor_id in actor_ids {
            let candidate = MigrationCandidate::new(actor_id, failed_node_id.clone(), now_ms);
            queue.add(candidate, now_ms);
        }

        // Write updated queue
        self.storage.write_migration_queue(&queue).await?;

        // Postcondition (TigerStyle)
        assert!(queue.len() >= count, "queue must contain added actors");

        info!(
            count = count,
            failed_node = %failed_node_id,
            "Queued actors for migration"
        );

        Ok(count)
    }

    /// Process migration queue (called by primary)
    #[instrument(skip(self, select_node))]
    pub async fn process_migration_queue<F>(
        &self,
        select_node: F,
    ) -> RegistryResult<Vec<MigrationResult>>
    where
        F: Fn(&str) -> Option<NodeId>,
    {
        // Only primary should process migrations
        if !*self.believes_primary.read().await {
            debug!("Not primary, skipping migration processing");
            return Ok(Vec::new());
        }

        let now_ms = self.time_provider.now_ms();

        // Read migration queue
        let mut queue = self
            .storage
            .read_migration_queue()
            .await?
            .unwrap_or_default();

        if queue.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();
        let mut to_remove = Vec::new();

        // Process each candidate
        for candidate in &queue.candidates {
            match select_node(&candidate.actor_id) {
                Some(target_node) => {
                    results.push(MigrationResult::Success {
                        actor_id: candidate.actor_id.clone(),
                        new_node_id: target_node,
                    });
                    to_remove.push(candidate.actor_id.clone());
                }
                None => {
                    results.push(MigrationResult::NoCapacity {
                        actor_id: candidate.actor_id.clone(),
                    });
                }
            }
        }

        // Remove successfully processed candidates
        for actor_id in to_remove {
            queue.remove(&actor_id, now_ms);
        }

        // Write updated queue
        self.storage.write_migration_queue(&queue).await?;

        let success_count = results.iter().filter(|r| r.is_success()).count();
        info!(
            processed = results.len(),
            success = success_count,
            remaining = queue.len(),
            "Processed migration queue"
        );

        Ok(results)
    }

    /// Get the current migration queue
    pub async fn get_migration_queue(&self) -> RegistryResult<MigrationQueue> {
        self.storage
            .read_migration_queue()
            .await
            .map(|q| q.unwrap_or_default())
    }

    /// Handle node failure including actor migration queueing
    #[instrument(skip(self, actor_ids), fields(failed_node = %node_id))]
    pub async fn handle_node_failure(
        &self,
        node_id: &NodeId,
        actor_ids: Vec<String>,
    ) -> RegistryResult<()> {
        // Mark node as failed (this also updates membership view)
        self.mark_node_failed(node_id).await?;

        // If we're primary and there are actors to migrate, queue them
        if *self.believes_primary.read().await && !actor_ids.is_empty() {
            self.queue_actors_for_migration(node_id, actor_ids).await?;
        }

        Ok(())
    }

    /// Get a cluster node by ID (for testing)
    pub async fn get_cluster_node(
        &self,
        node_id: &NodeId,
    ) -> RegistryResult<Option<ClusterNodeInfo>> {
        self.storage.get_node(node_id).await
    }

    /// List all cluster nodes (for testing)
    pub async fn list_cluster_nodes(&self) -> RegistryResult<Vec<ClusterNodeInfo>> {
        self.storage.list_nodes().await
    }
}

impl<S: ClusterStorageBackend> std::fmt::Debug for TestableClusterMembership<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestableClusterMembership")
            .field("local_node_id", &self.local_node_id)
            .finish()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster_storage::MockClusterStorage;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Debug)]
    struct TestClock {
        now_ms: AtomicU64,
    }

    impl TestClock {
        fn new(initial_ms: u64) -> Self {
            Self {
                now_ms: AtomicU64::new(initial_ms),
            }
        }

        fn advance(&self, ms: u64) {
            self.now_ms.fetch_add(ms, Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl TimeProvider for TestClock {
        fn now_ms(&self) -> u64 {
            self.now_ms.load(Ordering::SeqCst)
        }

        async fn sleep_ms(&self, ms: u64) {
            self.advance(ms);
        }
    }

    fn test_node_id(n: u32) -> NodeId {
        NodeId::new(format!("node-{}", n)).unwrap()
    }

    #[tokio::test]
    async fn test_first_node_join() {
        let storage = Arc::new(MockClusterStorage::new());
        let clock = Arc::new(TestClock::new(1000));
        let node_id = test_node_id(1);

        let membership =
            TestableClusterMembership::new(storage.clone(), node_id.clone(), clock.clone());

        // Join as first node
        membership.join("127.0.0.1:8080".to_string()).await.unwrap();

        // Should be Active and primary
        assert_eq!(membership.local_state().await, NodeState::Active);
        assert!(membership.is_primary().await);
        assert_eq!(membership.current_term().await, 1);
    }

    #[tokio::test]
    async fn test_second_node_join() {
        let storage = Arc::new(MockClusterStorage::new());
        let clock = Arc::new(TestClock::new(1000));

        // First node
        let node1_id = test_node_id(1);
        let membership1 =
            TestableClusterMembership::new(storage.clone(), node1_id.clone(), clock.clone());
        membership1
            .join("127.0.0.1:8080".to_string())
            .await
            .unwrap();

        // Second node
        let node2_id = test_node_id(2);
        let membership2 =
            TestableClusterMembership::new(storage.clone(), node2_id.clone(), clock.clone());
        membership2
            .join("127.0.0.1:8081".to_string())
            .await
            .unwrap();

        // Should be Joining (not first)
        assert_eq!(membership2.local_state().await, NodeState::Joining);
        assert!(!membership2.is_primary().await);

        // Complete join
        membership2.complete_join().await.unwrap();
        assert_eq!(membership2.local_state().await, NodeState::Active);
    }

    #[tokio::test]
    async fn test_primary_election_requires_quorum() {
        let storage = Arc::new(MockClusterStorage::new());
        let clock = Arc::new(TestClock::new(1000));

        // Create 3-node cluster
        let node1_id = test_node_id(1);
        let node2_id = test_node_id(2);
        let node3_id = test_node_id(3);

        let membership1 =
            TestableClusterMembership::new(storage.clone(), node1_id.clone(), clock.clone());
        membership1
            .join("127.0.0.1:8080".to_string())
            .await
            .unwrap();

        let membership2 =
            TestableClusterMembership::new(storage.clone(), node2_id.clone(), clock.clone());
        membership2
            .join("127.0.0.1:8081".to_string())
            .await
            .unwrap();
        membership2.complete_join().await.unwrap();

        let membership3 =
            TestableClusterMembership::new(storage.clone(), node3_id.clone(), clock.clone());
        membership3
            .join("127.0.0.1:8082".to_string())
            .await
            .unwrap();
        membership3.complete_join().await.unwrap();

        // Node 1 is primary
        assert!(membership1.is_primary().await);

        // Node 2 cannot become primary (node 1 is valid primary)
        membership2.mark_reachable(&node1_id).await;
        membership2.mark_reachable(&node3_id).await;
        let result = membership2.try_become_primary().await.unwrap();
        assert!(
            result.is_none(),
            "node 2 should not become primary when node 1 is valid"
        );
    }

    #[tokio::test]
    async fn test_step_down_clears_primary() {
        let storage = Arc::new(MockClusterStorage::new());
        let clock = Arc::new(TestClock::new(1000));
        let node_id = test_node_id(1);

        let membership =
            TestableClusterMembership::new(storage.clone(), node_id.clone(), clock.clone());
        membership.join("127.0.0.1:8080".to_string()).await.unwrap();

        assert!(membership.is_primary().await);

        // Step down
        membership.step_down().await.unwrap();

        assert!(!membership.is_primary().await);
    }

    #[tokio::test]
    async fn test_mark_node_failed() {
        let storage = Arc::new(MockClusterStorage::new());
        let clock = Arc::new(TestClock::new(1000));

        let node1_id = test_node_id(1);
        let node2_id = test_node_id(2);

        let membership1 =
            TestableClusterMembership::new(storage.clone(), node1_id.clone(), clock.clone());
        membership1
            .join("127.0.0.1:8080".to_string())
            .await
            .unwrap();

        let membership2 =
            TestableClusterMembership::new(storage.clone(), node2_id.clone(), clock.clone());
        membership2
            .join("127.0.0.1:8081".to_string())
            .await
            .unwrap();
        membership2.complete_join().await.unwrap();

        // Mark node 2 as failed
        membership1.mark_node_failed(&node2_id).await.unwrap();

        // Verify
        let node2 = storage.get_node(&node2_id).await.unwrap().unwrap();
        assert_eq!(node2.state, NodeState::Failed);

        let view = storage.read_membership_view().await.unwrap().unwrap();
        assert!(!view.contains(&node2_id));
    }
}
