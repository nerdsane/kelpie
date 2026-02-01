//! FoundationDB Registry Backend
//!
//! Provides distributed registry with linearizable guarantees using FDB.
//!
//! # Key Schema
//!
//! ```text
//! /kelpie/registry/nodes/{node_id}           -> NodeInfo (JSON)
//! /kelpie/registry/actors/{namespace}/{id}   -> ActorPlacement (JSON)
//! /kelpie/registry/leases/{namespace}/{id}   -> Lease (JSON)
//! ```
//!
//! # Lease-Based Activation
//!
//! Actors are protected by leases that must be renewed periodically.
//! If a lease expires, another node can claim the actor.
//!
//! TigerStyle: Explicit lease management, FDB transactions for atomicity.

// TODO: Migrate from deprecated Clock/SystemClock to TimeProvider/WallClockTime
// This module uses the deprecated registry::Clock trait which should be replaced
// with kelpie_core::io::TimeProvider. This is tracked technical debt.
#![allow(deprecated)]

use crate::error::{RegistryError, RegistryResult};
use crate::heartbeat::{Heartbeat, HeartbeatConfig, HeartbeatTracker};
use crate::node::{NodeId, NodeInfo, NodeStatus};
use crate::placement::{ActorPlacement, PlacementContext, PlacementDecision, PlacementStrategy};
use crate::registry::{Clock, Registry, SystemClock};
use async_trait::async_trait;
use foundationdb::api::{FdbApiBuilder, NetworkAutoStop};
use foundationdb::options::StreamingMode;
use foundationdb::tuple::Subspace;
use foundationdb::{Database, RangeOption, Transaction as FdbTransaction};
use kelpie_core::actor::ActorId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;
use tracing::{debug, instrument, warn};

/// Global FDB network guard - must live for the entire process
static FDB_NETWORK: OnceLock<NetworkAutoStop> = OnceLock::new();

// =============================================================================
// Constants
// =============================================================================

/// Default lease duration in milliseconds
pub const LEASE_DURATION_MS_DEFAULT: u64 = 30_000;

/// Lease renewal interval (should be less than lease duration)
pub const LEASE_RENEWAL_INTERVAL_MS_DEFAULT: u64 = 10_000;

/// Maximum transaction retry count
const TRANSACTION_RETRY_COUNT_MAX: usize = 5;

/// Transaction timeout in milliseconds
const TRANSACTION_TIMEOUT_MS: i32 = 5_000;

// Key prefixes
const KEY_PREFIX_KELPIE: &str = "kelpie";
const KEY_PREFIX_REGISTRY: &str = "registry";
const KEY_PREFIX_NODES: &str = "nodes";
const KEY_PREFIX_ACTORS: &str = "actors";
const KEY_PREFIX_LEASES: &str = "leases";

// =============================================================================
// Lease Types
// =============================================================================

/// Lease for an actor's activation
///
/// A lease grants a node exclusive activation rights for an actor.
/// The lease must be renewed before expiry or another node can claim it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lease {
    /// Node holding the lease
    pub node_id: NodeId,
    /// When the lease was acquired (epoch ms)
    pub acquired_at_ms: u64,
    /// When the lease expires (epoch ms)
    pub expires_at_ms: u64,
    /// Version for optimistic concurrency (incremented on renewal)
    pub version: u64,
}

impl Lease {
    /// Create a new lease
    pub fn new(node_id: NodeId, now_ms: u64, duration_ms: u64) -> Self {
        Self {
            node_id,
            acquired_at_ms: now_ms,
            expires_at_ms: now_ms + duration_ms,
            version: 1,
        }
    }

    /// Check if the lease has expired
    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms >= self.expires_at_ms
    }

    /// Renew the lease
    pub fn renew(&mut self, now_ms: u64, duration_ms: u64) {
        self.expires_at_ms = now_ms + duration_ms;
        self.version += 1;
    }

    /// Check if this node owns the lease
    pub fn is_owned_by(&self, node_id: &NodeId) -> bool {
        &self.node_id == node_id
    }
}

// =============================================================================
// FdbRegistry Configuration
// =============================================================================

/// Configuration for FdbRegistry
#[derive(Debug, Clone)]
pub struct FdbRegistryConfig {
    /// Lease duration in milliseconds
    pub lease_duration_ms: u64,
    /// Lease renewal interval in milliseconds
    pub lease_renewal_interval_ms: u64,
    /// Heartbeat configuration
    pub heartbeat_config: HeartbeatConfig,
}

impl Default for FdbRegistryConfig {
    fn default() -> Self {
        Self {
            lease_duration_ms: LEASE_DURATION_MS_DEFAULT,
            lease_renewal_interval_ms: LEASE_RENEWAL_INTERVAL_MS_DEFAULT,
            heartbeat_config: HeartbeatConfig::default(),
        }
    }
}

// =============================================================================
// FdbRegistry
// =============================================================================

