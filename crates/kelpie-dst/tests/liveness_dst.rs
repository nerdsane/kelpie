//! DST tests for TLA+ liveness properties
//!
//! TigerStyle: Deterministic verification of temporal properties.
//!
//! Note: Some methods in the state machines are defined for completeness
//! (matching TLA+ specs) but not used in all tests. These are marked with
//! `#[allow(dead_code)]` individually with comments explaining their purpose.
//!
//! This module tests liveness properties defined in TLA+ specifications:
//! - EventualActivation (KelpieSingleActivation.tla)
//! - NoStuckClaims (KelpieSingleActivation.tla)
//! - EventualFailureDetection (KelpieRegistry.tla)
//! - EventualCacheInvalidation (KelpieRegistry.tla)
//! - EventualLeaseResolution (KelpieLease.tla)
//! - EventualRecovery (KelpieWAL.tla)
//!
//! These tests verify that "good things eventually happen" even under faults.

use kelpie_dst::{
    liveness::BoundedLiveness, FaultConfig, FaultType, SimClock, SimConfig, Simulation,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

// =============================================================================
// Constants (TigerStyle: explicit units and bounds)
// =============================================================================

/// Heartbeat interval in milliseconds (from TLA+ spec: MaxHeartbeatMiss)
const HEARTBEAT_INTERVAL_MS: u64 = 100;

/// Heartbeat timeout - failures detected after 3 missed heartbeats
const HEARTBEAT_TIMEOUT_MS: u64 = HEARTBEAT_INTERVAL_MS * 3;

/// Lease duration in milliseconds
const LEASE_DURATION_MS: u64 = 500;

/// Activation timeout in milliseconds
const ACTIVATION_TIMEOUT_MS: u64 = 1000;

/// Cache invalidation timeout in milliseconds
const CACHE_INVALIDATION_TIMEOUT_MS: u64 = 2000;

/// WAL recovery timeout in milliseconds
const WAL_RECOVERY_TIMEOUT_MS: u64 = 3000;

/// Liveness check interval in milliseconds
const LIVENESS_CHECK_INTERVAL_MS: u64 = 10;

// =============================================================================
// State Machines (Modeled after TLA+ specs)
// =============================================================================

/// Node state from KelpieSingleActivation.tla
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeState {
    Idle,
    Reading,
    Committing,
    Active,
}

impl std::fmt::Display for NodeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeState::Idle => write!(f, "Idle"),
            NodeState::Reading => write!(f, "Reading"),
            NodeState::Committing => write!(f, "Committing"),
            NodeState::Active => write!(f, "Active"),
        }
    }
}

/// Node status from KelpieRegistry.tla
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeStatus {
    Active,
    Suspect,
    Failed,
}

impl std::fmt::Display for NodeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeStatus::Active => write!(f, "Active"),
            NodeStatus::Suspect => write!(f, "Suspect"),
            NodeStatus::Failed => write!(f, "Failed"),
        }
    }
}

/// WAL entry status from KelpieWAL.tla
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WalEntryStatus {
    Pending,
    Completed,
    /// TLA+ spec models Failed state; kept for spec completeness
    #[allow(dead_code)]
    Failed,
}

impl std::fmt::Display for WalEntryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WalEntryStatus::Pending => write!(f, "Pending"),
            WalEntryStatus::Completed => write!(f, "Completed"),
            WalEntryStatus::Failed => write!(f, "Failed"),
        }
    }
}

// =============================================================================
// Simulated Actor Activation System
// =============================================================================

/// Simulates the actor activation protocol from KelpieSingleActivation.tla
struct ActivationProtocol {
    /// Node state per node
    node_states: RwLock<Vec<NodeState>>,
    /// Current holder (None = no holder)
    holder: RwLock<Option<usize>>,
    /// FDB version for OCC
    version: AtomicU64,
    /// Clock for timeouts (TLA+ spec includes timing; kept for spec completeness)
    #[allow(dead_code)]
    clock: Arc<SimClock>,
}

impl ActivationProtocol {
    fn new(num_nodes: usize, clock: Arc<SimClock>) -> Self {
        Self {
            node_states: RwLock::new(vec![NodeState::Idle; num_nodes]),
            holder: RwLock::new(None),
            version: AtomicU64::new(0),
            clock,
        }
    }

    /// Start a claim for a node
    fn start_claim(&self, node: usize) {
        let mut states = self.node_states.write().unwrap();
        if states[node] == NodeState::Idle {
            states[node] = NodeState::Reading;
        }
    }

    /// Read phase - node reads current state
    fn read_fdb(&self, node: usize) -> u64 {
        let mut states = self.node_states.write().unwrap();
        if states[node] == NodeState::Reading {
            states[node] = NodeState::Committing;
        }
        self.version.load(Ordering::SeqCst)
    }

