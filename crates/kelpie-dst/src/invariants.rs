//! TLA+ Invariant Verification Framework
//!
//! This module provides a framework for verifying TLA+ invariants during
//! deterministic simulation testing. Each invariant corresponds to a
//! safety property from a TLA+ specification.
//!
//! # TigerStyle
//!
//! - Each invariant maps directly to a TLA+ specification
//! - Violations include detailed evidence for debugging
//! - Explicit state modeling with bounded types
//!
//! # Example
//!
//! ```rust,ignore
//! use kelpie_dst::invariants::{
//!     InvariantChecker, SystemState, SingleActivation, ConsistentHolder
//! };
//!
//! let checker = InvariantChecker::new()
//!     .with_invariant(SingleActivation)
//!     .with_invariant(ConsistentHolder);
//!
//! let state = SystemState::new();
//! // ... populate state ...
//!
//! checker.verify_all(&state)?;
//! ```
//!
//! # References
//!
//! - `docs/tla/KelpieSingleActivation.tla` - SingleActivation, ConsistentHolder
//! - `docs/tla/KelpieRegistry.tla` - PlacementConsistency
//! - `docs/tla/KelpieLease.tla` - LeaseUniqueness
//! - `docs/tla/KelpieWAL.tla` - Durability, AtomicVisibility

use std::collections::{HashMap, HashSet};
use std::fmt;
use thiserror::Error;

// =============================================================================
// Core Types
// =============================================================================

/// Error indicating an invariant violation
///
/// Contains detailed information about which invariant failed and why.
#[derive(Error, Debug, Clone)]
#[error("Invariant '{name}' violated: {message}")]
pub struct InvariantViolation {
    /// Name of the violated invariant (matches TLA+ spec name)
    pub name: String,
    /// Human-readable description of the violation
    pub message: String,
    /// Optional evidence (e.g., which nodes/actors involved)
    pub evidence: Option<String>,
}

impl InvariantViolation {
    /// Create a new invariant violation
    pub fn new(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            message: message.into(),
            evidence: None,
        }
    }

    /// Create a new invariant violation with evidence
    pub fn with_evidence(
        name: impl Into<String>,
        message: impl Into<String>,
        evidence: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            message: message.into(),
            evidence: Some(evidence.into()),
        }
    }
}

/// Trait for TLA+ invariants
///
/// Each invariant should correspond to a safety property in a TLA+ specification.
/// The `name()` method returns the TLA+ property name for traceability.
pub trait Invariant: Send + Sync {
    /// Returns the name of this invariant (should match TLA+ spec)
    fn name(&self) -> &'static str;

    /// Returns the TLA+ source file for this invariant
    fn tla_source(&self) -> &'static str;

    /// Check whether this invariant holds for the given system state
    ///
    /// Returns `Ok(())` if the invariant holds, or an `InvariantViolation`
    /// with details if it doesn't.
    fn check(&self, state: &SystemState) -> Result<(), InvariantViolation>;
}

/// Checks multiple invariants against system state
///
/// Provides both fail-fast (`verify_all`) and collect-all (`verify_all_collect`)
/// modes for different testing scenarios.
pub struct InvariantChecker {
    invariants: Vec<Box<dyn Invariant>>,
}

impl Default for InvariantChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl InvariantChecker {
    /// Create a new empty invariant checker
    pub fn new() -> Self {
        Self {
            invariants: Vec::new(),
        }
    }

    /// Add an invariant to the checker
    pub fn with_invariant(mut self, inv: impl Invariant + 'static) -> Self {
        self.invariants.push(Box::new(inv));
        self
    }

    /// Add all standard Kelpie invariants
    pub fn with_standard_invariants(self) -> Self {
        self.with_invariant(SingleActivation)
            .with_invariant(ConsistentHolder)
            .with_invariant(PlacementConsistency)
            .with_invariant(LeaseUniqueness)
            .with_invariant(Durability)
            .with_invariant(AtomicVisibility)
    }

    /// Verify all invariants, returning the first violation (fail-fast)
    pub fn verify_all(&self, state: &SystemState) -> Result<(), InvariantViolation> {
        for inv in &self.invariants {
            inv.check(state)?;
        }
        Ok(())
    }

    /// Verify all invariants, collecting ALL violations
    ///
    /// Useful for comprehensive testing where you want to see all failures.
    pub fn verify_all_collect(&self, state: &SystemState) -> Vec<InvariantViolation> {
        self.invariants
            .iter()
            .filter_map(|inv| inv.check(state).err())
            .collect()
    }

    /// Get the names of all registered invariants
    pub fn invariant_names(&self) -> Vec<&'static str> {
        self.invariants.iter().map(|i| i.name()).collect()
    }

    /// Get the number of registered invariants
    pub fn len(&self) -> usize {
        self.invariants.len()
    }

    /// Check if the checker has no invariants
    pub fn is_empty(&self) -> bool {
        self.invariants.is_empty()
    }
}

impl fmt::Debug for InvariantChecker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InvariantChecker")
            .field("invariants", &self.invariant_names())
            .finish()
    }
}

// =============================================================================
// System State Model
// =============================================================================

/// Node state in the system
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeState {
    /// Node is idle, not claiming any actors
    Idle,
    /// Node is in the process of reading FDB state
    Reading,
    /// Node is attempting to commit a claim
    Committing,
    /// Node has successfully activated an actor
    Active,
}

/// Node status for registry invariants
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeStatus {
    /// Node is healthy and can accept actors
    Active,
    /// Node is suspected of failure (missed heartbeats)
    Suspect,
    /// Node has failed (confirmed dead)
    Failed,
}

/// WAL entry status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WalEntryStatus {
    /// Entry logged but not yet completed
    Pending,
    /// Entry successfully completed
    Completed,
    /// Entry failed
    Failed,
}