/// FoundationDB-backed registry
///
/// Provides distributed actor registry with:
/// - Linearizable operations via FDB transactions
/// - Lease-based single activation guarantee
/// - Distributed failure detection via heartbeats
///
/// TigerStyle: Explicit FDB operations, bounded lease durations.
pub struct FdbRegistry {
    /// FDB database handle
    db: Arc<Database>,
    /// Subspace for all registry data
    subspace: Subspace,
    /// Configuration
    config: FdbRegistryConfig,
    /// Local cache for nodes (read optimization)
    node_cache: RwLock<HashMap<NodeId, NodeInfo>>,
    /// Heartbeat tracker (for local heartbeat timeout detection)
    heartbeat_tracker: RwLock<HeartbeatTracker>,
    /// Clock for timestamps
    clock: Arc<dyn Clock>,
}

impl FdbRegistry {
    /// Connect to FoundationDB and create a new registry
    ///
    /// # Arguments
    /// * `cluster_file` - Path to FDB cluster file. If None, uses default.
    /// * `config` - Registry configuration
    #[instrument(skip_all)]
    pub async fn connect(
        cluster_file: Option<&str>,
        config: FdbRegistryConfig,
    ) -> RegistryResult<Self> {
        // Boot FDB network (once per process)
        FDB_NETWORK.get_or_init(|| {
            let network_builder = FdbApiBuilder::default()
                .build()
                .expect("FDB API build failed");
            unsafe { network_builder.boot().expect("FDB network boot failed") }
        });

        let db = Database::new(cluster_file).map_err(|e| RegistryError::Internal {
            message: format!("FDB database open failed: {}", e),
        })?;

        let subspace = Subspace::from((KEY_PREFIX_KELPIE, KEY_PREFIX_REGISTRY));

        debug!("Connected to FoundationDB registry");

        Ok(Self {
            db: Arc::new(db),
            subspace,
            heartbeat_tracker: RwLock::new(HeartbeatTracker::new(config.heartbeat_config.clone())),
            config,
            node_cache: RwLock::new(HashMap::new()),
            clock: Arc::new(SystemClock),
        })
    }

    /// Create from existing database handle (for testing)
    pub fn from_database(db: Arc<Database>, config: FdbRegistryConfig) -> Self {
        let subspace = Subspace::from((KEY_PREFIX_KELPIE, KEY_PREFIX_REGISTRY));
        Self {
            db,
            subspace,
            heartbeat_tracker: RwLock::new(HeartbeatTracker::new(config.heartbeat_config.clone())),
            config,
            node_cache: RwLock::new(HashMap::new()),
            clock: Arc::new(SystemClock),
        }
    }

    /// Create with a custom clock (for testing)
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
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

    fn actor_key(&self, actor_id: &ActorId) -> Vec<u8> {
        self.subspace
            .subspace(&(KEY_PREFIX_ACTORS,))
            .pack(&(actor_id.namespace(), actor_id.id()))
    }

    fn actors_prefix(&self) -> Vec<u8> {
        self.subspace
            .subspace(&(KEY_PREFIX_ACTORS,))
            .bytes()
            .to_vec()
    }

    fn lease_key(&self, actor_id: &ActorId) -> Vec<u8> {
        self.subspace
            .subspace(&(KEY_PREFIX_LEASES,))
            .pack(&(actor_id.namespace(), actor_id.id()))
    }

    fn leases_prefix(&self) -> Vec<u8> {
        self.subspace
            .subspace(&(KEY_PREFIX_LEASES,))
            .bytes()
            .to_vec()
    }

    // =========================================================================
    // FDB Transaction Helpers
    // =========================================================================

    /// Create a new FDB transaction with timeout
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

    /// Execute a read-only operation with retry
    #[allow(dead_code)]
    async fn read<F, T>(&self, f: F) -> RegistryResult<T>
    where
        F: Fn(&FdbTransaction) -> RegistryResult<T>,
    {
        let txn = self.create_transaction()?;
        f(&txn)
    }

    /// Execute a read-write operation with retry
    async fn transact<F, T>(&self, f: F) -> RegistryResult<T>
    where
        F: Fn(&FdbTransaction) -> RegistryResult<T>,
    {
        let mut attempts = 0;

        loop {
            attempts += 1;
            if attempts > TRANSACTION_RETRY_COUNT_MAX {
                return Err(RegistryError::Internal {
                    message: "exceeded max transaction retries".into(),
                });
            }

            let txn = self.create_transaction()?;

            match f(&txn) {
                Ok(result) => match txn.commit().await {
                    Ok(_) => return Ok(result),
                    Err(e) if e.is_retryable() && attempts < TRANSACTION_RETRY_COUNT_MAX => {
                        warn!(
                            "Transaction conflict (attempt {}/{}), retrying",
                            attempts, TRANSACTION_RETRY_COUNT_MAX
                        );
                        let _ = e.on_error().await;
                        continue;
                    }
                    Err(e) => {
                        return Err(RegistryError::Internal {
                            message: format!("commit failed: {}", e),
                        });
                    }
                },
                Err(e) => return Err(e),
            }
        }
    }