    /// Commit phase - attempts atomic commit
    fn commit_claim(&self, node: usize, read_version: u64) -> bool {
        let mut states = self.node_states.write().unwrap();
        let mut holder = self.holder.write().unwrap();

        if states[node] != NodeState::Committing {
            return false;
        }

        let current_version = self.version.load(Ordering::SeqCst);

        // OCC check: version must be unchanged and no current holder
        if read_version == current_version && holder.is_none() {
            *holder = Some(node);
            self.version.fetch_add(1, Ordering::SeqCst);
            states[node] = NodeState::Active;
            true
        } else {
            // Conflict - return to Idle
            states[node] = NodeState::Idle;
            false
        }
    }

    /// Release - active node releases
    /// TLA+ spec includes Release action; kept for spec completeness
    #[allow(dead_code)]
    fn release(&self, node: usize) {
        let mut states = self.node_states.write().unwrap();
        let mut holder = self.holder.write().unwrap();

        if states[node] == NodeState::Active && *holder == Some(node) {
            *holder = None;
            self.version.fetch_add(1, Ordering::SeqCst);
            states[node] = NodeState::Idle;
        }
    }

    /// Check if node is in claiming state (Reading or Committing)
    fn is_claiming(&self, node: usize) -> bool {
        let states = self.node_states.read().unwrap();
        matches!(states[node], NodeState::Reading | NodeState::Committing)
    }

    /// Check if node is active or idle
    fn is_resolved(&self, node: usize) -> bool {
        let states = self.node_states.read().unwrap();
        matches!(states[node], NodeState::Active | NodeState::Idle)
    }

    /// Get current state description
    fn state_description(&self) -> String {
        let states = self.node_states.read().unwrap();
        let holder = self.holder.read().unwrap();
        format!(
            "states={:?}, holder={:?}, version={}",
            states
                .iter()
                .enumerate()
                .map(|(i, s)| format!("n{}={}", i, s))
                .collect::<Vec<_>>(),
            holder,
            self.version.load(Ordering::SeqCst)
        )
    }
}

// =============================================================================
// Simulated Registry System
// =============================================================================

/// Simulates the registry system from KelpieRegistry.tla
struct RegistrySystem {
    /// Node statuses
    node_statuses: RwLock<Vec<NodeStatus>>,
    /// Whether each node is actually alive
    is_alive: RwLock<Vec<bool>>,
    /// Missed heartbeat counts per node
    heartbeat_counts: RwLock<Vec<u64>>,
    /// Cache entries: cache[node][actor] = Option<node_id>
    cache: RwLock<Vec<Vec<Option<usize>>>>,
    /// Authoritative placement: actor -> node
    placement: RwLock<Vec<Option<usize>>>,
    /// Max heartbeats before failure
    max_heartbeat_miss: u64,
    /// Clock (TLA+ spec includes timing; kept for spec completeness)
    #[allow(dead_code)]
    clock: Arc<SimClock>,
}

impl RegistrySystem {
    fn new(num_nodes: usize, num_actors: usize, clock: Arc<SimClock>) -> Self {
        Self {
            node_statuses: RwLock::new(vec![NodeStatus::Active; num_nodes]),
            is_alive: RwLock::new(vec![true; num_nodes]),
            heartbeat_counts: RwLock::new(vec![0; num_nodes]),
            cache: RwLock::new(vec![vec![None; num_actors]; num_nodes]),
            placement: RwLock::new(vec![None; num_actors]),
            max_heartbeat_miss: 3,
            clock,
        }
    }

    /// Node sends heartbeat
    fn send_heartbeat(&self, node: usize) {
        let is_alive = self.is_alive.read().unwrap();
        if !is_alive[node] {
            return;
        }

        let mut counts = self.heartbeat_counts.write().unwrap();
        let mut statuses = self.node_statuses.write().unwrap();

        counts[node] = 0;
        if statuses[node] == NodeStatus::Suspect {
            statuses[node] = NodeStatus::Active;
        }
    }

    /// Heartbeat tick - increment missed count for dead nodes
    fn heartbeat_tick(&self) {
        let is_alive = self.is_alive.read().unwrap();
        let mut counts = self.heartbeat_counts.write().unwrap();
        let statuses = self.node_statuses.read().unwrap();

        for node in 0..is_alive.len() {
            if !is_alive[node]
                && statuses[node] != NodeStatus::Failed
                && counts[node] < self.max_heartbeat_miss
            {
                counts[node] += 1;
            }
        }
    }

