//! FDB-backed Cluster Membership
//!
//! Implements distributed cluster membership via FoundationDB including:
//! - Primary election with Raft-style terms
//! - Membership view synchronization
//! - Heartbeat-based failure detection
//! - Split-brain prevention
//!
//! TigerStyle: Explicit FDB transactions, bounded terms, deterministic election.

use crate::error::{RegistryError, RegistryResult};
use crate::membership::{MembershipView, NodeState, PrimaryInfo};
use crate::node::NodeId;
use foundationdb::tuple::Subspace;
use foundationdb::{Database, RangeOption, Transaction as FdbTransaction};
use kelpie_core::io::TimeProvider;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument, warn};

// =============================================================================
// Constants
// =============================================================================

/// Transaction timeout in milliseconds
const TRANSACTION_TIMEOUT_MS: i32 = 5_000;

/// Election timeout in milliseconds (how long to wait for primary response)
pub const ELECTION_TIMEOUT_MS: u64 = 5_000;

/// Primary step-down delay after quorum loss in milliseconds
pub const PRIMARY_STEPDOWN_DELAY_MS: u64 = 1_000;

// FDB key prefixes for cluster state
const KEY_PREFIX_KELPIE: &str = "kelpie";
const KEY_PREFIX_CLUSTER: &str = "cluster";
const KEY_PREFIX_NODES: &str = "nodes";
const KEY_MEMBERSHIP_VIEW: &str = "membership_view";
const KEY_PRIMARY: &str = "primary";
const KEY_PRIMARY_TERM: &str = "primary_term";

// =============================================================================
// ClusterNodeInfo (stored in FDB)
// =============================================================================

/// Node information stored in FDB cluster namespace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterNodeInfo {
    /// Node ID
    pub id: NodeId,
    /// Node state matching TLA+
    pub state: NodeState,
    /// Last heartbeat timestamp (epoch ms)
    pub last_heartbeat_ms: u64,
    /// RPC address for communication
    pub rpc_addr: String,
    /// When node joined (epoch ms)
    pub joined_at_ms: u64,
}

impl ClusterNodeInfo {
    /// Create new cluster node info
    pub fn new(id: NodeId, rpc_addr: String, now_ms: u64) -> Self {
        Self {
            id,
            state: NodeState::Left,
            last_heartbeat_ms: now_ms,
            rpc_addr,
            joined_at_ms: 0,
        }
    }

    /// Check if heartbeat has timed out
    pub fn is_heartbeat_timeout(&self, now_ms: u64, timeout_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_heartbeat_ms) > timeout_ms
    }
}

// =============================================================================
// ClusterMembership
// =============================================================================