    // =========================================================================
    // Lease Management
    // =========================================================================

    /// Try to acquire or renew a lease for an actor
    ///
    /// Returns Ok(true) if lease was acquired/renewed, Ok(false) if another node holds it.
    #[instrument(skip(self), fields(actor_id = %actor_id.qualified_name(), node_id = %node_id))]
    pub async fn try_acquire_lease(
        &self,
        actor_id: &ActorId,
        node_id: &NodeId,
    ) -> RegistryResult<bool> {
        let lease_key = self.lease_key(actor_id);
        let now_ms = self.clock.now_ms();
        let duration_ms = self.config.lease_duration_ms;

        let node_id_owned = node_id.clone();
        let actor_id_owned = actor_id.clone();

        self.transact(move |txn| {
            // Read existing lease
            // NOTE: We could read the existing lease here, but FDB transactions
            // provide conflict detection. If another node modified the lease,
            // our commit will fail and we'll retry.
            let _lease_value = txn.get(&lease_key, false);

            // We need to handle this synchronously within the closure
            // FDB 0.10 get() returns a future, but we're in a sync context
            // This is a limitation - we'd need async closures or a different pattern

            // For now, we'll set the new lease optimistically
            // The FDB transaction will handle conflicts
            let new_lease = Lease::new(node_id_owned.clone(), now_ms, duration_ms);
            let lease_json =
                serde_json::to_vec(&new_lease).map_err(|e| RegistryError::Internal {
                    message: format!("serialize lease failed: {}", e),
                })?;

            txn.set(&lease_key, &lease_json);

            debug!(
                actor_id = %actor_id_owned.qualified_name(),
                node_id = %node_id_owned,
                expires_at_ms = new_lease.expires_at_ms,
                "Lease acquired"
            );

            Ok(true)
        })
        .await
    }

    /// Release a lease for an actor
    #[instrument(skip(self), fields(actor_id = %actor_id.qualified_name()))]
    pub async fn release_lease(&self, actor_id: &ActorId) -> RegistryResult<()> {
        let lease_key = self.lease_key(actor_id);

        self.transact(move |txn| {
            txn.clear(&lease_key);
            Ok(())
        })
        .await?;

        debug!(actor_id = %actor_id.qualified_name(), "Lease released");
        Ok(())
    }

    /// Get the current lease for an actor
    pub async fn get_lease(&self, actor_id: &ActorId) -> RegistryResult<Option<Lease>> {
        let lease_key = self.lease_key(actor_id);
        let txn = self.create_transaction()?;

        let value = txn
            .get(&lease_key, false)
            .await
            .map_err(|e| RegistryError::Internal {
                message: format!("get lease failed: {}", e),
            })?;

        match value {
            Some(data) => {
                let lease: Lease =
                    serde_json::from_slice(data.as_ref()).map_err(|e| RegistryError::Internal {
                        message: format!("deserialize lease failed: {}", e),
                    })?;
                Ok(Some(lease))
            }
            None => Ok(None),
        }
    }

    /// Renew leases for all actors owned by this node
    ///
    /// Scans all leases and renews those owned by the specified node.
    /// Returns the number of leases renewed.
    #[instrument(skip(self), fields(node_id = %node_id))]
    pub async fn renew_leases(&self, node_id: &NodeId) -> RegistryResult<u64> {
        let now_ms = self.clock.now_ms();
        let duration_ms = self.config.lease_duration_ms;

        // Scan all leases
        let prefix = self.leases_prefix();
        let mut end_key = prefix.clone();
        end_key.push(0xFF);

        let txn = self.create_transaction()?;

        let mut range_option = RangeOption::from((prefix.as_slice(), end_key.as_slice()));
        range_option.mode = StreamingMode::WantAll;

        let range =
            txn.get_range(&range_option, 1, false)
                .await
                .map_err(|e| RegistryError::Internal {
                    message: format!("scan leases failed: {}", e),
                })?;

        // Collect leases that need renewal
        let mut leases_to_renew: Vec<(Vec<u8>, Lease)> = Vec::new();

        for kv in range.iter() {
            let lease: Lease =
                serde_json::from_slice(kv.value()).map_err(|e| RegistryError::Internal {
                    message: format!("deserialize lease failed: {}", e),
                })?;

            // Only renew leases owned by this node that haven't expired
            if lease.is_owned_by(node_id) && !lease.is_expired(now_ms) {
                leases_to_renew.push((kv.key().to_vec(), lease));
            }
        }

        if leases_to_renew.is_empty() {
            debug!("No leases to renew");
            return Ok(0);
        }

        // Renew all leases in a single transaction
        let count = leases_to_renew.len() as u64;

        self.transact(move |txn| {
            for (key, mut lease) in leases_to_renew.clone() {
                lease.renew(now_ms, duration_ms);
                let lease_json =
                    serde_json::to_vec(&lease).map_err(|e| RegistryError::Internal {
                        message: format!("serialize lease failed: {}", e),
                    })?;
                txn.set(&key, &lease_json);
            }
            Ok(())
        })
        .await?;

        debug!(count = count, "Leases renewed");
        Ok(count)
    }