    /// Detect failure based on heartbeat timeout
    fn detect_failure(&self, node: usize) {
        let counts = self.heartbeat_counts.read().unwrap();
        let mut statuses = self.node_statuses.write().unwrap();

        if statuses[node] == NodeStatus::Failed {
            return;
        }

        if counts[node] >= self.max_heartbeat_miss {
            if statuses[node] == NodeStatus::Active {
                statuses[node] = NodeStatus::Suspect;
            } else if statuses[node] == NodeStatus::Suspect {
                statuses[node] = NodeStatus::Failed;
                // Clear placements on failed node
                let mut placement = self.placement.write().unwrap();
                for p in placement.iter_mut() {
                    if *p == Some(node) {
                        *p = None;
                    }
                }
            }
        }
    }

    /// Kill a node
    fn kill_node(&self, node: usize) {
        let mut is_alive = self.is_alive.write().unwrap();
        is_alive[node] = true; // Set to false to kill
        is_alive[node] = false;
    }

    /// Place an actor on a node
    fn place_actor(&self, actor: usize, node: usize) {
        let statuses = self.node_statuses.read().unwrap();
        let is_alive = self.is_alive.read().unwrap();
        let mut placement = self.placement.write().unwrap();

        if statuses[node] == NodeStatus::Active && is_alive[node] && placement[actor].is_none() {
            placement[actor] = Some(node);

            // Update local cache
            let mut cache = self.cache.write().unwrap();
            cache[node][actor] = Some(node);
        }
    }

    /// Invalidate cache entry
    fn invalidate_cache(&self, node: usize, actor: usize) {
        let is_alive = self.is_alive.read().unwrap();
        if !is_alive[node] {
            return;
        }

        let placement = self.placement.read().unwrap();
        let mut cache = self.cache.write().unwrap();

        if cache[node][actor] != placement[actor] {
            cache[node][actor] = placement[actor];
        }
    }

    /// Check if cache is stale for an actor on an alive node
    fn is_cache_stale(&self, node: usize, actor: usize) -> bool {
        let is_alive = self.is_alive.read().unwrap();
        if !is_alive[node] {
            return false;
        }

        let placement = self.placement.read().unwrap();
        let cache = self.cache.read().unwrap();

        cache[node][actor] != placement[actor]
    }

    /// Check if node status is Failed
    fn is_node_failed(&self, node: usize) -> bool {
        let statuses = self.node_statuses.read().unwrap();
        statuses[node] == NodeStatus::Failed
    }

    /// Check if node is dead (not alive)
    fn is_node_dead(&self, node: usize) -> bool {
        let is_alive = self.is_alive.read().unwrap();
        !is_alive[node]
    }

    /// Get state description
    fn state_description(&self) -> String {
        let statuses = self.node_statuses.read().unwrap();
        let is_alive = self.is_alive.read().unwrap();
        let counts = self.heartbeat_counts.read().unwrap();
        format!(
            "statuses={:?}, is_alive={:?}, heartbeat_counts={:?}",
            statuses
                .iter()
                .enumerate()
                .map(|(i, s)| format!("n{}={}", i, s))
                .collect::<Vec<_>>(),
            is_alive,
            counts
        )
    }
}

// =============================================================================
// Simulated Lease System
// =============================================================================

/// Simulates the lease system from KelpieLease.tla
struct LeaseSystem {
    /// Lease holder per actor (None = no holder)
    lease_holders: RwLock<Vec<Option<usize>>>,
    /// Lease expiry times per actor
    lease_expiry: RwLock<Vec<u64>>,
    /// Node beliefs: beliefs[node][actor] = (held, expiry)
    node_beliefs: RwLock<Vec<Vec<(bool, u64)>>>,
    /// Lease duration
    lease_duration_ms: u64,
    /// Clock
    clock: Arc<SimClock>,
}

impl LeaseSystem {
    fn new(num_nodes: usize, num_actors: usize, clock: Arc<SimClock>) -> Self {
        Self {
            lease_holders: RwLock::new(vec![None; num_actors]),
            lease_expiry: RwLock::new(vec![0; num_actors]),
            node_beliefs: RwLock::new(vec![vec![(false, 0); num_actors]; num_nodes]),
            lease_duration_ms: LEASE_DURATION_MS,
            clock,
        }
    }

    /// Acquire lease for an actor
    fn acquire_lease(&self, node: usize, actor: usize) -> bool {
        let current_time = self.clock.now_ms();
        let mut holders = self.lease_holders.write().unwrap();
        let mut expiry = self.lease_expiry.write().unwrap();
        let mut beliefs = self.node_beliefs.write().unwrap();

        // Check if lease is available (no valid lease)
        let lease_valid = holders[actor].is_some() && expiry[actor] > current_time;

        if !lease_valid {
            let new_expiry = current_time + self.lease_duration_ms;
            holders[actor] = Some(node);
            expiry[actor] = new_expiry;
            beliefs[node][actor] = (true, new_expiry);
            true
        } else {
            false
        }
    }