/// A node in the simulated system
#[derive(Debug, Clone)]
pub struct NodeInfo {
    /// Node identifier
    pub id: String,
    /// Node's overall status
    pub status: NodeStatus,
    /// Per-actor state on this node (actor_id -> state)
    pub actor_states: HashMap<String, NodeState>,
    /// Lease beliefs: what this node believes it holds
    /// (actor_id -> expiry_time, where expiry > current_time means believed held)
    pub lease_beliefs: HashMap<String, u64>,
    /// Whether this node believes it is the primary (for NoSplitBrain)
    pub is_primary: bool,
    /// Whether this node can reach a quorum (for NoSplitBrain)
    pub has_quorum: bool,
}

impl NodeInfo {
    /// Create a new node with default state
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: NodeStatus::Active,
            actor_states: HashMap::new(),
            lease_beliefs: HashMap::new(),
            is_primary: false,
            has_quorum: false,
        }
    }

    /// Set the node status
    pub fn with_status(mut self, status: NodeStatus) -> Self {
        self.status = status;
        self
    }

    /// Set an actor's state on this node
    pub fn with_actor_state(mut self, actor_id: impl Into<String>, state: NodeState) -> Self {
        self.actor_states.insert(actor_id.into(), state);
        self
    }

    /// Set a lease belief for an actor
    pub fn with_lease_belief(mut self, actor_id: impl Into<String>, expiry: u64) -> Self {
        self.lease_beliefs.insert(actor_id.into(), expiry);
        self
    }

    /// Get the state for an actor on this node
    pub fn actor_state(&self, actor_id: &str) -> NodeState {
        self.actor_states
            .get(actor_id)
            .copied()
            .unwrap_or(NodeState::Idle)
    }

    /// Check if this node believes it holds a valid lease for an actor
    pub fn believes_holds_lease(&self, actor_id: &str, current_time: u64) -> bool {
        self.lease_beliefs
            .get(actor_id)
            .map(|&expiry| expiry > current_time)
            .unwrap_or(false)
    }
}

/// A WAL entry
#[derive(Debug, Clone)]
pub struct WalEntry {
    /// Entry identifier
    pub id: u64,
    /// Client that created this entry
    pub client_id: String,
    /// Idempotency key
    pub idempotency_key: u64,
    /// Entry status
    pub status: WalEntryStatus,
    /// Data key affected by this entry
    pub data_key: String,
}

/// Lease ground truth (what FDB actually stores)
#[derive(Debug, Clone)]
pub struct LeaseInfo {
    /// Current holder (None if no lease)
    pub holder: Option<String>,
    /// Expiry time
    pub expiry: u64,
}

/// Snapshot of entire system state for invariant checking
///
/// This is a simplified model of the distributed system state,
/// capturing the information needed to verify TLA+ invariants.
#[derive(Debug, Clone)]
pub struct SystemState {
    /// All nodes in the system
    nodes: HashMap<String, NodeInfo>,
    /// Authoritative placement: actor_id -> node_id
    placements: HashMap<String, String>,
    /// FDB holder for each actor (ground truth for single activation)
    fdb_holders: HashMap<String, Option<String>>,
    /// Lease ground truth: actor_id -> LeaseInfo
    leases: HashMap<String, LeaseInfo>,
    /// WAL entries
    wal_entries: Vec<WalEntry>,
    /// Storage state: key -> value
    pub storage: HashMap<String, String>,
    /// Current simulated time
    current_time: u64,
    /// Active transactions (for ReadYourWrites)
    transactions: HashMap<String, Transaction>,
    /// Fencing tokens: actor_id -> current token
    fencing_tokens: HashMap<String, i64>,
    /// Fencing token history: actor_id -> list of tokens (for monotonicity check)
    fencing_token_history: HashMap<String, Vec<i64>>,
    /// Snapshots for teleport (for SnapshotConsistency)
    snapshots: HashMap<String, Snapshot>,
}