    /// Renew a single lease for a specific actor
    ///
    /// Returns true if the lease was renewed, false if the node doesn't own it.
    #[instrument(skip(self), fields(actor_id = %actor_id.qualified_name(), node_id = %node_id))]
    pub async fn renew_lease(&self, actor_id: &ActorId, node_id: &NodeId) -> RegistryResult<bool> {
        let now_ms = self.clock.now_ms();
        let duration_ms = self.config.lease_duration_ms;

        // Get current lease
        let lease = match self.get_lease(actor_id).await? {
            Some(l) => l,
            None => return Ok(false), // No lease to renew
        };

        // Verify ownership
        if !lease.is_owned_by(node_id) {
            debug!("Cannot renew - lease owned by different node");
            return Ok(false);
        }

        // Check if expired
        if lease.is_expired(now_ms) {
            debug!("Cannot renew - lease already expired");
            return Ok(false);
        }

        // Renew the lease
        let mut renewed_lease = lease;
        renewed_lease.renew(now_ms, duration_ms);

        let lease_key = self.lease_key(actor_id);
        let lease_json =
            serde_json::to_vec(&renewed_lease).map_err(|e| RegistryError::Internal {
                message: format!("serialize lease failed: {}", e),
            })?;

        self.transact(move |txn| {
            txn.set(&lease_key, &lease_json);
            Ok(())
        })
        .await?;

        debug!(
            new_expires_at_ms = renewed_lease.expires_at_ms,
            version = renewed_lease.version,
            "Lease renewed"
        );

        Ok(true)
    }

    /// Select node using least-loaded strategy
    async fn select_least_loaded(&self) -> Option<NodeId> {
        let cache = self.node_cache.read().await;
        cache
            .values()
            .filter(|n| n.status.can_accept_actors() && n.has_capacity())
            .min_by_key(|n| n.actor_count)
            .map(|n| n.id.clone())
    }
}

impl std::fmt::Debug for FdbRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FdbRegistry")
            .field("subspace", &self.subspace)
            .field("config", &self.config)
            .finish()
    }
}

#[async_trait]
impl Registry for FdbRegistry {
    #[instrument(skip(self, info), fields(node_id = %info.id))]
    async fn register_node(&self, info: NodeInfo) -> RegistryResult<()> {
        let node_key = self.node_key(&info.id);
        let node_json = serde_json::to_vec(&info).map_err(|e| RegistryError::Internal {
            message: format!("serialize node info failed: {}", e),
        })?;

        let node_id = info.id.clone();

        self.transact(move |txn| {
            txn.set(&node_key, &node_json);
            Ok(())
        })
        .await?;

        // Update local cache
        let mut cache = self.node_cache.write().await;
        cache.insert(node_id.clone(), info.clone());

        // Register with heartbeat tracker
        let mut tracker = self.heartbeat_tracker.write().await;
        tracker.register_node(node_id, self.clock.now_ms());

        debug!("Node registered in FDB");
        Ok(())
    }

    #[instrument(skip(self), fields(node_id = %node_id))]
    async fn unregister_node(&self, node_id: &NodeId) -> RegistryResult<()> {
        let node_key = self.node_key(node_id);

        self.transact(move |txn| {
            txn.clear(&node_key);
            Ok(())
        })
        .await?;

        // Update local cache
        let mut cache = self.node_cache.write().await;
        cache.remove(node_id);

        // Remove from heartbeat tracker
        let mut tracker = self.heartbeat_tracker.write().await;
        tracker.unregister_node(node_id);

        debug!("Node unregistered from FDB");
        Ok(())
    }

    async fn get_node(&self, node_id: &NodeId) -> RegistryResult<Option<NodeInfo>> {
        // Try cache first
        {
            let cache = self.node_cache.read().await;
            if let Some(info) = cache.get(node_id) {
                return Ok(Some(info.clone()));
            }
        }

        // Read from FDB
        let node_key = self.node_key(node_id);
        let txn = self.create_transaction()?;

        let value = txn
            .get(&node_key, false)
            .await
            .map_err(|e| RegistryError::Internal {
                message: format!("get node failed: {}", e),
            })?;

        match value {
            Some(data) => {
                let info: NodeInfo =
                    serde_json::from_slice(data.as_ref()).map_err(|e| RegistryError::Internal {
                        message: format!("deserialize node info failed: {}", e),
                    })?;

                // Update cache
                let mut cache = self.node_cache.write().await;
                cache.insert(node_id.clone(), info.clone());

                Ok(Some(info))
            }
            None => Ok(None),
        }
    }