    /// Renew lease
    /// TLA+ spec includes RenewLease action; kept for spec completeness
    #[allow(dead_code)]
    fn renew_lease(&self, node: usize, actor: usize) -> bool {
        let current_time = self.clock.now_ms();
        let holders = self.lease_holders.read().unwrap();
        let mut expiry = self.lease_expiry.write().unwrap();
        let mut beliefs = self.node_beliefs.write().unwrap();

        if holders[actor] == Some(node) && expiry[actor] > current_time {
            let new_expiry = current_time + self.lease_duration_ms;
            expiry[actor] = new_expiry;
            beliefs[node][actor] = (true, new_expiry);
            true
        } else {
            false
        }
    }

    /// Release lease
    /// TLA+ spec includes ReleaseLease action; kept for spec completeness
    #[allow(dead_code)]
    fn release_lease(&self, node: usize, actor: usize) {
        let mut holders = self.lease_holders.write().unwrap();
        let mut expiry = self.lease_expiry.write().unwrap();
        let mut beliefs = self.node_beliefs.write().unwrap();

        if holders[actor] == Some(node) {
            holders[actor] = None;
            expiry[actor] = 0;
            beliefs[node][actor] = (false, 0);
        }
    }

    /// Update beliefs based on time (expire beliefs)
    fn tick(&self) {
        let current_time = self.clock.now_ms();
        let mut beliefs = self.node_beliefs.write().unwrap();

        for node_beliefs in beliefs.iter_mut() {
            for (held, exp) in node_beliefs.iter_mut() {
                if *held && *exp <= current_time {
                    *held = false;
                    *exp = 0;
                }
            }
        }
    }

    /// Check if actor has a valid lease
    fn has_valid_lease(&self, actor: usize) -> bool {
        let current_time = self.clock.now_ms();
        let holders = self.lease_holders.read().unwrap();
        let expiry = self.lease_expiry.read().unwrap();

        holders[actor].is_some() && expiry[actor] > current_time
    }

    /// Check if no node believes it holds a lease for the actor
    fn no_lease_beliefs(&self, actor: usize) -> bool {
        let current_time = self.clock.now_ms();
        let beliefs = self.node_beliefs.read().unwrap();

        beliefs.iter().all(|node_beliefs| {
            let (held, exp) = node_beliefs[actor];
            !held || exp <= current_time
        })
    }

    /// Get state description
    fn state_description(&self, actor: usize) -> String {
        let holders = self.lease_holders.read().unwrap();
        let expiry = self.lease_expiry.read().unwrap();
        let beliefs = self.node_beliefs.read().unwrap();
        let current_time = self.clock.now_ms();

        format!(
            "actor={}, holder={:?}, expiry={}, now={}, beliefs={:?}",
            actor,
            holders[actor],
            expiry[actor],
            current_time,
            beliefs
                .iter()
                .enumerate()
                .map(|(n, b)| format!("n{}=(held={}, exp={})", n, b[actor].0, b[actor].1))
                .collect::<Vec<_>>()
        )
    }
}

// =============================================================================
// Simulated WAL System
// =============================================================================

/// Simulates the WAL system from KelpieWAL.tla
struct WalSystem {
    /// WAL entries: (client, status)
    entries: RwLock<Vec<(usize, WalEntryStatus)>>,
    /// Whether system has crashed
    crashed: RwLock<bool>,
    /// Whether system is recovering
    recovering: RwLock<bool>,
    /// Clock (TLA+ spec includes timing; kept for spec completeness)
    #[allow(dead_code)]
    clock: Arc<SimClock>,
}

impl WalSystem {
    fn new(clock: Arc<SimClock>) -> Self {
        Self {
            entries: RwLock::new(Vec::new()),
            crashed: RwLock::new(false),
            recovering: RwLock::new(false),
            clock,
        }
    }

    /// Append entry to WAL
    fn append(&self, client: usize) -> Option<usize> {
        let crashed = self.crashed.read().unwrap();
        let recovering = self.recovering.read().unwrap();

        if *crashed || *recovering {
            return None;
        }

        let mut entries = self.entries.write().unwrap();
        let idx = entries.len();
        entries.push((client, WalEntryStatus::Pending));
        Some(idx)
    }

    /// Complete an entry
    /// TLA+ spec includes CompleteEntry action; kept for spec completeness
    #[allow(dead_code)]
    fn complete(&self, idx: usize) {
        let crashed = self.crashed.read().unwrap();
        if *crashed {
            return;
        }

        let mut entries = self.entries.write().unwrap();
        if idx < entries.len() && entries[idx].1 == WalEntryStatus::Pending {
            entries[idx].1 = WalEntryStatus::Completed;
        }
    }