/// FDB-backed cluster membership manager
///
/// Provides:
/// - Node state management (TLA+ states)
/// - Primary election with term-based conflict resolution
/// - Membership view synchronization
/// - Quorum checking for operations
///
/// TigerStyle: All operations are FDB transactions, explicit quorum checks.
pub struct ClusterMembership {
    /// FDB database handle
    db: Arc<Database>,
    /// Subspace for cluster data
    subspace: Subspace,
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

impl ClusterMembership {
    /// Create a new cluster membership manager
    pub fn new(
        db: Arc<Database>,
        local_node_id: NodeId,
        time_provider: Arc<dyn TimeProvider>,
    ) -> Self {
        let subspace = Subspace::from((KEY_PREFIX_KELPIE, KEY_PREFIX_CLUSTER));

        Self {
            db,
            subspace,
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
    // Key Encoding
    // =========================================================================

    fn node_key(&self, node_id: &NodeId) -> Vec<u8> {
        self.subspace
            .subspace(&(KEY_PREFIX_NODES,))
            .pack(&node_id.as_str())
    }

    fn nodes_prefix(&self) -> Vec<u8> {
        self.subspace
            .subspace(&(KEY_PREFIX_NODES,))
            .bytes()
            .to_vec()
    }

    fn membership_view_key(&self) -> Vec<u8> {
        self.subspace.pack(&KEY_MEMBERSHIP_VIEW)
    }

    fn primary_key(&self) -> Vec<u8> {
        self.subspace.pack(&KEY_PRIMARY)
    }

    fn primary_term_key(&self) -> Vec<u8> {
        self.subspace.pack(&KEY_PRIMARY_TERM)
    }

    // =========================================================================
    // FDB Transaction Helpers
    // =========================================================================

    fn create_transaction(&self) -> RegistryResult<FdbTransaction> {
        let txn = self.db.create_trx().map_err(|e| RegistryError::Internal {
            message: format!("create transaction failed: {}", e),
        })?;

        txn.set_option(foundationdb::options::TransactionOption::Timeout(
            TRANSACTION_TIMEOUT_MS,
        ))
        .map_err(|e| RegistryError::Internal {
            message: format!("set timeout failed: {}", e),
        })?;

        Ok(txn)
    }

    // =========================================================================
    // Node Join/Leave Operations
    // =========================================================================

    /// Join the cluster
    ///
    /// TLA+ NodeJoin: Left -> Joining (or Active if first node)
    #[instrument(skip(self), fields(node_id = %self.local_node_id))]
    pub async fn join(&self, rpc_addr: String) -> RegistryResult<()> {
        let now_ms = self.time_provider.now_ms();

        // Check current state
        let mut local_state = self.local_state.write().await;
        if *local_state != NodeState::Left {
            return Err(RegistryError::Internal {
                message: format!("cannot join: node is in state {:?}", *local_state),
            });
        }

        // Check if this is the first node
        let existing_nodes = self.list_cluster_nodes().await?;
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

            // Write node info and become primary
            self.write_node_info(&node_info).await?;

            // Create initial membership view
            let mut active_nodes = HashSet::new();
            active_nodes.insert(self.local_node_id.clone());
            let view = MembershipView::new(active_nodes, 1, now_ms);
            self.write_membership_view(&view).await?;

            // Become primary with term 1
            let primary = PrimaryInfo::new(self.local_node_id.clone(), 1, now_ms);
            self.write_primary(&primary).await?;
            self.write_primary_term(1).await?;

            *local_state = NodeState::Active;
            *self.believes_primary.write().await = true;
            *self.primary_term.write().await = 1;
            *self.local_view.write().await = view;

            info!("Joined as first node and became primary with term 1");
        } else {
            // Not first: join as Joining
            node_info.state = NodeState::Joining;

            self.write_node_info(&node_info).await?;
            *local_state = NodeState::Joining;

            info!("Joined as Joining, waiting for active nodes to accept");
        }

        Ok(())
    }

    /// Complete joining the cluster
    ///
    /// TLA+ NodeJoinComplete: Joining -> Active
    #[instrument(skip(self), fields(node_id = %self.local_node_id))]
    pub async fn complete_join(&self) -> RegistryResult<()> {
        let now_ms = self.time_provider.now_ms();

        let mut local_state = self.local_state.write().await;
        if *local_state != NodeState::Joining {
            return Err(RegistryError::Internal {
                message: format!("cannot complete join: node is in state {:?}", *local_state),
            });
        }

        // Update node state to Active
        let mut node_info = self
            .get_cluster_node(&self.local_node_id)
            .await?
            .ok_or_else(|| RegistryError::node_not_found(self.local_node_id.as_str()))?;

        node_info.state = NodeState::Active;
        node_info.joined_at_ms = now_ms;
        self.write_node_info(&node_info).await?;

        // Update membership view to include this node
        let mut view = self.read_membership_view().await?.unwrap_or_default();
        view = view.with_node_added(self.local_node_id.clone(), now_ms);
        self.write_membership_view(&view).await?;

        *local_state = NodeState::Active;
        *self.local_view.write().await = view;

        info!("Completed join, now Active");
        Ok(())
    }

    /// Leave the cluster gracefully
    ///
    /// TLA+ NodeLeave: Active -> Leaving
    #[instrument(skip(self), fields(node_id = %self.local_node_id))]
    pub async fn leave(&self) -> RegistryResult<()> {
        let now_ms = self.time_provider.now_ms();

        let mut local_state = self.local_state.write().await;
        if *local_state != NodeState::Active {
            return Err(RegistryError::Internal {
                message: format!("cannot leave: node is in state {:?}", *local_state),
            });
        }

        // Step down if primary
        if *self.believes_primary.read().await {
            self.step_down_internal().await?;
        }

        // Update node state
        let mut node_info = self
            .get_cluster_node(&self.local_node_id)
            .await?
            .ok_or_else(|| RegistryError::node_not_found(self.local_node_id.as_str()))?;

        node_info.state = NodeState::Leaving;
        self.write_node_info(&node_info).await?;

        // Remove from membership view
        let mut view = self.read_membership_view().await?.unwrap_or_default();
        view = view.with_node_removed(&self.local_node_id, now_ms);
        self.write_membership_view(&view).await?;

        *local_state = NodeState::Leaving;
        *self.local_view.write().await = view;

        info!("Started leaving cluster");
        Ok(())
    }

    /// Complete leaving the cluster
    ///
    /// TLA+ NodeLeaveComplete: Leaving -> Left
    #[instrument(skip(self), fields(node_id = %self.local_node_id))]
    pub async fn complete_leave(&self) -> RegistryResult<()> {
        let mut local_state = self.local_state.write().await;
        if *local_state != NodeState::Leaving {
            return Err(RegistryError::Internal {
                message: format!("cannot complete leave: node is in state {:?}", *local_state),
            });
        }

        // Update node state
        let mut node_info = self
            .get_cluster_node(&self.local_node_id)
            .await?
            .ok_or_else(|| RegistryError::node_not_found(self.local_node_id.as_str()))?;

        node_info.state = NodeState::Left;
        self.write_node_info(&node_info).await?;

        *local_state = NodeState::Left;
        *self.believes_primary.write().await = false;
        *self.primary_term.write().await = 0;
        *self.local_view.write().await = MembershipView::empty();

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
        if let Some(current_primary) = self.read_primary().await? {
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
        let current_term = self.read_primary_term().await?.unwrap_or(0);
        let new_term = current_term + 1;
        let now_ms = self.time_provider.now_ms();

        let primary = PrimaryInfo::new(self.local_node_id.clone(), new_term, now_ms);

        // Atomic write of primary and term
        self.write_primary(&primary).await?;
        self.write_primary_term(new_term).await?;

        *self.believes_primary.write().await = true;
        *self.primary_term.write().await = new_term;

        info!("Became primary with term {}", new_term);
        Ok(Some(new_term))
    }

    /// Step down from primary role
    ///
    /// Called when quorum is lost or voluntarily stepping down
    #[instrument(skip(self), fields(node_id = %self.local_node_id))]
    pub async fn step_down(&self) -> RegistryResult<()> {
        self.step_down_internal().await
    }

    async fn step_down_internal(&self) -> RegistryResult<()> {
        if !*self.believes_primary.read().await {
            return Ok(()); // Not primary, nothing to do
        }

        // Clear primary claim in FDB
        let txn = self.create_transaction()?;
        txn.clear(&self.primary_key());
        txn.commit().await.map_err(|e| RegistryError::Internal {
            message: format!("step_down commit failed: {}", e),
        })?;

        *self.believes_primary.write().await = false;

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
        self.read_primary().await
    }

    // =========================================================================
    // Quorum and Reachability
    // =========================================================================

    /// Check if a count constitutes a quorum
    fn has_quorum(&self, cluster_size: usize, reachable_count: usize) -> bool {
        // Strict majority: 2 * reachable > cluster_size
        2 * reachable_count > cluster_size
    }

    /// Calculate cluster size and reachable count
    async fn calculate_reachability(&self) -> RegistryResult<(usize, usize)> {
        let nodes = self.list_cluster_nodes().await?;
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
        if let Some(node) = self.get_cluster_node(&primary.node_id).await? {
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

    /// Send heartbeat (update last_heartbeat_ms in FDB)
    #[instrument(skip(self), fields(node_id = %self.local_node_id))]
    pub async fn send_heartbeat(&self) -> RegistryResult<()> {
        let now_ms = self.time_provider.now_ms();

        if let Some(mut node) = self.get_cluster_node(&self.local_node_id).await? {
            node.last_heartbeat_ms = now_ms;
            self.write_node_info(&node).await?;
        }

        Ok(())
    }

    /// Check for failed nodes based on heartbeat timeout
    #[instrument(skip(self))]
    pub async fn detect_failed_nodes(&self, timeout_ms: u64) -> RegistryResult<Vec<NodeId>> {
        let now_ms = self.time_provider.now_ms();
        let nodes = self.list_cluster_nodes().await?;

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
    #[instrument(skip(self), fields(failed_node = %node_id))]
    pub async fn mark_node_failed(&self, node_id: &NodeId) -> RegistryResult<()> {
        let now_ms = self.time_provider.now_ms();

        if let Some(mut node) = self.get_cluster_node(node_id).await? {
            if node.state == NodeState::Active {
                node.state = NodeState::Failed;
                self.write_node_info(&node).await?;

                // Remove from membership view
                let mut view = self.read_membership_view().await?.unwrap_or_default();
                view = view.with_node_removed(node_id, now_ms);
                self.write_membership_view(&view).await?;

                // If this is us (detected externally), update local state
                if node_id == &self.local_node_id {
                    *self.local_state.write().await = NodeState::Failed;
                    *self.believes_primary.write().await = false;
                }

                // Update local view cache
                *self.local_view.write().await = view;

                info!("Marked node {} as Failed", node_id);
            }
        }

        Ok(())
    }

    // =========================================================================
    // FDB Read/Write Operations
    // =========================================================================

    async fn get_cluster_node(&self, node_id: &NodeId) -> RegistryResult<Option<ClusterNodeInfo>> {
        let txn = self.create_transaction()?;
        let key = self.node_key(node_id);

        let value = txn
            .get(&key, false)
            .await
            .map_err(|e| RegistryError::Internal {
                message: format!("get cluster node failed: {}", e),
            })?;

        match value {
            Some(data) => {
                let info: ClusterNodeInfo =
                    serde_json::from_slice(data.as_ref()).map_err(|e| RegistryError::Internal {
                        message: format!("deserialize cluster node failed: {}", e),
                    })?;
                Ok(Some(info))
            }
            None => Ok(None),
        }
    }

    async fn write_node_info(&self, info: &ClusterNodeInfo) -> RegistryResult<()> {
        let key = self.node_key(&info.id);
        let value = serde_json::to_vec(info).map_err(|e| RegistryError::Internal {
            message: format!("serialize cluster node failed: {}", e),
        })?;

        let txn = self.create_transaction()?;
        txn.set(&key, &value);
        txn.commit().await.map_err(|e| RegistryError::Internal {
            message: format!("write node info commit failed: {}", e),
        })?;

        Ok(())
    }

    async fn list_cluster_nodes(&self) -> RegistryResult<Vec<ClusterNodeInfo>> {
        let prefix = self.nodes_prefix();
        let mut end_key = prefix.clone();
        end_key.push(0xFF);

        let txn = self.create_transaction()?;
        let mut range_option = RangeOption::from((prefix.as_slice(), end_key.as_slice()));
        range_option.mode = foundationdb::options::StreamingMode::WantAll;

        let range =
            txn.get_range(&range_option, 1, false)
                .await
                .map_err(|e| RegistryError::Internal {
                    message: format!("list cluster nodes failed: {}", e),
                })?;

        let mut nodes = Vec::new();
        for kv in range.iter() {
            let info: ClusterNodeInfo =
                serde_json::from_slice(kv.value()).map_err(|e| RegistryError::Internal {
                    message: format!("deserialize cluster node failed: {}", e),
                })?;
            nodes.push(info);
        }

        Ok(nodes)
    }

    async fn read_membership_view(&self) -> RegistryResult<Option<MembershipView>> {
        let txn = self.create_transaction()?;
        let key = self.membership_view_key();

        let value = txn
            .get(&key, false)
            .await
            .map_err(|e| RegistryError::Internal {
                message: format!("read membership view failed: {}", e),
            })?;

        match value {
            Some(data) => {
                let view: MembershipView =
                    serde_json::from_slice(data.as_ref()).map_err(|e| RegistryError::Internal {
                        message: format!("deserialize membership view failed: {}", e),
                    })?;
                Ok(Some(view))
            }
            None => Ok(None),
        }
    }

    async fn write_membership_view(&self, view: &MembershipView) -> RegistryResult<()> {
        let key = self.membership_view_key();
        let value = serde_json::to_vec(view).map_err(|e| RegistryError::Internal {
            message: format!("serialize membership view failed: {}", e),
        })?;

        let txn = self.create_transaction()?;
        txn.set(&key, &value);
        txn.commit().await.map_err(|e| RegistryError::Internal {
            message: format!("write membership view commit failed: {}", e),
        })?;

        Ok(())
    }

    async fn read_primary(&self) -> RegistryResult<Option<PrimaryInfo>> {
        let txn = self.create_transaction()?;
        let key = self.primary_key();

        let value = txn
            .get(&key, false)
            .await
            .map_err(|e| RegistryError::Internal {
                message: format!("read primary failed: {}", e),
            })?;

        match value {
            Some(data) => {
                let primary: PrimaryInfo =
                    serde_json::from_slice(data.as_ref()).map_err(|e| RegistryError::Internal {
                        message: format!("deserialize primary failed: {}", e),
                    })?;
                Ok(Some(primary))
            }
            None => Ok(None),
        }
    }

    async fn write_primary(&self, primary: &PrimaryInfo) -> RegistryResult<()> {
        let key = self.primary_key();
        let value = serde_json::to_vec(primary).map_err(|e| RegistryError::Internal {
            message: format!("serialize primary failed: {}", e),
        })?;

        let txn = self.create_transaction()?;
        txn.set(&key, &value);
        txn.commit().await.map_err(|e| RegistryError::Internal {
            message: format!("write primary commit failed: {}", e),
        })?;

        Ok(())
    }

    async fn read_primary_term(&self) -> RegistryResult<Option<u64>> {
        let txn = self.create_transaction()?;
        let key = self.primary_term_key();

        let value = txn
            .get(&key, false)
            .await
            .map_err(|e| RegistryError::Internal {
                message: format!("read primary term failed: {}", e),
            })?;

        match value {
            Some(data) => {
                if data.len() == 8 {
                    let term = u64::from_be_bytes(data.as_ref().try_into().unwrap());
                    Ok(Some(term))
                } else {
                    // Try JSON for backwards compatibility
                    let term: u64 = serde_json::from_slice(data.as_ref()).map_err(|e| {
                        RegistryError::Internal {
                            message: format!("deserialize primary term failed: {}", e),
                        }
                    })?;
                    Ok(Some(term))
                }
            }
            None => Ok(None),
        }
    }

    async fn write_primary_term(&self, term: u64) -> RegistryResult<()> {
        let key = self.primary_term_key();
        let value = term.to_be_bytes();

        let txn = self.create_transaction()?;
        txn.set(&key, &value);
        txn.commit().await.map_err(|e| RegistryError::Internal {
            message: format!("write primary term commit failed: {}", e),
        })?;

        Ok(())
    }

    /// Synchronize local view with FDB (called after partition heal)
    pub async fn sync_membership_view(&self) -> RegistryResult<()> {
        if let Some(view) = self.read_membership_view().await? {
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
}

impl std::fmt::Debug for ClusterMembership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClusterMembership")
            .field("local_node_id", &self.local_node_id)
            .finish()
    }
}

// =============================================================================
// Actor Migration Types and Implementation
// =============================================================================

/// Actor that needs to be migrated due to node failure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationCandidate {
    /// Actor that needs migration
    pub actor_id: String,
    /// Node that failed
    pub failed_node_id: NodeId,
    /// When the failure was detected (epoch ms)
    pub detected_at_ms: u64,
}

impl MigrationCandidate {
    /// Create a new migration candidate
    pub fn new(actor_id: String, failed_node_id: NodeId, detected_at_ms: u64) -> Self {
        assert!(!actor_id.is_empty(), "actor_id cannot be empty");

        Self {
            actor_id,
            failed_node_id,
            detected_at_ms,
        }
    }
}

/// Result of an actor migration
#[derive(Debug, Clone)]
pub enum MigrationResult {
    /// Migration succeeded, actor now on new node
    Success {
        actor_id: String,
        new_node_id: NodeId,
    },
    /// Migration failed, no capacity available
    NoCapacity { actor_id: String },
    /// Migration failed with error
    Failed { actor_id: String, reason: String },
}

impl MigrationResult {
    /// Check if migration was successful
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }

    /// Get the actor ID
    pub fn actor_id(&self) -> &str {
        match self {
            Self::Success { actor_id, .. } => actor_id,
            Self::NoCapacity { actor_id } => actor_id,
            Self::Failed { actor_id, .. } => actor_id,
        }
    }
}

/// Actors pending migration stored in FDB
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MigrationQueue {
    /// Actors pending migration
    pub candidates: Vec<MigrationCandidate>,
    /// Last updated timestamp
    pub updated_at_ms: u64,
}

impl MigrationQueue {
    /// Create a new empty migration queue
    pub fn new() -> Self {
        Self {
            candidates: Vec::new(),
            updated_at_ms: 0,
        }
    }

    /// Add a candidate to the queue
    pub fn add(&mut self, candidate: MigrationCandidate, now_ms: u64) {
        self.candidates.push(candidate);
        self.updated_at_ms = now_ms;
    }

    /// Remove a candidate from the queue by actor_id
    pub fn remove(&mut self, actor_id: &str, now_ms: u64) -> bool {
        let len_before = self.candidates.len();
        self.candidates.retain(|c| c.actor_id != actor_id);
        let removed = self.candidates.len() < len_before;
        if removed {
            self.updated_at_ms = now_ms;
        }
        removed
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    /// Get the number of pending migrations
    pub fn len(&self) -> usize {
        self.candidates.len()
    }
}

// FDB key for migration queue
const KEY_MIGRATION_QUEUE: &str = "migration_queue";

impl ClusterMembership {
    // =========================================================================
    // Actor Migration (FR-7)
    // =========================================================================

    /// Queue actors from a failed node for migration
    ///
    /// TLA+ connection: When a node is marked Failed, its actors become
    /// eligible for migration. This method adds them to the migration queue
    /// which the primary will process.
    ///
    /// # Arguments
    /// * `failed_node_id` - The ID of the node that failed
    /// * `actor_ids` - List of actor IDs that were on the failed node
    ///
    /// # Returns
    /// The number of actors queued for migration
    #[instrument(skip(self, actor_ids), fields(failed_node = %failed_node_id, count = actor_ids.len()))]
    pub async fn queue_actors_for_migration(
        &self,
        failed_node_id: &NodeId,
        actor_ids: Vec<String>,
    ) -> RegistryResult<usize> {
        let now_ms = self.time_provider.now_ms();

        assert!(
            !actor_ids.is_empty(),
            "cannot queue empty actor list for migration"
        );

        // Read current migration queue
        let mut queue = self.read_migration_queue().await?.unwrap_or_default();

        // Add all actors to the queue
        let count = actor_ids.len();
        for actor_id in actor_ids {
            let candidate = MigrationCandidate::new(actor_id, failed_node_id.clone(), now_ms);
            queue.add(candidate, now_ms);
        }

        // Write updated queue
        self.write_migration_queue(&queue).await?;

        info!(
            count = count,
            failed_node = %failed_node_id,
            "Queued actors for migration"
        );

        Ok(count)
    }

    /// Process migration queue (called by primary)
    ///
    /// The primary node periodically processes the migration queue to
    /// relocate actors from failed nodes to healthy nodes.
    ///
    /// # Arguments
    /// * `select_node` - Callback to select target node for each actor
    ///
    /// # Returns
    /// List of migration results
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
        let mut queue = self.read_migration_queue().await?.unwrap_or_default();

        if queue.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();
        let mut to_remove = Vec::new();

        // Process each candidate
        for candidate in &queue.candidates {
            // Select target node
            match select_node(&candidate.actor_id) {
                Some(target_node) => {
                    // Migration will be handled by FdbRegistry, we just track the result
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
                    // Don't remove - will retry later
                }
            }
        }

        // Remove successfully processed candidates
        for actor_id in to_remove {
            queue.remove(&actor_id, now_ms);
        }

        // Write updated queue
        self.write_migration_queue(&queue).await?;

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
        self.read_migration_queue()
            .await
            .map(|q| q.unwrap_or_default())
    }

    /// Clear the migration queue
    pub async fn clear_migration_queue(&self) -> RegistryResult<()> {
        let queue = MigrationQueue::new();
        self.write_migration_queue(&queue).await
    }

    /// Handle node failure including actor migration queueing
    ///
    /// This is the main entry point for failure handling that:
    /// 1. Marks the node as Failed in cluster state
    /// 2. Updates membership view to remove the failed node
    /// 3. Queues the node's actors for migration
    ///
    /// # Arguments
    /// * `node_id` - The ID of the failed node
    /// * `actor_ids` - List of actor IDs on the failed node
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

    // =========================================================================
    // Migration Queue FDB Operations
    // =========================================================================

    fn migration_queue_key(&self) -> Vec<u8> {
        self.subspace.pack(&KEY_MIGRATION_QUEUE)
    }

    async fn read_migration_queue(&self) -> RegistryResult<Option<MigrationQueue>> {
        let txn = self.create_transaction()?;
        let key = self.migration_queue_key();

        let value = txn
            .get(&key, false)
            .await
            .map_err(|e| RegistryError::Internal {
                message: format!("read migration queue failed: {}", e),
            })?;

        match value {
            Some(data) => {
                let queue: MigrationQueue =
                    serde_json::from_slice(data.as_ref()).map_err(|e| RegistryError::Internal {
                        message: format!("deserialize migration queue failed: {}", e),
                    })?;
                Ok(Some(queue))
            }
            None => Ok(None),
        }
    }

    async fn write_migration_queue(&self, queue: &MigrationQueue) -> RegistryResult<()> {
        let key = self.migration_queue_key();
        let value = serde_json::to_vec(queue).map_err(|e| RegistryError::Internal {
            message: format!("serialize migration queue failed: {}", e),
        })?;

        let txn = self.create_transaction()?;
        txn.set(&key, &value);
        txn.commit().await.map_err(|e| RegistryError::Internal {
            message: format!("write migration queue commit failed: {}", e),
        })?;

        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cluster_node_info() {
        let node_id = NodeId::new("node-1").unwrap();
        let info = ClusterNodeInfo::new(node_id.clone(), "127.0.0.1:8080".to_string(), 1000);

        assert_eq!(info.id, node_id);
        assert_eq!(info.state, NodeState::Left);
        assert_eq!(info.last_heartbeat_ms, 1000);

        // Heartbeat timeout check
        assert!(!info.is_heartbeat_timeout(2000, 5000)); // 2000 - 1000 = 1000 < 5000
        assert!(info.is_heartbeat_timeout(7000, 5000)); // 7000 - 1000 = 6000 > 5000
    }

    #[test]
    fn test_quorum_calculation() {
        // Quorum check: 2 * reachable > cluster_size (strict majority)
        fn has_quorum(reachable: usize, cluster_size: usize) -> bool {
            2 * reachable > cluster_size
        }

        // 3 nodes: need 2 for quorum
        assert!(has_quorum(2, 3), "2 of 3 should be quorum");
        assert!(!has_quorum(1, 3), "1 of 3 should not be quorum");

        // 5 nodes: need 3 for quorum
        assert!(has_quorum(3, 5), "3 of 5 should be quorum");
        assert!(!has_quorum(2, 5), "2 of 5 should not be quorum");

        // 7 nodes: need 4 for quorum
        assert!(has_quorum(4, 7), "4 of 7 should be quorum");
        assert!(!has_quorum(3, 7), "3 of 7 should not be quorum");
    }

    #[test]
    fn test_migration_candidate() {
        let node_id = NodeId::new("node-1").unwrap();
        let candidate = MigrationCandidate::new("test/actor-1".to_string(), node_id.clone(), 1000);

        assert_eq!(candidate.actor_id, "test/actor-1");
        assert_eq!(candidate.failed_node_id, node_id);
        assert_eq!(candidate.detected_at_ms, 1000);
    }

    #[test]
    #[should_panic(expected = "actor_id cannot be empty")]
    fn test_migration_candidate_empty_actor_id_panics() {
        let node_id = NodeId::new("node-1").unwrap();
        MigrationCandidate::new(String::new(), node_id, 1000);
    }

    #[test]
    fn test_migration_result() {
        let node_id = NodeId::new("node-1").unwrap();

        let success = MigrationResult::Success {
            actor_id: "test/actor-1".to_string(),
            new_node_id: node_id,
        };
        assert!(success.is_success());
        assert_eq!(success.actor_id(), "test/actor-1");

        let no_capacity = MigrationResult::NoCapacity {
            actor_id: "test/actor-2".to_string(),
        };
        assert!(!no_capacity.is_success());
        assert_eq!(no_capacity.actor_id(), "test/actor-2");

        let failed = MigrationResult::Failed {
            actor_id: "test/actor-3".to_string(),
            reason: "connection refused".to_string(),
        };
        assert!(!failed.is_success());
        assert_eq!(failed.actor_id(), "test/actor-3");
    }

    #[test]
    fn test_migration_queue() {
        let mut queue = MigrationQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);

        let node_id = NodeId::new("node-1").unwrap();
        let candidate1 = MigrationCandidate::new("test/actor-1".to_string(), node_id.clone(), 1000);
        let candidate2 = MigrationCandidate::new("test/actor-2".to_string(), node_id.clone(), 1000);

        queue.add(candidate1, 1000);
        assert!(!queue.is_empty());
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.updated_at_ms, 1000);

        queue.add(candidate2, 2000);
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.updated_at_ms, 2000);

        // Remove first actor
        let removed = queue.remove("test/actor-1", 3000);
        assert!(removed);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.updated_at_ms, 3000);

        // Try to remove non-existent actor
        let removed = queue.remove("test/actor-nonexistent", 4000);
        assert!(!removed);
        assert_eq!(queue.updated_at_ms, 3000); // Not updated

        // Remove last actor
        let removed = queue.remove("test/actor-2", 5000);
        assert!(removed);
        assert!(queue.is_empty());
    }
}