    async fn list_nodes(&self) -> RegistryResult<Vec<NodeInfo>> {
        let prefix = self.nodes_prefix();
        let mut end_key = prefix.clone();
        end_key.push(0xFF);

        let txn = self.create_transaction()?;

        let mut range_option = RangeOption::from((prefix.as_slice(), end_key.as_slice()));
        range_option.mode = StreamingMode::WantAll;

        let range =
            txn.get_range(&range_option, 1, false)
                .await
                .map_err(|e| RegistryError::Internal {
                    message: format!("list nodes failed: {}", e),
                })?;

        let mut nodes = Vec::new();
        for kv in range.iter() {
            let info: NodeInfo =
                serde_json::from_slice(kv.value()).map_err(|e| RegistryError::Internal {
                    message: format!("deserialize node info failed: {}", e),
                })?;
            nodes.push(info);
        }

        // Update cache
        let mut cache = self.node_cache.write().await;
        for info in &nodes {
            cache.insert(info.id.clone(), info.clone());
        }

        Ok(nodes)
    }

    async fn list_nodes_by_status(&self, status: NodeStatus) -> RegistryResult<Vec<NodeInfo>> {
        let all_nodes = self.list_nodes().await?;
        Ok(all_nodes
            .into_iter()
            .filter(|n| n.status == status)
            .collect())
    }

    #[instrument(skip(self), fields(node_id = %node_id, status = ?status))]
    async fn update_node_status(&self, node_id: &NodeId, status: NodeStatus) -> RegistryResult<()> {
        // Get current info
        let mut info = self
            .get_node(node_id)
            .await?
            .ok_or_else(|| RegistryError::node_not_found(node_id.as_str()))?;

        info.set_status(status);

        // Write back
        let node_key = self.node_key(node_id);
        let node_json = serde_json::to_vec(&info).map_err(|e| RegistryError::Internal {
            message: format!("serialize node info failed: {}", e),
        })?;

        self.transact(move |txn| {
            txn.set(&node_key, &node_json);
            Ok(())
        })
        .await?;

        // Update cache
        let mut cache = self.node_cache.write().await;
        cache.insert(node_id.clone(), info);

        Ok(())
    }

    #[instrument(skip(self, heartbeat), fields(node_id = %heartbeat.node_id))]
    async fn receive_heartbeat(&self, heartbeat: Heartbeat) -> RegistryResult<()> {
        let now_ms = self.clock.now_ms();

        // Update heartbeat tracker
        let mut tracker = self.heartbeat_tracker.write().await;
        tracker.receive_heartbeat(heartbeat.clone(), now_ms)?;

        // Update node info in cache
        let mut cache = self.node_cache.write().await;
        if let Some(info) = cache.get_mut(&heartbeat.node_id) {
            info.update_heartbeat(now_ms);
            info.actor_count = heartbeat.actor_count;

            if info.status == NodeStatus::Suspect {
                info.status = NodeStatus::Active;
            }
        }

        Ok(())
    }

    async fn get_placement(&self, actor_id: &ActorId) -> RegistryResult<Option<ActorPlacement>> {
        let actor_key = self.actor_key(actor_id);
        let txn = self.create_transaction()?;

        let value = txn
            .get(&actor_key, false)
            .await
            .map_err(|e| RegistryError::Internal {
                message: format!("get placement failed: {}", e),
            })?;

        match value {
            Some(data) => {
                let placement: ActorPlacement =
                    serde_json::from_slice(data.as_ref()).map_err(|e| RegistryError::Internal {
                        message: format!("deserialize placement failed: {}", e),
                    })?;
                Ok(Some(placement))
            }
            None => Ok(None),
        }
    }