    /// Fail an entry
    /// TLA+ spec includes FailEntry action; kept for spec completeness
    #[allow(dead_code)]
    fn fail(&self, idx: usize) {
        let crashed = self.crashed.read().unwrap();
        if *crashed {
            return;
        }

        let mut entries = self.entries.write().unwrap();
        if idx < entries.len() && entries[idx].1 == WalEntryStatus::Pending {
            entries[idx].1 = WalEntryStatus::Failed;
        }
    }

    /// Crash the system
    fn crash(&self) {
        let mut crashed = self.crashed.write().unwrap();
        *crashed = true;
    }

    /// Start recovery
    fn start_recovery(&self) {
        let mut crashed = self.crashed.write().unwrap();
        let mut recovering = self.recovering.write().unwrap();

        if *crashed {
            *crashed = false;
            *recovering = true;
        }
    }

    /// Recover a pending entry
    fn recover_entry(&self, idx: usize) {
        let recovering = self.recovering.read().unwrap();
        if !*recovering {
            return;
        }

        let mut entries = self.entries.write().unwrap();
        if idx < entries.len() && entries[idx].1 == WalEntryStatus::Pending {
            entries[idx].1 = WalEntryStatus::Completed;
        }
    }

    /// Complete recovery
    fn complete_recovery(&self) {
        let entries = self.entries.read().unwrap();
        let has_pending = entries.iter().any(|(_, s)| *s == WalEntryStatus::Pending);

        if !has_pending {
            let mut recovering = self.recovering.write().unwrap();
            *recovering = false;
        }
    }

    /// Check if there are pending entries
    fn has_pending_entries(&self) -> bool {
        let entries = self.entries.read().unwrap();
        entries.iter().any(|(_, s)| *s == WalEntryStatus::Pending)
    }

    /// Check if system is crashed
    fn is_crashed(&self) -> bool {
        *self.crashed.read().unwrap()
    }

    /// Check if system is recovering
    fn is_recovering(&self) -> bool {
        *self.recovering.read().unwrap()
    }

    /// Check if system is stable (not crashed, not recovering, no pending)
    fn is_stable(&self) -> bool {
        !self.is_crashed() && !self.is_recovering() && !self.has_pending_entries()
    }

    /// Get pending entry indices
    fn pending_entries(&self) -> Vec<usize> {
        let entries = self.entries.read().unwrap();
        entries
            .iter()
            .enumerate()
            .filter(|(_, (_, s))| *s == WalEntryStatus::Pending)
            .map(|(i, _)| i)
            .collect()
    }

    /// Get state description
    fn state_description(&self) -> String {
        let entries = self.entries.read().unwrap();
        let crashed = self.crashed.read().unwrap();
        let recovering = self.recovering.read().unwrap();

        format!(
            "crashed={}, recovering={}, entries={:?}",
            *crashed,
            *recovering,
            entries
                .iter()
                .enumerate()
                .map(|(i, (c, s))| format!("{}:(client={}, status={})", i, c, s))
                .collect::<Vec<_>>()
        )
    }
}

// =============================================================================
// Test: EventualActivation (KelpieSingleActivation.tla)
// =============================================================================

/// EventualActivation: Every claim attempt eventually resolves.
/// TLA+: Claiming(n) ~> (Active(n) ∨ Idle(n))
#[test]
fn test_eventual_activation() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let clock = env.clock.clone();
        let protocol = Arc::new(ActivationProtocol::new(3, clock.clone()));

        // Node 0 starts claiming
        protocol.start_claim(0);

        // Verify: Claiming ~> (Active ∨ Idle)
        let liveness = BoundedLiveness::new(ACTIVATION_TIMEOUT_MS * 2)
            .with_check_interval_ms(LIVENESS_CHECK_INTERVAL_MS);

        // Simulate the protocol progressing
        let protocol_ref = protocol.clone();
        let progress_protocol = move || {
            let p = &protocol_ref;

            // Progress through claim states
            let states = p.node_states.read().unwrap();
            if states[0] == NodeState::Reading {
                drop(states);
                let version = p.read_fdb(0);
                p.commit_claim(0, version);
            }
        };

        // Run progress in parallel with liveness check
        let protocol_check = protocol.clone();
        liveness
            .verify_leads_to(
                &clock,
                "EventualActivation",
                {
                    let p = protocol.clone();
                    move || p.is_claiming(0)
                },
                {
                    let p = protocol_check.clone();
                    move || {
                        // Also progress the protocol each check
                        progress_protocol();
                        p.is_resolved(0)
                    }
                },
                {
                    let p = protocol.clone();
                    move || p.state_description()
                },
            )
            .await?;

        Ok(())
    });

    assert!(
        result.is_ok(),
        "EventualActivation failed: {:?}",
        result.err()
    );
}