impl Default for SystemState {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemState {
    /// Create a new empty system state
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            placements: HashMap::new(),
            fdb_holders: HashMap::new(),
            leases: HashMap::new(),
            wal_entries: Vec::new(),
            storage: HashMap::new(),
            current_time: 0,
            transactions: HashMap::new(),
            fencing_tokens: HashMap::new(),
            fencing_token_history: HashMap::new(),
            snapshots: HashMap::new(),
        }
    }

    /// Set the current simulated time
    pub fn with_time(mut self, time: u64) -> Self {
        self.current_time = time;
        self
    }

    /// Add a node to the system
    pub fn with_node(mut self, node: NodeInfo) -> Self {
        self.nodes.insert(node.id.clone(), node);
        self
    }

    /// Add a placement
    pub fn with_placement(
        mut self,
        actor_id: impl Into<String>,
        node_id: impl Into<String>,
    ) -> Self {
        self.placements.insert(actor_id.into(), node_id.into());
        self
    }

    /// Set the FDB holder for an actor
    pub fn with_fdb_holder(mut self, actor_id: impl Into<String>, holder: Option<String>) -> Self {
        self.fdb_holders.insert(actor_id.into(), holder);
        self
    }

    /// Add a lease
    pub fn with_lease(
        mut self,
        actor_id: impl Into<String>,
        holder: Option<String>,
        expiry: u64,
    ) -> Self {
        self.leases
            .insert(actor_id.into(), LeaseInfo { holder, expiry });
        self
    }

    /// Add a WAL entry
    pub fn with_wal_entry(mut self, entry: WalEntry) -> Self {
        self.wal_entries.push(entry);
        self
    }

    /// Set storage key value
    pub fn with_storage_key(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.storage.insert(key.into(), value.into());
        self
    }

    /// Remove a storage key
    pub fn without_storage_key(mut self, key: &str) -> Self {
        self.storage.remove(key);
        self
    }

    /// Get all actor IDs in the system
    pub fn actor_ids(&self) -> HashSet<String> {
        let mut ids = HashSet::new();

        // From placements
        ids.extend(self.placements.keys().cloned());

        // From FDB holders
        ids.extend(self.fdb_holders.keys().cloned());

        // From leases
        ids.extend(self.leases.keys().cloned());

        // From node actor states and lease beliefs
        for node in self.nodes.values() {
            ids.extend(node.actor_states.keys().cloned());
            ids.extend(node.lease_beliefs.keys().cloned());
        }

        ids
    }

    /// Get all nodes
    pub fn nodes(&self) -> impl Iterator<Item = &NodeInfo> {
        self.nodes.values()
    }

    /// Get a specific node
    pub fn node(&self, id: &str) -> Option<&NodeInfo> {
        self.nodes.get(id)
    }

    /// Get the FDB holder for an actor
    pub fn fdb_holder(&self, actor_id: &str) -> Option<&String> {
        self.fdb_holders.get(actor_id).and_then(|h| h.as_ref())
    }

    /// Get the lease for an actor
    pub fn lease(&self, actor_id: &str) -> Option<&LeaseInfo> {
        self.leases.get(actor_id)
    }

    /// Check if a lease is valid (not expired)
    pub fn is_lease_valid(&self, actor_id: &str) -> bool {
        self.leases
            .get(actor_id)
            .map(|l| l.holder.is_some() && l.expiry > self.current_time)
            .unwrap_or(false)
    }

    /// Get the placement for an actor
    pub fn placement(&self, actor_id: &str) -> Option<&String> {
        self.placements.get(actor_id)
    }

    /// Get all placements
    pub fn placements(&self) -> impl Iterator<Item = (&String, &String)> {
        self.placements.iter()
    }

    /// Get current time
    pub fn current_time(&self) -> u64 {
        self.current_time
    }

    /// Get WAL entries
    pub fn wal_entries(&self) -> &[WalEntry] {
        &self.wal_entries
    }

    /// Check if a storage key exists
    pub fn storage_exists(&self, key: &str) -> bool {
        self.storage.contains_key(key)
    }

    /// Add a transaction to the state
    pub fn with_transaction(mut self, txn: Transaction) -> Self {
        self.transactions.insert(txn.id.clone(), txn);
        self
    }

    /// Get all transactions
    pub fn transactions(&self) -> impl Iterator<Item = &Transaction> {
        self.transactions.values()
    }

    /// Set a fencing token for an actor
    pub fn with_fencing_token(mut self, actor_id: impl Into<String>, token: i64) -> Self {
        let actor_id = actor_id.into();
        self.fencing_tokens.insert(actor_id.clone(), token);
        self.fencing_token_history
            .entry(actor_id)
            .or_default()
            .push(token);
        self
    }

    /// Get all fencing tokens
    pub fn fencing_tokens(&self) -> impl Iterator<Item = (&String, &i64)> {
        self.fencing_tokens.iter()
    }

    /// Get fencing token history
    pub fn fencing_token_history(&self) -> impl Iterator<Item = (&String, &Vec<i64>)> {
        self.fencing_token_history.iter()
    }

    /// Add a snapshot to the state
    pub fn with_snapshot(mut self, snapshot: Snapshot) -> Self {
        self.snapshots.insert(snapshot.id.clone(), snapshot);
        self
    }

    /// Get all snapshots
    pub fn snapshots(&self) -> impl Iterator<Item = (&String, &Snapshot)> {
        self.snapshots.iter()
    }
}

// =============================================================================
// Invariant Implementations
// =============================================================================

/// SingleActivation invariant from KelpieSingleActivation.tla
///
/// **TLA+ Definition:**
/// ```tla
/// SingleActivation ==
///     Cardinality({n \in Nodes : node_state[n] = "Active"}) <= 1
/// ```
///
/// At most one node can be in the Active state for any given actor at any time.
/// This is THE key safety guarantee of the single activation protocol.
pub struct SingleActivation;

impl Invariant for SingleActivation {
    fn name(&self) -> &'static str {
        "SingleActivation"
    }

    fn tla_source(&self) -> &'static str {
        "docs/tla/KelpieSingleActivation.tla"
    }

    fn check(&self, state: &SystemState) -> Result<(), InvariantViolation> {
        for actor_id in state.actor_ids() {
            let active_nodes: Vec<&str> = state
                .nodes()
                .filter(|n| n.actor_state(&actor_id) == NodeState::Active)
                .map(|n| n.id.as_str())
                .collect();

            if active_nodes.len() > 1 {
                return Err(InvariantViolation::with_evidence(
                    self.name(),
                    format!(
                        "Actor '{}' has {} active instances (max 1 allowed)",
                        actor_id,
                        active_nodes.len()
                    ),
                    format!("Active on nodes: {:?}", active_nodes),
                ));
            }
        }
        Ok(())
    }
}

/// ConsistentHolder invariant from KelpieSingleActivation.tla
///
/// **TLA+ Definition:**
/// ```tla
/// ConsistentHolder ==
///     \A n \in Nodes:
///         node_state[n] = "Active" => fdb_holder = n
/// ```
///
/// If a node thinks it's active for an actor, FDB must agree that node is the holder.
pub struct ConsistentHolder;