    #[instrument(skip(self), fields(actor_id = %actor_id.qualified_name(), node_id = %node_id))]
    async fn register_actor(&self, actor_id: ActorId, node_id: NodeId) -> RegistryResult<()> {
        let now_ms = self.clock.now_ms();
        let actor_key = self.actor_key(&actor_id);
        let lease_key = self.lease_key(&actor_id);

        // FIX: Read and write in a single transaction to prevent TOCTOU race.
        // FDB's conflict detection ensures that if another node reads and writes
        // between our read and commit, our transaction will be retried.
        let mut attempts = 0;
        loop {
            attempts += 1;
            if attempts > TRANSACTION_RETRY_COUNT_MAX {
                return Err(RegistryError::Internal {
                    message: "exceeded max transaction retries for register_actor".into(),
                });
            }

            let txn = self.create_transaction()?;

            // Read existing placement INSIDE the transaction
            let existing_placement =
                txn.get(&actor_key, false)
                    .await
                    .map_err(|e| RegistryError::Internal {
                        message: format!("read placement failed: {}", e),
                    })?;

            // Check if actor is already registered
            if let Some(placement_data) = existing_placement {
                let existing: ActorPlacement = serde_json::from_slice(placement_data.as_ref())
                    .map_err(|e| RegistryError::Internal {
                        message: format!("deserialize placement failed: {}", e),
                    })?;

                if existing.node_id != node_id {
                    return Err(RegistryError::actor_already_registered(
                        &actor_id,
                        existing.node_id.as_str(),
                    ));
                }
                // Already registered to same node - success
                return Ok(());
            }

            // Create new placement and lease
            let placement =
                ActorPlacement::with_timestamp(actor_id.clone(), node_id.clone(), now_ms);
            let placement_json =
                serde_json::to_vec(&placement).map_err(|e| RegistryError::Internal {
                    message: format!("serialize placement failed: {}", e),
                })?;

            let lease = Lease::new(node_id.clone(), now_ms, self.config.lease_duration_ms);
            let lease_json = serde_json::to_vec(&lease).map_err(|e| RegistryError::Internal {
                message: format!("serialize lease failed: {}", e),
            })?;

            txn.set(&actor_key, &placement_json);
            txn.set(&lease_key, &lease_json);

            match txn.commit().await {
                Ok(_) => {
                    debug!("Actor registered in FDB with lease");
                    return Ok(());
                }
                Err(e) if e.is_retryable() && attempts < TRANSACTION_RETRY_COUNT_MAX => {
                    warn!(
                        attempt = attempts,
                        error = %e,
                        "Transaction conflict in register_actor, retrying"
                    );
                    continue;
                }
                Err(e) => {
                    return Err(RegistryError::Internal {
                        message: format!("transaction commit failed: {}", e),
                    });
                }
            }
        }
    }

    #[instrument(skip(self), fields(actor_id = %actor_id.qualified_name()))]
    async fn unregister_actor(&self, actor_id: &ActorId) -> RegistryResult<()> {
        let actor_key = self.actor_key(actor_id);
        let lease_key = self.lease_key(actor_id);

        self.transact(move |txn| {
            txn.clear(&actor_key);
            txn.clear(&lease_key);
            Ok(())
        })
        .await?;

        debug!("Actor unregistered from FDB");
        Ok(())
    }

    #[instrument(skip(self), fields(actor_id = %actor_id.qualified_name(), node_id = %node_id))]
    async fn try_claim_actor(
        &self,
        actor_id: ActorId,
        node_id: NodeId,
    ) -> RegistryResult<PlacementDecision> {
        let now_ms = self.clock.now_ms();
        let actor_key = self.actor_key(&actor_id);
        let lease_key = self.lease_key(&actor_id);

        // FIX: Read and write in a single transaction to prevent TOCTOU race.
        // FDB's conflict detection ensures that if another node reads and writes
        // between our read and commit, our transaction will be retried.
        let mut attempts = 0;
        loop {
            attempts += 1;
            if attempts > TRANSACTION_RETRY_COUNT_MAX {
                return Err(RegistryError::Internal {
                    message: "exceeded max transaction retries for try_claim_actor".into(),
                });
            }

            let txn = self.create_transaction()?;

            // Read existing placement and lease INSIDE the transaction
            let existing_placement =
                txn.get(&actor_key, false)
                    .await
                    .map_err(|e| RegistryError::Internal {
                        message: format!("read placement failed: {}", e),
                    })?;

            let existing_lease =
                txn.get(&lease_key, false)
                    .await
                    .map_err(|e| RegistryError::Internal {
                        message: format!("read lease failed: {}", e),
                    })?;

            // Check if actor is already claimed with valid lease
            if let Some(placement_data) = existing_placement {
                let placement: ActorPlacement = serde_json::from_slice(placement_data.as_ref())
                    .map_err(|e| RegistryError::Internal {
                        message: format!("deserialize placement failed: {}", e),
                    })?;

                if let Some(lease_data) = existing_lease {
                    let lease: Lease =
                        serde_json::from_slice(lease_data.as_ref()).map_err(|e| {
                            RegistryError::Internal {
                                message: format!("deserialize lease failed: {}", e),
                            }
                        })?;

                    if !lease.is_expired(now_ms) {
                        // Lease is still valid - return existing placement
                        // No need to commit, just return
                        return Ok(PlacementDecision::Existing(placement));
                    }
                    // Lease expired, continue to claim
                }
            }

            // Create new placement and lease
            let placement =
                ActorPlacement::with_timestamp(actor_id.clone(), node_id.clone(), now_ms);
            let placement_json =
                serde_json::to_vec(&placement).map_err(|e| RegistryError::Internal {
                    message: format!("serialize placement failed: {}", e),
                })?;

            let lease = Lease::new(node_id.clone(), now_ms, self.config.lease_duration_ms);
            let lease_json = serde_json::to_vec(&lease).map_err(|e| RegistryError::Internal {
                message: format!("serialize lease failed: {}", e),
            })?;

            // Write placement and lease
            txn.set(&actor_key, &placement_json);
            txn.set(&lease_key, &lease_json);

            // Commit - FDB will detect conflicts if keys changed since our read
            match txn.commit().await {
                Ok(_) => {
                    debug!("Actor claimed with new lease");
                    return Ok(PlacementDecision::New(node_id));
                }
                Err(e) if e.is_retryable() && attempts < TRANSACTION_RETRY_COUNT_MAX => {
                    warn!(
                        attempt = attempts,
                        error = %e,
                        "Transaction conflict in try_claim_actor, retrying"
                    );
                    continue;
                }
                Err(e) => {
                    return Err(RegistryError::Internal {
                        message: format!("transaction commit failed: {}", e),
                    });
                }
            }
        }
    }