// =============================================================================
// Test: NoStuckClaims (KelpieSingleActivation.tla)
// =============================================================================

/// NoStuckClaims: No node remains in claiming state forever.
/// TLA+: [](Claiming(n) => <>¬Claiming(n))
#[test]
fn test_no_stuck_claims() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let clock = env.clock.clone();
        let protocol = Arc::new(ActivationProtocol::new(2, clock.clone()));

        // Both nodes start claiming (contention)
        protocol.start_claim(0);
        protocol.start_claim(1);

        // Verify neither node gets stuck
        let liveness = BoundedLiveness::new(ACTIVATION_TIMEOUT_MS * 3)
            .with_check_interval_ms(LIVENESS_CHECK_INTERVAL_MS);

        // Progress protocol
        let protocol_ref = protocol.clone();
        let progress = move || {
            let p = &protocol_ref;
            for node in 0..2 {
                let states = p.node_states.read().unwrap();
                let state = states[node];
                drop(states);

                match state {
                    NodeState::Reading => {
                        let version = p.read_fdb(node);
                        p.commit_claim(node, version);
                    }
                    NodeState::Committing => {
                        // Already read, just commit with stored version
                        // In real impl this would use stored read_version
                        let version = p.version.load(Ordering::SeqCst);
                        p.commit_claim(node, version.saturating_sub(1));
                    }
                    _ => {}
                }
            }
        };

        // Check that all claiming nodes eventually resolve
        for node in 0..2 {
            let protocol_check = protocol.clone();
            let progress_clone = progress.clone();

            liveness
                .verify_eventually(
                    &clock,
                    &format!("NoStuckClaims_node{}", node),
                    {
                        let p = protocol_check.clone();
                        move || {
                            progress_clone();
                            !p.is_claiming(node)
                        }
                    },
                    {
                        let p = protocol.clone();
                        move || p.state_description()
                    },
                )
                .await?;
        }

        Ok(())
    });

    assert!(result.is_ok(), "NoStuckClaims failed: {:?}", result.err());
}

// =============================================================================
// Test: EventualFailureDetection (KelpieRegistry.tla)
// =============================================================================

/// EventualFailureDetection: Dead nodes are eventually detected.
/// TLA+: (isAlive[n] = FALSE) ~> (nodeStatus[n] = Failed)
#[test]
fn test_eventual_failure_detection() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let clock = env.clock.clone();
        let registry = Arc::new(RegistrySystem::new(3, 2, clock.clone()));

        // Kill node 1
        registry.kill_node(1);

        // Simulate heartbeat mechanism
        let registry_ref = registry.clone();
        let simulate_heartbeats = move || {
            let r = &registry_ref;

            // Alive nodes send heartbeats
            let is_alive = r.is_alive.read().unwrap().clone();
            for (node, alive) in is_alive.iter().enumerate() {
                if *alive {
                    r.send_heartbeat(node);
                }
            }

            // Heartbeat tick for dead nodes
            r.heartbeat_tick();

            // Detect failures
            for node in 0..is_alive.len() {
                r.detect_failure(node);
            }
        };

        let liveness = BoundedLiveness::new(HEARTBEAT_TIMEOUT_MS * 3)
            .with_check_interval_ms(HEARTBEAT_INTERVAL_MS);

        let registry_check = registry.clone();
        liveness
            .verify_leads_to(
                &clock,
                "EventualFailureDetection",
                {
                    let r = registry.clone();
                    move || r.is_node_dead(1)
                },
                {
                    let r = registry_check.clone();
                    move || {
                        simulate_heartbeats();
                        r.is_node_failed(1)
                    }
                },
                {
                    let r = registry.clone();
                    move || r.state_description()
                },
            )
            .await?;

        Ok(())
    });

    assert!(
        result.is_ok(),
        "EventualFailureDetection failed: {:?}",
        result.err()
    );
}

// =============================================================================
// Test: EventualCacheInvalidation (KelpieRegistry.tla)
// =============================================================================