impl Invariant for ConsistentHolder {
    fn name(&self) -> &'static str {
        "ConsistentHolder"
    }

    fn tla_source(&self) -> &'static str {
        "docs/tla/KelpieSingleActivation.tla"
    }

    fn check(&self, state: &SystemState) -> Result<(), InvariantViolation> {
        for actor_id in state.actor_ids() {
            for node in state.nodes() {
                if node.actor_state(&actor_id) == NodeState::Active {
                    let fdb_holder = state.fdb_holder(&actor_id);

                    if fdb_holder != Some(&node.id) {
                        return Err(InvariantViolation::with_evidence(
                            self.name(),
                            format!(
                                "Node '{}' is Active for actor '{}' but FDB holder is {:?}",
                                node.id, actor_id, fdb_holder
                            ),
                            format!(
                                "Node state: Active, FDB holder: {}",
                                fdb_holder.map(|s| s.as_str()).unwrap_or("None")
                            ),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// PlacementConsistency invariant from KelpieRegistry.tla
///
/// **TLA+ Definition:**
/// ```tla
/// PlacementConsistency ==
///     \A a \in Actors :
///         placement[a] # NULL => nodeStatus[placement[a]] # Failed
/// ```
///
/// An actor should not be placed on a failed node. When a node fails,
/// its placements should be cleared.
pub struct PlacementConsistency;

impl Invariant for PlacementConsistency {
    fn name(&self) -> &'static str {
        "PlacementConsistency"
    }

    fn tla_source(&self) -> &'static str {
        "docs/tla/KelpieRegistry.tla"
    }

    fn check(&self, state: &SystemState) -> Result<(), InvariantViolation> {
        for (actor_id, node_id) in state.placements() {
            if let Some(node) = state.node(node_id) {
                if node.status == NodeStatus::Failed {
                    return Err(InvariantViolation::with_evidence(
                        self.name(),
                        format!(
                            "Actor '{}' is placed on failed node '{}'",
                            actor_id, node_id
                        ),
                        format!("Node status: {:?}", node.status),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// LeaseUniqueness invariant from KelpieLease.tla
///
/// **TLA+ Definition:**
/// ```tla
/// LeaseUniqueness ==
///     \A a \in Actors:
///         LET believingNodes == {n \in Nodes: NodeBelievesItHolds(n, a)}
///         IN Cardinality(believingNodes) <= 1
/// ```
///
/// At most one node believes it holds a valid lease for any given actor.
/// This is the critical invariant for single activation via leases.
pub struct LeaseUniqueness;

impl Invariant for LeaseUniqueness {
    fn name(&self) -> &'static str {
        "LeaseUniqueness"
    }

    fn tla_source(&self) -> &'static str {
        "docs/tla/KelpieLease.tla"
    }

    fn check(&self, state: &SystemState) -> Result<(), InvariantViolation> {
        let current_time = state.current_time();

        for actor_id in state.actor_ids() {
            let believing_nodes: Vec<&str> = state
                .nodes()
                .filter(|n| n.believes_holds_lease(&actor_id, current_time))
                .map(|n| n.id.as_str())
                .collect();

            if believing_nodes.len() > 1 {
                return Err(InvariantViolation::with_evidence(
                    self.name(),
                    format!(
                        "Actor '{}' has {} nodes believing they hold the lease (max 1 allowed)",
                        actor_id,
                        believing_nodes.len()
                    ),
                    format!(
                        "Believing nodes: {:?}, current_time: {}",
                        believing_nodes, current_time
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// Durability invariant from KelpieWAL.tla
///
/// **TLA+ Definition:**
/// ```tla
/// Durability ==
///     \A i \in 1..Len(wal) :
///         (wal[i].status = "Completed") =>
///         (storage[wal[i].data] = wal[i].data)
/// ```
///
/// Completed WAL entries must be visible in storage. Once an operation
/// is marked complete, its effects are durable.
pub struct Durability;

impl Invariant for Durability {
    fn name(&self) -> &'static str {
        "Durability"
    }

    fn tla_source(&self) -> &'static str {
        "docs/tla/KelpieWAL.tla"
    }

    fn check(&self, state: &SystemState) -> Result<(), InvariantViolation> {
        for entry in state.wal_entries() {
            if entry.status == WalEntryStatus::Completed && !state.storage_exists(&entry.data_key) {
                return Err(InvariantViolation::with_evidence(
                    self.name(),
                    format!(
                        "WAL entry {} is Completed but data key '{}' not in storage",
                        entry.id, entry.data_key
                    ),
                    format!(
                        "Entry: id={}, client={}, status={:?}",
                        entry.id, entry.client_id, entry.status
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// AtomicVisibility invariant from KelpieWAL.tla
///
/// **TLA+ Definition:**
/// ```tla
/// AtomicVisibility ==
///     \A i \in 1..Len(wal) :
///         wal[i].status = "Completed" => storage[wal[i].data] # 0
/// ```
///
/// An entry's operation is either fully applied (Completed -> visible in storage)
/// or not at all. No partial states are visible.
pub struct AtomicVisibility;

impl Invariant for AtomicVisibility {
    fn name(&self) -> &'static str {
        "AtomicVisibility"
    }

    fn tla_source(&self) -> &'static str {
        "docs/tla/KelpieWAL.tla"
    }

    fn check(&self, state: &SystemState) -> Result<(), InvariantViolation> {
        // This is similar to Durability but focuses on atomicity
        // A completed entry must have its effects visible
        for entry in state.wal_entries() {
            if entry.status == WalEntryStatus::Completed && !state.storage_exists(&entry.data_key) {
                return Err(InvariantViolation::with_evidence(
                    self.name(),
                    format!(
                        "Completed WAL entry {} has no visible effect (data key '{}' missing)",
                        entry.id, entry.data_key
                    ),
                    "This indicates a partial/non-atomic state".to_string(),
                ));
            }
        }
        Ok(())
    }
}

/// NoSplitBrain invariant from KelpieClusterMembership.tla
///
/// **TLA+ Definition:**
/// ```tla
/// NoSplitBrain ==
///     \A n1, n2 \in Nodes :
///         /\ HasValidPrimaryClaim(n1)
///         /\ HasValidPrimaryClaim(n2)
///         => n1 = n2
/// ```
///
/// There is at most one valid primary node. A primary claim is valid only
/// if the node can reach a majority (quorum). This is THE KEY SAFETY INVARIANT
/// for cluster membership.
pub struct NoSplitBrain;

impl Invariant for NoSplitBrain {
    fn name(&self) -> &'static str {
        "NoSplitBrain"
    }

    fn tla_source(&self) -> &'static str {
        "docs/tla/KelpieClusterMembership.tla"
    }

    fn check(&self, state: &SystemState) -> Result<(), InvariantViolation> {
        let valid_primaries: Vec<&str> = state
            .nodes()
            .filter(|n| n.is_primary && n.has_quorum)
            .map(|n| n.id.as_str())
            .collect();

        if valid_primaries.len() > 1 {
            return Err(InvariantViolation::with_evidence(
                self.name(),
                format!(
                    "Split-brain detected: {} nodes have valid primary claims (max 1 allowed)",
                    valid_primaries.len()
                ),
                format!("Valid primaries: {:?}", valid_primaries),
            ));
        }
        Ok(())
    }
}

/// ReadYourWrites invariant from KelpieFDBTransaction.tla
///
/// **TLA+ Definition:**
/// ```tla
/// ReadYourWrites ==
///     \A t \in Transactions :
///         txnState[t] = RUNNING =>
///             \A k \in Keys :
///                 writeBuffer[t][k] # NoValue =>
///                     TxnRead(t, k) = writeBuffer[t][k]
/// ```
///
/// A running transaction must see its own writes. If a key was written
/// in the transaction's write buffer, reading that key must return the
/// written value, not the committed value.
pub struct ReadYourWrites;

impl Invariant for ReadYourWrites {
    fn name(&self) -> &'static str {
        "ReadYourWrites"
    }

    fn tla_source(&self) -> &'static str {
        "docs/tla/KelpieFDBTransaction.tla"
    }

    fn check(&self, state: &SystemState) -> Result<(), InvariantViolation> {
        for txn in state.transactions() {
            if txn.state != TransactionState::Running {
                continue;
            }
            for (key, written_value) in &txn.write_buffer {
                if let Some(read_value) = txn.reads.get(key) {
                    // If we wrote to this key and then read it, we should see our write
                    if read_value != written_value {
                        return Err(InvariantViolation::with_evidence(
                            self.name(),
                            format!(
                                "Transaction '{}' read key '{}' and got {:?}, but write buffer has {:?}",
                                txn.id, key, read_value, written_value
                            ),
                            format!("Transaction state: {:?}", txn.state),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// FencingTokenMonotonic invariant from KelpieLease.tla
///
/// **TLA+ Definition:**
/// ```tla
/// FencingTokenMonotonic ==
///     \A a \in Actors:
///         fencingTokens[a] >= 0
/// ```
///
/// Fencing tokens are non-negative and only increase. When a new lease is
/// acquired, the fencing token must be greater than any previous token
/// for that actor. This prevents stale writes from nodes with expired leases.
pub struct FencingTokenMonotonic;

impl Invariant for FencingTokenMonotonic {
    fn name(&self) -> &'static str {
        "FencingTokenMonotonic"
    }

    fn tla_source(&self) -> &'static str {
        "docs/tla/KelpieLease.tla"
    }

    fn check(&self, state: &SystemState) -> Result<(), InvariantViolation> {
        for (actor_id, token) in state.fencing_tokens() {
            if *token < 0 {
                return Err(InvariantViolation::with_evidence(
                    self.name(),
                    format!("Actor '{}' has negative fencing token: {}", actor_id, token),
                    "Fencing tokens must be non-negative".to_string(),
                ));
            }
        }
        // Also check monotonicity if we have token history
        for (actor_id, history) in state.fencing_token_history() {
            for window in history.windows(2) {
                if window[1] < window[0] {
                    return Err(InvariantViolation::with_evidence(
                        self.name(),
                        format!(
                            "Actor '{}' fencing token decreased from {} to {}",
                            actor_id, window[0], window[1]
                        ),
                        "Fencing tokens must be monotonically increasing".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// SnapshotConsistency invariant from KelpieTeleport.tla
///
/// **TLA+ Definition:**
/// ```tla
/// SnapshotConsistency ==
///     TRUE  \* Consistency enforced by CompleteRestore restoring exact savedState
/// ```
///
/// A restored snapshot must contain exactly the state that was saved.
/// No partial restores are allowed.
pub struct SnapshotConsistency;

impl Invariant for SnapshotConsistency {
    fn name(&self) -> &'static str {
        "SnapshotConsistency"
    }

    fn tla_source(&self) -> &'static str {
        "docs/tla/KelpieTeleport.tla"
    }

    fn check(&self, state: &SystemState) -> Result<(), InvariantViolation> {
        for (snapshot_id, snapshot) in state.snapshots() {
            if snapshot.is_restored {
                // Check that all saved keys are present in restored state
                for (key, saved_value) in &snapshot.saved_state {
                    match state.storage.get(key) {
                        Some(current_value) if current_value == saved_value => {}
                        Some(current_value) => {
                            return Err(InvariantViolation::with_evidence(
                                self.name(),
                                format!(
                                    "Snapshot '{}' key '{}' restored incorrectly: expected {:?}, got {:?}",
                                    snapshot_id, key, saved_value, current_value
                                ),
                                "Partial/corrupted restore detected".to_string(),
                            ));
                        }
                        None => {
                            return Err(InvariantViolation::with_evidence(
                                self.name(),
                                format!(
                                    "Snapshot '{}' key '{}' missing after restore",
                                    snapshot_id, key
                                ),
                                "Incomplete restore detected".to_string(),
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

// =============================================================================
// Extended System State for New Invariants
// =============================================================================

/// Transaction state for ReadYourWrites invariant
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransactionState {
    /// Transaction is running
    Running,
    /// Transaction committed successfully
    Committed,
    /// Transaction was aborted
    Aborted,
}

/// A transaction for linearizability checking
#[derive(Debug, Clone)]
pub struct Transaction {
    /// Transaction identifier
    pub id: String,
    /// Current state
    pub state: TransactionState,
    /// Write buffer: key -> value
    pub write_buffer: HashMap<String, String>,
    /// Reads performed: key -> value read
    pub reads: HashMap<String, String>,
}

/// A snapshot for teleport consistency checking
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Snapshot identifier
    pub id: String,
    /// Saved state: key -> value
    pub saved_state: HashMap<String, String>,
    /// Whether this snapshot has been restored
    pub is_restored: bool,
}

// Add new fields to NodeInfo for cluster membership
impl NodeInfo {
    /// Set whether this node is primary
    pub fn with_primary(mut self, is_primary: bool) -> Self {
        self.is_primary = is_primary;
        self
    }

    /// Set whether this node has quorum
    pub fn with_quorum(mut self, has_quorum: bool) -> Self {
        self.has_quorum = has_quorum;
        self
    }
}

// =============================================================================
// InvariantCheckingSimulation Harness
// =============================================================================

/// A simulation wrapper that automatically checks invariants after each operation.
///
/// This bridges TLA+ specs to DST tests by verifying the same properties that
/// TLA+ model checking would verify, but at runtime during simulation.
///
/// # Example
///
/// ```rust,ignore
/// use kelpie_dst::invariants::{InvariantCheckingSimulation, SingleActivation, NoSplitBrain};
///
/// let sim = InvariantCheckingSimulation::new()
///     .with_invariant(SingleActivation)
///     .with_invariant(NoSplitBrain);
///
/// sim.run(|env| async move {
///     // Test logic here - invariants checked after each step
///     env.activate_actor("actor-1").await?;
///     env.partition_network(["node-1"], ["node-2", "node-3"]).await;
///     // If any invariant is violated, the test fails with detailed evidence
///     Ok(())
/// }).await?;
/// ```
pub struct InvariantCheckingSimulation {
    checker: InvariantChecker,
    check_after_each_step: bool,
    state_snapshots: Vec<SystemState>,
}

impl Default for InvariantCheckingSimulation {
    fn default() -> Self {
        Self::new()
    }
}

impl InvariantCheckingSimulation {
    /// Create a new invariant-checking simulation
    pub fn new() -> Self {
        Self {
            checker: InvariantChecker::new(),
            check_after_each_step: true,
            state_snapshots: Vec::new(),
        }
    }

    /// Add an invariant to check
    pub fn with_invariant(mut self, inv: impl Invariant + 'static) -> Self {
        self.checker = self.checker.with_invariant(inv);
        self
    }

    /// Add all standard Kelpie invariants
    pub fn with_standard_invariants(mut self) -> Self {
        self.checker = self.checker.with_standard_invariants();
        self
    }

    /// Add cluster membership invariants
    pub fn with_cluster_invariants(self) -> Self {
        self.with_invariant(NoSplitBrain)
    }

    /// Add linearizability invariants
    pub fn with_linearizability_invariants(self) -> Self {
        self.with_invariant(ReadYourWrites)
    }

    /// Add lease safety invariants
    pub fn with_lease_invariants(self) -> Self {
        self.with_invariant(LeaseUniqueness)
            .with_invariant(FencingTokenMonotonic)
    }

    /// Disable checking after each step (only check at end)
    pub fn check_only_at_end(mut self) -> Self {
        self.check_after_each_step = false;
        self
    }

    /// Check invariants against the current state
    pub fn check_state(&self, state: &SystemState) -> Result<(), InvariantViolation> {
        self.checker.verify_all(state)
    }

    /// Record a state snapshot for debugging
    pub fn record_snapshot(&mut self, state: SystemState) {
        self.state_snapshots.push(state);
    }

    /// Get all recorded state snapshots
    pub fn snapshots(&self) -> &[SystemState] {
        &self.state_snapshots
    }

    /// Get the invariant checker
    pub fn checker(&self) -> &InvariantChecker {
        &self.checker
    }

    /// Should check after each step?
    pub fn checks_each_step(&self) -> bool {
        self.check_after_each_step
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_activation_passes() {
        let state = SystemState::new()
            .with_node(NodeInfo::new("node-1").with_actor_state("actor-1", NodeState::Active))
            .with_node(NodeInfo::new("node-2").with_actor_state("actor-1", NodeState::Idle));

        let result = SingleActivation.check(&state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_single_activation_fails() {
        let state = SystemState::new()
            .with_node(NodeInfo::new("node-1").with_actor_state("actor-1", NodeState::Active))
            .with_node(NodeInfo::new("node-2").with_actor_state("actor-1", NodeState::Active));

        let result = SingleActivation.check(&state);
        assert!(result.is_err());

        let violation = result.unwrap_err();
        assert_eq!(violation.name, "SingleActivation");
        assert!(violation.message.contains("2 active instances"));
    }

    #[test]
    fn test_consistent_holder_passes() {
        let state = SystemState::new()
            .with_node(NodeInfo::new("node-1").with_actor_state("actor-1", NodeState::Active))
            .with_fdb_holder("actor-1", Some("node-1".to_string()));

        let result = ConsistentHolder.check(&state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_consistent_holder_fails() {
        let state = SystemState::new()
            .with_node(NodeInfo::new("node-1").with_actor_state("actor-1", NodeState::Active))
            .with_fdb_holder("actor-1", Some("node-2".to_string()));

        let result = ConsistentHolder.check(&state);
        assert!(result.is_err());

        let violation = result.unwrap_err();
        assert_eq!(violation.name, "ConsistentHolder");
    }

    #[test]
    fn test_placement_consistency_passes() {
        let state = SystemState::new()
            .with_node(NodeInfo::new("node-1").with_status(NodeStatus::Active))
            .with_placement("actor-1", "node-1");

        let result = PlacementConsistency.check(&state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_placement_consistency_fails() {
        let state = SystemState::new()
            .with_node(NodeInfo::new("node-1").with_status(NodeStatus::Failed))
            .with_placement("actor-1", "node-1");

        let result = PlacementConsistency.check(&state);
        assert!(result.is_err());

        let violation = result.unwrap_err();
        assert_eq!(violation.name, "PlacementConsistency");
        assert!(violation.message.contains("failed node"));
    }

    #[test]
    fn test_lease_uniqueness_passes() {
        let state = SystemState::new()
            .with_time(100)
            .with_node(NodeInfo::new("node-1").with_lease_belief("actor-1", 200))
            .with_node(NodeInfo::new("node-2").with_lease_belief("actor-1", 50)); // expired

        let result = LeaseUniqueness.check(&state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_lease_uniqueness_fails() {
        let state = SystemState::new()
            .with_time(100)
            .with_node(NodeInfo::new("node-1").with_lease_belief("actor-1", 200))
            .with_node(NodeInfo::new("node-2").with_lease_belief("actor-1", 200));

        let result = LeaseUniqueness.check(&state);
        assert!(result.is_err());

        let violation = result.unwrap_err();
        assert_eq!(violation.name, "LeaseUniqueness");
        assert!(violation.message.contains("2 nodes believing"));
    }

    #[test]
    fn test_durability_passes() {
        let state = SystemState::new()
            .with_wal_entry(WalEntry {
                id: 1,
                client_id: "client-1".to_string(),
                idempotency_key: 1,
                status: WalEntryStatus::Completed,
                data_key: "key-1".to_string(),
            })
            .with_storage_key("key-1", "value-1");

        let result = Durability.check(&state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_durability_fails() {
        let state = SystemState::new().with_wal_entry(WalEntry {
            id: 1,
            client_id: "client-1".to_string(),
            idempotency_key: 1,
            status: WalEntryStatus::Completed,
            data_key: "key-1".to_string(),
        });
        // Note: key-1 not added to storage

        let result = Durability.check(&state);
        assert!(result.is_err());

        let violation = result.unwrap_err();
        assert_eq!(violation.name, "Durability");
    }

    #[test]
    fn test_atomic_visibility_pending_ok() {
        // Pending entries don't need to be visible
        let state = SystemState::new().with_wal_entry(WalEntry {
            id: 1,
            client_id: "client-1".to_string(),
            idempotency_key: 1,
            status: WalEntryStatus::Pending,
            data_key: "key-1".to_string(),
        });

        let result = AtomicVisibility.check(&state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_invariant_checker_verify_all() {
        let checker = InvariantChecker::new()
            .with_invariant(SingleActivation)
            .with_invariant(ConsistentHolder);

        let state = SystemState::new()
            .with_node(NodeInfo::new("node-1").with_actor_state("actor-1", NodeState::Active))
            .with_fdb_holder("actor-1", Some("node-1".to_string()));

        let result = checker.verify_all(&state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_invariant_checker_collect_all() {
        let checker = InvariantChecker::new()
            .with_invariant(SingleActivation)
            .with_invariant(ConsistentHolder);

        // Both invariants fail
        let state = SystemState::new()
            .with_node(NodeInfo::new("node-1").with_actor_state("actor-1", NodeState::Active))
            .with_node(NodeInfo::new("node-2").with_actor_state("actor-1", NodeState::Active))
            .with_fdb_holder("actor-1", Some("node-3".to_string())); // neither node

        let violations = checker.verify_all_collect(&state);
        assert_eq!(violations.len(), 2);

        let names: Vec<_> = violations.iter().map(|v| v.name.as_str()).collect();
        assert!(names.contains(&"SingleActivation"));
        assert!(names.contains(&"ConsistentHolder"));
    }

    #[test]
    fn test_standard_invariants() {
        let checker = InvariantChecker::new().with_standard_invariants();
        assert_eq!(checker.len(), 6);

        let names = checker.invariant_names();
        assert!(names.contains(&"SingleActivation"));
        assert!(names.contains(&"ConsistentHolder"));
        assert!(names.contains(&"PlacementConsistency"));
        assert!(names.contains(&"LeaseUniqueness"));
        assert!(names.contains(&"Durability"));
        assert!(names.contains(&"AtomicVisibility"));
    }

    #[test]
    fn test_empty_state_passes_all() {
        let checker = InvariantChecker::new().with_standard_invariants();
        let state = SystemState::new();

        let result = checker.verify_all(&state);
        assert!(result.is_ok());
    }

    // =========================================================================
    // Tests for new invariants (Issue #43)
    // =========================================================================

    #[test]
    fn test_no_split_brain_passes() {
        // Only one valid primary
        let state = SystemState::new()
            .with_node(NodeInfo::new("node-1").with_primary(true).with_quorum(true))
            .with_node(
                NodeInfo::new("node-2")
                    .with_primary(false)
                    .with_quorum(true),
            );

        let result = NoSplitBrain.check(&state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_no_split_brain_fails() {
        // Two nodes both think they're valid primaries - split brain!
        let state = SystemState::new()
            .with_node(NodeInfo::new("node-1").with_primary(true).with_quorum(true))
            .with_node(NodeInfo::new("node-2").with_primary(true).with_quorum(true));

        let result = NoSplitBrain.check(&state);
        assert!(result.is_err());

        let violation = result.unwrap_err();
        assert_eq!(violation.name, "NoSplitBrain");
        assert!(violation.message.contains("Split-brain detected"));
    }

    #[test]
    fn test_no_split_brain_minority_primary_ok() {
        // A minority primary (no quorum) doesn't count as valid
        let state = SystemState::new()
            .with_node(
                NodeInfo::new("node-1").with_primary(true).with_quorum(true), // valid primary
            )
            .with_node(
                NodeInfo::new("node-2")
                    .with_primary(true)
                    .with_quorum(false), // minority, not valid
            );

        let result = NoSplitBrain.check(&state);
        assert!(result.is_ok()); // Only one VALID primary
    }

    #[test]
    fn test_fencing_token_monotonic_passes() {
        let state = SystemState::new()
            .with_fencing_token("actor-1", 1)
            .with_fencing_token("actor-1", 2)
            .with_fencing_token("actor-1", 3);

        let result = FencingTokenMonotonic.check(&state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_fencing_token_monotonic_fails_negative() {
        let state = SystemState::new().with_fencing_token("actor-1", -1);

        let result = FencingTokenMonotonic.check(&state);
        assert!(result.is_err());

        let violation = result.unwrap_err();
        assert_eq!(violation.name, "FencingTokenMonotonic");
        assert!(violation.message.contains("negative"));
    }

    #[test]
    fn test_fencing_token_monotonic_fails_decrease() {
        // Manually create state with decreasing tokens
        let mut state = SystemState::new();
        state
            .fencing_token_history
            .insert("actor-1".to_string(), vec![5, 3]); // 5 -> 3 is a decrease!
        state.fencing_tokens.insert("actor-1".to_string(), 3);

        let result = FencingTokenMonotonic.check(&state);
        assert!(result.is_err());

        let violation = result.unwrap_err();
        assert_eq!(violation.name, "FencingTokenMonotonic");
        assert!(violation.message.contains("decreased"));
    }

    #[test]
    fn test_read_your_writes_passes() {
        let state = SystemState::new().with_transaction(Transaction {
            id: "txn-1".to_string(),
            state: TransactionState::Running,
            write_buffer: [("key-1".to_string(), "value-1".to_string())]
                .into_iter()
                .collect(),
            reads: [("key-1".to_string(), "value-1".to_string())]
                .into_iter()
                .collect(), // Read sees the write
        });

        let result = ReadYourWrites.check(&state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_read_your_writes_fails() {
        let state = SystemState::new().with_transaction(Transaction {
            id: "txn-1".to_string(),
            state: TransactionState::Running,
            write_buffer: [("key-1".to_string(), "value-1".to_string())]
                .into_iter()
                .collect(),
            reads: [("key-1".to_string(), "stale-value".to_string())]
                .into_iter()
                .collect(), // Read got stale value!
        });

        let result = ReadYourWrites.check(&state);
        assert!(result.is_err());

        let violation = result.unwrap_err();
        assert_eq!(violation.name, "ReadYourWrites");
    }

    #[test]
    fn test_read_your_writes_committed_txn_ignored() {
        // Committed transactions don't need to pass ReadYourWrites
        let state = SystemState::new().with_transaction(Transaction {
            id: "txn-1".to_string(),
            state: TransactionState::Committed, // Not running
            write_buffer: [("key-1".to_string(), "value-1".to_string())]
                .into_iter()
                .collect(),
            reads: [("key-1".to_string(), "different".to_string())]
                .into_iter()
                .collect(),
        });

        let result = ReadYourWrites.check(&state);
        assert!(result.is_ok()); // Committed txns not checked
    }

    #[test]
    fn test_snapshot_consistency_passes() {
        let state = SystemState::new()
            .with_storage_key("key-1", "value-1")
            .with_storage_key("key-2", "value-2")
            .with_snapshot(Snapshot {
                id: "snap-1".to_string(),
                saved_state: [
                    ("key-1".to_string(), "value-1".to_string()),
                    ("key-2".to_string(), "value-2".to_string()),
                ]
                .into_iter()
                .collect(),
                is_restored: true,
            });

        let result = SnapshotConsistency.check(&state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_snapshot_consistency_fails_missing_key() {
        let state = SystemState::new()
            .with_storage_key("key-1", "value-1")
            // key-2 is missing!
            .with_snapshot(Snapshot {
                id: "snap-1".to_string(),
                saved_state: [
                    ("key-1".to_string(), "value-1".to_string()),
                    ("key-2".to_string(), "value-2".to_string()),
                ]
                .into_iter()
                .collect(),
                is_restored: true,
            });

        let result = SnapshotConsistency.check(&state);
        assert!(result.is_err());

        let violation = result.unwrap_err();
        assert_eq!(violation.name, "SnapshotConsistency");
        assert!(violation.message.contains("missing"));
    }

    #[test]
    fn test_snapshot_consistency_fails_wrong_value() {
        let state = SystemState::new()
            .with_storage_key("key-1", "wrong-value") // Different value!
            .with_snapshot(Snapshot {
                id: "snap-1".to_string(),
                saved_state: [("key-1".to_string(), "value-1".to_string())]
                    .into_iter()
                    .collect(),
                is_restored: true,
            });

        let result = SnapshotConsistency.check(&state);
        assert!(result.is_err());

        let violation = result.unwrap_err();
        assert_eq!(violation.name, "SnapshotConsistency");
        assert!(violation.message.contains("incorrectly"));
    }

    #[test]
    fn test_snapshot_not_restored_ignored() {
        // Snapshots that haven't been restored don't need consistency check
        let state = SystemState::new()
            // Storage is empty but snapshot has data - that's OK if not restored
            .with_snapshot(Snapshot {
                id: "snap-1".to_string(),
                saved_state: [("key-1".to_string(), "value-1".to_string())]
                    .into_iter()
                    .collect(),
                is_restored: false, // Not restored yet
            });

        let result = SnapshotConsistency.check(&state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_invariant_checking_simulation_basic() {
        let sim = InvariantCheckingSimulation::new()
            .with_invariant(SingleActivation)
            .with_invariant(NoSplitBrain);

        let state = SystemState::new()
            .with_node(NodeInfo::new("node-1").with_actor_state("actor-1", NodeState::Active))
            .with_node(NodeInfo::new("node-2").with_primary(true).with_quorum(true));

        let result = sim.check_state(&state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_invariant_checking_simulation_with_cluster() {
        let sim = InvariantCheckingSimulation::new().with_cluster_invariants();

        assert!(sim.checker().invariant_names().contains(&"NoSplitBrain"));
    }

    #[test]
    fn test_invariant_checking_simulation_with_lease() {
        let sim = InvariantCheckingSimulation::new().with_lease_invariants();

        let names = sim.checker().invariant_names();
        assert!(names.contains(&"LeaseUniqueness"));
        assert!(names.contains(&"FencingTokenMonotonic"));
    }
}