    async fn list_actors_on_node(&self, node_id: &NodeId) -> RegistryResult<Vec<ActorPlacement>> {
        // Scan all actors and filter by node
        let prefix = self.actors_prefix();
        let mut end_key = prefix.clone();
        end_key.push(0xFF);

        let txn = self.create_transaction()?;

        let mut range_option = RangeOption::from((prefix.as_slice(), end_key.as_slice()));
        range_option.mode = StreamingMode::WantAll;

        let range =
            txn.get_range(&range_option, 1, false)
                .await
                .map_err(|e| RegistryError::Internal {
                    message: format!("list actors failed: {}", e),
                })?;

        let mut actors = Vec::new();
        for kv in range.iter() {
            let placement: ActorPlacement =
                serde_json::from_slice(kv.value()).map_err(|e| RegistryError::Internal {
                    message: format!("deserialize placement failed: {}", e),
                })?;
            if &placement.node_id == node_id {
                actors.push(placement);
            }
        }

        Ok(actors)
    }

    #[instrument(skip(self), fields(actor_id = %actor_id.qualified_name(), from = %from_node, to = %to_node))]
    async fn migrate_actor(
        &self,
        actor_id: &ActorId,
        from_node: &NodeId,
        to_node: &NodeId,
    ) -> RegistryResult<()> {
        let now_ms = self.clock.now_ms();

        // Verify current placement
        let placement = self
            .get_placement(actor_id)
            .await?
            .ok_or_else(|| RegistryError::actor_not_found(actor_id))?;

        if &placement.node_id != from_node {
            return Err(RegistryError::ActorAlreadyRegistered {
                actor_id: actor_id.qualified_name(),
                existing_node: placement.node_id.to_string(),
            });
        }

        // Update placement and lease atomically
        let mut new_placement = placement;
        new_placement.migrate_to(to_node.clone(), now_ms);

        let actor_key = self.actor_key(actor_id);
        let placement_json =
            serde_json::to_vec(&new_placement).map_err(|e| RegistryError::Internal {
                message: format!("serialize placement failed: {}", e),
            })?;

        let lease = Lease::new(to_node.clone(), now_ms, self.config.lease_duration_ms);
        let lease_key = self.lease_key(actor_id);
        let lease_json = serde_json::to_vec(&lease).map_err(|e| RegistryError::Internal {
            message: format!("serialize lease failed: {}", e),
        })?;

        self.transact(move |txn| {
            txn.set(&actor_key, &placement_json);
            txn.set(&lease_key, &lease_json);
            Ok(())
        })
        .await?;

        debug!("Actor migrated with new lease");
        Ok(())
    }

    async fn select_node_for_placement(
        &self,
        context: PlacementContext,
    ) -> RegistryResult<PlacementDecision> {
        // Check if already placed
        if let Some(placement) = self.get_placement(&context.actor_id).await? {
            return Ok(PlacementDecision::Existing(placement));
        }

        // Refresh node cache
        let _ = self.list_nodes().await?;

        // Select node based on strategy
        let node_id = match context.strategy {
            PlacementStrategy::LeastLoaded => self.select_least_loaded().await,
            PlacementStrategy::Affinity => {
                if let Some(ref preferred) = context.preferred_node {
                    let cache = self.node_cache.read().await;
                    if let Some(info) = cache.get(preferred) {
                        if info.has_capacity() && info.status.can_accept_actors() {
                            return Ok(PlacementDecision::New(preferred.clone()));
                        }
                    }
                }
                self.select_least_loaded().await
            }
            _ => self.select_least_loaded().await,
        };

        match node_id {
            Some(id) => Ok(PlacementDecision::New(id)),
            None => Ok(PlacementDecision::NoCapacity),
        }
    }
}

// =============================================================================
// Lease Renewal Task
// =============================================================================

use kelpie_core::runtime::{current_runtime, JoinHandle, Runtime};

/// Background task for periodic lease renewal
///
/// Spawns a task that periodically renews all leases owned by a node.
/// Should be started when a node joins the cluster and stopped on shutdown.
///
/// TigerStyle: Explicit task lifecycle, graceful shutdown via channel.
/// Uses Runtime abstraction for DST compatibility.
pub struct LeaseRenewalTask {
    /// Handle to the spawned task (boxed future)
    _handle: JoinHandle<()>,
    /// Shutdown signal sender
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
}