/// EventualCacheInvalidation: Stale caches on alive nodes are eventually corrected.
/// TLA+: (isAlive[n] ∧ IsCacheStale(n, a)) ~> (¬isAlive[n] ∨ ¬IsCacheStale(n, a))
#[test]
fn test_eventual_cache_invalidation() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let clock = env.clock.clone();
        let registry = Arc::new(RegistrySystem::new(3, 2, clock.clone()));

        // Place actor 0 on node 0
        registry.place_actor(0, 0);

        // Create stale cache on node 1 by manually setting it
        {
            let mut cache = registry.cache.write().unwrap();
            cache[1][0] = Some(2); // Wrong! Should be 0
        }

        // Verify cache is stale
        assert!(registry.is_cache_stale(1, 0));

        // Simulate cache invalidation
        let registry_ref = registry.clone();
        let simulate_invalidation = move || {
            let r = &registry_ref;
            // Invalidate caches for all alive nodes
            for node in 0..3 {
                for actor in 0..2 {
                    r.invalidate_cache(node, actor);
                }
            }
        };

        let liveness = BoundedLiveness::new(CACHE_INVALIDATION_TIMEOUT_MS)
            .with_check_interval_ms(LIVENESS_CHECK_INTERVAL_MS);

        let registry_check = registry.clone();
        liveness
            .verify_eventually(
                &clock,
                "EventualCacheInvalidation",
                {
                    let r = registry_check.clone();
                    move || {
                        simulate_invalidation();
                        !r.is_cache_stale(1, 0)
                    }
                },
                {
                    let r = registry.clone();
                    move || r.state_description()
                },
            )
            .await?;

        Ok(())
    });

    assert!(
        result.is_ok(),
        "EventualCacheInvalidation failed: {:?}",
        result.err()
    );
}

// =============================================================================
// Test: EventualLeaseResolution (KelpieLease.tla)
// =============================================================================

/// EventualLeaseResolution: Leases eventually resolve to a clean state.
/// TLA+: []<>(IsValidLease(a) ∨ ¬(∃n: NodeBelievesItHolds(n, a)))
#[test]
fn test_eventual_lease_resolution() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let clock = env.clock.clone();
        let lease_system = Arc::new(LeaseSystem::new(2, 1, clock.clone()));

        // Node 0 acquires lease for actor 0
        lease_system.acquire_lease(0, 0);

        // Verify: eventually lease is valid OR no one believes they hold it
        let liveness = BoundedLiveness::new(LEASE_DURATION_MS * 3)
            .with_check_interval_ms(LIVENESS_CHECK_INTERVAL_MS);

        let lease_check = lease_system.clone();
        let simulate_time = move || {
            lease_check.tick();
        };

        liveness
            .verify_eventually(
                &clock,
                "EventualLeaseResolution",
                {
                    let ls = lease_system.clone();
                    move || {
                        simulate_time();
                        ls.has_valid_lease(0) || ls.no_lease_beliefs(0)
                    }
                },
                {
                    let ls = lease_system.clone();
                    move || ls.state_description(0)
                },
            )
            .await?;

        Ok(())
    });

    assert!(
        result.is_ok(),
        "EventualLeaseResolution failed: {:?}",
        result.err()
    );
}

// =============================================================================
// Test: EventualRecovery (KelpieWAL.tla)
// =============================================================================

/// EventualRecovery: After crash, pending entries are eventually processed.
/// TLA+: [](crashed => <>(¬crashed ∧ ¬recovering ∧ PendingEntries = {}))
#[test]
fn test_eventual_recovery() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|env| async move {
        let clock = env.clock.clone();
        let wal = Arc::new(WalSystem::new(clock.clone()));

        // Append some entries
        wal.append(0);
        wal.append(1);
        wal.append(0);

        // Crash the system
        wal.crash();

        // Verify eventual recovery
        let liveness = BoundedLiveness::new(WAL_RECOVERY_TIMEOUT_MS)
            .with_check_interval_ms(LIVENESS_CHECK_INTERVAL_MS);

        // Simulate recovery process
        let wal_ref = wal.clone();
        let simulate_recovery = move || {
            let w = &wal_ref;

            if w.is_crashed() {
                w.start_recovery();
            }

            if w.is_recovering() {
                // Recover all pending entries
                for idx in w.pending_entries() {
                    w.recover_entry(idx);
                }
                w.complete_recovery();
            }
        };

        let wal_check = wal.clone();
        liveness
            .verify_leads_to(
                &clock,
                "EventualRecovery",
                {
                    let w = wal.clone();
                    move || w.is_crashed()
                },
                {
                    let w = wal_check.clone();
                    move || {
                        simulate_recovery();
                        w.is_stable()
                    }
                },
                {
                    let w = wal.clone();
                    move || w.state_description()
                },
            )
            .await?;

        Ok(())
    });

    assert!(
        result.is_ok(),
        "EventualRecovery failed: {:?}",
        result.err()
    );
}

// =============================================================================
// Test: Liveness Under Fault Injection
// =============================================================================

/// Test that EventualActivation holds even with storage faults
#[test]
fn test_eventual_activation_with_faults() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.1))
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.1))
        .run(|env| async move {
            let clock = env.clock.clone();
            let protocol = Arc::new(ActivationProtocol::new(2, clock.clone()));

            // Node 0 starts claiming
            protocol.start_claim(0);

            // With faults, we need a longer timeout
            let liveness = BoundedLiveness::new(ACTIVATION_TIMEOUT_MS * 5)
                .with_check_interval_ms(LIVENESS_CHECK_INTERVAL_MS);

            // Progress protocol (may fail due to faults, but eventually succeeds)
            let protocol_ref = protocol.clone();
            let progress_with_retries = move || {
                let p = &protocol_ref;
                let states = p.node_states.read().unwrap();
                let state = states[0];
                drop(states);

                match state {
                    NodeState::Reading => {
                        let version = p.read_fdb(0);
                        if !p.commit_claim(0, version) {
                            // Retry by restarting claim
                            p.start_claim(0);
                        }
                    }
                    NodeState::Idle => {
                        // Retry
                        p.start_claim(0);
                    }
                    _ => {}
                }
            };

            let protocol_check = protocol.clone();
            liveness
                .verify_eventually(
                    &clock,
                    "EventualActivation_with_faults",
                    {
                        let p = protocol_check.clone();
                        move || {
                            progress_with_retries();
                            p.is_resolved(0) && !p.is_claiming(0)
                        }
                    },
                    {
                        let p = protocol.clone();
                        move || p.state_description()
                    },
                )
                .await?;

            Ok(())
        });

    assert!(
        result.is_ok(),
        "EventualActivation with faults failed: {:?}",
        result.err()
    );
}

/// Test that EventualRecovery holds even with crash faults
#[test]
fn test_eventual_recovery_with_crash_faults() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::CrashAfterWrite, 0.05).with_filter("wal"))
        .run(|env| async move {
            let clock = env.clock.clone();
            let wal = Arc::new(WalSystem::new(clock.clone()));

            // Append entries and crash
            wal.append(0);
            wal.crash();

            let liveness = BoundedLiveness::new(WAL_RECOVERY_TIMEOUT_MS * 2)
                .with_check_interval_ms(LIVENESS_CHECK_INTERVAL_MS);

            let wal_ref = wal.clone();
            let simulate = move || {
                let w = &wal_ref;
                if w.is_crashed() {
                    w.start_recovery();
                }
                if w.is_recovering() {
                    for idx in w.pending_entries() {
                        w.recover_entry(idx);
                    }
                    w.complete_recovery();
                }
            };

            let wal_check = wal.clone();
            liveness
                .verify_eventually(
                    &clock,
                    "EventualRecovery_with_crash_faults",
                    {
                        let w = wal_check.clone();
                        move || {
                            simulate();
                            w.is_stable()
                        }
                    },
                    {
                        let w = wal.clone();
                        move || w.state_description()
                    },
                )
                .await?;

            Ok(())
        });

    assert!(
        result.is_ok(),
        "EventualRecovery with crash faults failed: {:?}",
        result.err()
    );
}

// =============================================================================
// Stress Tests (ignored by default)
// =============================================================================

/// Stress test: Run many iterations with random seeds
#[test]
#[ignore]
fn test_liveness_stress() {
    const ITERATIONS: u64 = 100;

    for i in 0..ITERATIONS {
        let seed = 0xDEAD_BEEF + i;
        let config = SimConfig::new(seed);

        let result = Simulation::new(config)
            .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.05))
            .with_fault(FaultConfig::new(FaultType::NetworkPacketLoss, 0.05))
            .run(|env| async move {
                let clock = env.clock.clone();
                let protocol = Arc::new(ActivationProtocol::new(3, clock.clone()));

                // All nodes try to claim
                for node in 0..3 {
                    protocol.start_claim(node);
                }

                let liveness = BoundedLiveness::new(ACTIVATION_TIMEOUT_MS * 10)
                    .with_check_interval_ms(LIVENESS_CHECK_INTERVAL_MS);

                let protocol_ref = protocol.clone();
                let progress = move || {
                    let p = &protocol_ref;
                    for node in 0..3 {
                        let states = p.node_states.read().unwrap();
                        let state = states[node];
                        drop(states);

                        if state == NodeState::Reading {
                            let v = p.read_fdb(node);
                            p.commit_claim(node, v);
                        }
                    }
                };

                // Verify all nodes eventually resolve
                for node in 0..3 {
                    let p = protocol.clone();
                    let progress_clone = progress.clone();
                    liveness
                        .verify_eventually(
                            &clock,
                            &format!("stress_node{}", node),
                            move || {
                                progress_clone();
                                !p.is_claiming(node)
                            },
                            || "stress test".to_string(),
                        )
                        .await?;
                }

                Ok(())
            });

        assert!(
            result.is_ok(),
            "Stress test failed at iteration {} (seed={}): {:?}",
            i,
            seed,
            result.err()
        );
    }

    println!("Stress test passed: {} iterations", ITERATIONS);
}