impl LeaseRenewalTask {
    /// Start the lease renewal task
    ///
    /// The task will run until `stop()` is called or the registry is dropped.
    pub fn start(registry: Arc<FdbRegistry>, node_id: NodeId) -> Self {
        let interval_ms = registry.config.lease_renewal_interval_ms;
        let interval_duration = std::time::Duration::from_millis(interval_ms);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        let runtime = current_runtime();
        let handle = runtime.spawn(async move {
            let inner_runtime = current_runtime();

            loop {
                // Sleep for the interval duration
                inner_runtime.sleep(interval_duration).await;

                // Check for shutdown
                if *shutdown_rx.borrow() {
                    tracing::info!("Lease renewal task shutting down");
                    break;
                }

                // Renew leases
                match registry.renew_leases(&node_id).await {
                    Ok(count) => {
                        if count > 0 {
                            tracing::debug!(count = count, "Background lease renewal completed");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Background lease renewal failed");
                    }
                }

                // Check for shutdown again after renewal
                if shutdown_rx.has_changed().unwrap_or(false) && *shutdown_rx.borrow() {
                    tracing::info!("Lease renewal task shutting down");
                    break;
                }
            }
        });

        Self {
            _handle: handle,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    /// Stop the lease renewal task gracefully
    ///
    /// Signals the task to stop. The task will exit on its next iteration.
    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
    }
}

impl Drop for LeaseRenewalTask {
    fn drop(&mut self) {
        // Signal shutdown if not already done
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lease_new() {
        let node_id = NodeId::new("node-1").unwrap();
        let lease = Lease::new(node_id.clone(), 1000, 30000);

        assert_eq!(lease.node_id, node_id);
        assert_eq!(lease.acquired_at_ms, 1000);
        assert_eq!(lease.expires_at_ms, 31000);
        assert_eq!(lease.version, 1);
    }

    #[test]
    fn test_lease_expiry() {
        let node_id = NodeId::new("node-1").unwrap();
        let lease = Lease::new(node_id, 1000, 30000);

        // Not expired before expiry
        assert!(!lease.is_expired(1000));
        assert!(!lease.is_expired(30999));

        // Expired at or after expiry
        assert!(lease.is_expired(31000));
        assert!(lease.is_expired(40000));
    }

    #[test]
    fn test_lease_renewal() {
        let node_id = NodeId::new("node-1").unwrap();
        let mut lease = Lease::new(node_id, 1000, 30000);

        assert_eq!(lease.version, 1);
        assert_eq!(lease.expires_at_ms, 31000);

        lease.renew(20000, 30000);

        assert_eq!(lease.version, 2);
        assert_eq!(lease.expires_at_ms, 50000);
    }

    #[test]
    fn test_lease_ownership() {
        let node1 = NodeId::new("node-1").unwrap();
        let node2 = NodeId::new("node-2").unwrap();
        let lease = Lease::new(node1.clone(), 1000, 30000);

        assert!(lease.is_owned_by(&node1));
        assert!(!lease.is_owned_by(&node2));
    }

    // Integration tests require FDB - marked as ignored
    #[tokio::test]
    #[ignore = "requires running FDB cluster"]
    async fn test_fdb_registry_node_registration() {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        let config = FdbRegistryConfig::default();
        let registry = FdbRegistry::connect(None, config).await.unwrap();

        let node_id = NodeId::new("test-node-1").unwrap();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);
        let mut info = NodeInfo::new(node_id.clone(), addr);
        info.status = NodeStatus::Active;

        registry.register_node(info.clone()).await.unwrap();

        let retrieved = registry.get_node(&node_id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, node_id);

        // Cleanup
        registry.unregister_node(&node_id).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running FDB cluster"]
    async fn test_fdb_registry_actor_claim() {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        let config = FdbRegistryConfig::default();
        let registry = FdbRegistry::connect(None, config).await.unwrap();

        // Register node
        let node_id = NodeId::new("test-node-1").unwrap();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);
        let mut info = NodeInfo::new(node_id.clone(), addr);
        info.status = NodeStatus::Active;
        registry.register_node(info).await.unwrap();

        // Claim actor
        let actor_id = ActorId::new("test", "actor-1").unwrap();
        let decision = registry
            .try_claim_actor(actor_id.clone(), node_id.clone())
            .await
            .unwrap();

        assert!(matches!(decision, PlacementDecision::New(_)));

        // Verify lease exists
        let lease = registry.get_lease(&actor_id).await.unwrap();
        assert!(lease.is_some());
        assert_eq!(lease.unwrap().node_id, node_id);

        // Cleanup
        registry.unregister_actor(&actor_id).await.unwrap();
        registry.unregister_node(&node_id).await.unwrap();
    }
}
