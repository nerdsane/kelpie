//! Main cluster coordinator
//!
//! TigerStyle: Single entry point for cluster operations.

use crate::config::ClusterConfig;
use crate::error::{ClusterError, ClusterResult};
use crate::migration::{plan_migrations, MigrationCoordinator};
use crate::rpc::{RpcMessage, RpcTransport};
use async_trait::async_trait;
use bytes::Bytes;
use kelpie_core::actor::ActorId;
use kelpie_core::runtime::{JoinHandle, Runtime};
use kelpie_registry::{Heartbeat, NodeId, NodeInfo, NodeStatus, PlacementDecision, Registry};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, RwLock};
use tracing::{debug, info, warn};

// ============================================================================
// Actor State Provider
// ============================================================================

/// Trait for providing actor state during migration
///
/// This trait is implemented by the runtime to allow the cluster to:
/// 1. Get the serialized state of a local actor
/// 2. Deactivate an actor after successful migration
#[async_trait]
pub trait ActorStateProvider: Send + Sync {
    /// Get the serialized state of an actor
    ///
    /// Returns None if the actor is not locally active.
    async fn get_actor_state(&self, actor_id: &ActorId) -> Result<Option<Bytes>, String>;

    /// Deactivate an actor locally after migration
    ///
    /// This should stop the actor and clean up local resources.
    /// The registry update is handled by the cluster.
    async fn deactivate_local(&self, actor_id: &ActorId) -> Result<(), String>;
}

/// Cluster state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterState {
    /// Cluster not started
    Stopped,
    /// Cluster is initializing
    Initializing,
    /// Cluster is running
    Running,
    /// Cluster is shutting down
    ShuttingDown,
}

/// The main cluster coordinator
///
/// Manages cluster membership, heartbeats, and actor placement.
pub struct Cluster<R: Registry + 'static, T: RpcTransport + 'static, RT: Runtime + 'static> {
    /// Local node information
    local_node: NodeInfo,
    /// Cluster configuration
    config: ClusterConfig,
    /// Registry for actor placement
    registry: Arc<R>,
    /// RPC transport
    transport: Arc<T>,
    /// Runtime for task spawning and time
    runtime: RT,
    /// Migration coordinator
    migration: Arc<MigrationCoordinator<R, T>>,
    /// Actor state provider for migration
    state_provider: Option<Arc<dyn ActorStateProvider>>,
    /// Current cluster state
    state: RwLock<ClusterState>,
    /// Heartbeat task handle
    heartbeat_task: RwLock<Option<JoinHandle<()>>>,
    /// Failure detection task handle
    failure_task: RwLock<Option<JoinHandle<()>>>,
    /// Shutdown signal sender (sends true when shutting down)
    shutdown_tx: watch::Sender<bool>,
    /// Shutdown signal receiver (clone for each task)
    shutdown_rx: watch::Receiver<bool>,
    /// Next heartbeat sequence (for future use in coordinated heartbeats)
    #[allow(dead_code)]
    heartbeat_sequence: AtomicU64,
}

impl<R: Registry + 'static, T: RpcTransport + 'static, RT: Runtime + 'static> Cluster<R, T, RT> {
    /// Create a new cluster instance
    pub fn new(
        local_node: NodeInfo,
        config: ClusterConfig,
        registry: Arc<R>,
        transport: Arc<T>,
        runtime: RT,
    ) -> Self {
        let migration = Arc::new(MigrationCoordinator::new(
            local_node.id.clone(),
            registry.clone(),
            transport.clone(),
            config.rpc_timeout(),
        ));

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            local_node,
            config,
            registry,
            transport,
            runtime,
            migration,
            state_provider: None,
            state: RwLock::new(ClusterState::Stopped),
            heartbeat_task: RwLock::new(None),
            failure_task: RwLock::new(None),
            shutdown_tx,
            shutdown_rx,
            heartbeat_sequence: AtomicU64::new(0),
        }
    }

    /// Set the actor state provider for migration support
    ///
    /// The state provider allows the cluster to get actor state and deactivate
    /// actors locally during migration. Without a state provider, drain_actors()
    /// will only unregister actors without transferring state.
    pub fn with_state_provider(mut self, provider: Arc<dyn ActorStateProvider>) -> Self {
        self.state_provider = Some(provider);
        self
    }

    /// Get the local node ID
    pub fn local_node_id(&self) -> &NodeId {
        &self.local_node.id
    }

    /// Get the local node info
    pub fn local_node(&self) -> &NodeInfo {
        &self.local_node
    }

    /// Get the cluster configuration
    pub fn config(&self) -> &ClusterConfig {
        &self.config
    }

    /// Get the current cluster state
    pub async fn state(&self) -> ClusterState {
        *self.state.read().await
    }

    /// Check if cluster is running
    pub async fn is_running(&self) -> bool {
        *self.state.read().await == ClusterState::Running
    }

    /// Start the cluster
    pub async fn start(&self) -> ClusterResult<()> {
        {
            let mut state = self.state.write().await;
            if *state != ClusterState::Stopped {
                return Err(ClusterError::AlreadyStarted);
            }
            *state = ClusterState::Initializing;
        }

        info!(
            node_id = %self.local_node.id,
            addr = %self.config.rpc_addr,
            "starting cluster node"
        );

        // Register local node with registry
        let mut node = self.local_node.clone();
        node.status = NodeStatus::Active;
        self.registry.register_node(node).await?;

        // Start RPC transport
        self.transport.start().await?;

        // Join cluster if we have seed nodes
        if !self.config.seed_nodes.is_empty() {
            self.join_cluster().await?;
        }

        // Start heartbeat task
        self.start_heartbeat_task().await;

        // Start failure detection task
        self.start_failure_detection_task().await;

        {
            let mut state = self.state.write().await;
            *state = ClusterState::Running;
        }

        info!(
            node_id = %self.local_node.id,
            "cluster node started"
        );

        Ok(())
    }

    /// Stop the cluster
    pub async fn stop(&self) -> ClusterResult<()> {
        {
            let state = self.state.read().await;
            if *state == ClusterState::Stopped {
                return Ok(());
            }
        }

        info!(
            node_id = %self.local_node.id,
            "stopping cluster node"
        );

        // Signal shutdown via watch channel (receivers will see value change to true)
        let _ = self.shutdown_tx.send(true);

        {
            let mut state = self.state.write().await;
            *state = ClusterState::ShuttingDown;
        }

        // Drain actors if configured
        if self.config.drain_on_shutdown {
            if let Err(e) = self.drain_actors().await {
                warn!(error = %e, "failed to drain actors during shutdown");
            }
        }

        // Stop heartbeat task
        if let Some(task) = self.heartbeat_task.write().await.take() {
            // Wait for task to finish (signaled via shutdown notify)
            let _ = task.await;
        }

        // Stop failure detection task
        if let Some(task) = self.failure_task.write().await.take() {
            // Wait for task to finish (signaled via shutdown notify)
            let _ = task.await;
        }

        // Notify cluster of leave
        let leave_msg = RpcMessage::LeaveNotification {
            node_id: self.local_node.id.clone(),
        };
        let _ = self.transport.broadcast(leave_msg).await;

        // Stop transport
        self.transport.stop().await?;

        // Update local node status (transition through Leaving first)
        let _ = self
            .registry
            .update_node_status(&self.local_node.id, NodeStatus::Leaving)
            .await;
        let _ = self
            .registry
            .update_node_status(&self.local_node.id, NodeStatus::Left)
            .await;

        {
            let mut state = self.state.write().await;
            *state = ClusterState::Stopped;
        }

        info!(
            node_id = %self.local_node.id,
            "cluster node stopped"
        );

        Ok(())
    }

    /// Join an existing cluster
    ///
    /// TODO(Phase 3): This currently does nothing. Once FdbRegistry is implemented,
    /// cluster membership will be managed through FDB transactions instead of gossip.
    /// The seed_nodes config will be used for initial FDB cluster connection, not
    /// for a separate cluster join protocol.
    async fn join_cluster(&self) -> ClusterResult<()> {
        info!("joining cluster via seed nodes");

        for seed_addr in &self.config.seed_nodes {
            // TODO(Phase 3): Use FDB for cluster membership discovery
            // Seed nodes will point to FDB coordinators, not peer Kelpie nodes
            debug!(seed = %seed_addr, "seed node configured (FDB membership not yet implemented)");
        }

        // For now, single-node operation works. Multi-node requires FdbRegistry (Phase 3)
        Ok(())
    }

    /// Start the heartbeat sending task
    async fn start_heartbeat_task(&self) {
        let registry = self.registry.clone();
        let transport = self.transport.clone();
        let node_id = self.local_node.id.clone();
        let interval = Duration::from_millis(self.config.heartbeat.interval_ms);
        let mut shutdown_rx = self.shutdown_rx.clone();
        let sequence = Arc::new(AtomicU64::new(0));
        let runtime = self.runtime.clone();

        let task = self.runtime.spawn(async move {
            loop {
                // Check if shutdown was already signaled before we started
                if *shutdown_rx.borrow() {
                    debug!("heartbeat task shutting down (pre-signaled)");
                    break;
                }

                tokio::select! {
                    biased;  // Prioritize shutdown check

                    result = shutdown_rx.changed() => {
                        // Channel closed or value changed to true
                        if result.is_err() || *shutdown_rx.borrow() {
                            debug!("heartbeat task shutting down");
                            break;
                        }
                    }
                    _ = runtime.sleep(interval) => {
                        // Get current actor count
                        let actor_count = registry
                            .list_actors_on_node(&node_id)
                            .await
                            .map(|a| a.len() as u64)
                            .unwrap_or(0);

                        let seq = sequence.fetch_add(1, Ordering::SeqCst);

                        let heartbeat = Heartbeat::new(
                            node_id.clone(),
                            now_ms(),
                            NodeStatus::Active,
                            actor_count,
                            1_000_000 - actor_count, // Available capacity
                            seq,
                        );

                        // Broadcast heartbeat
                        if let Err(e) = transport.broadcast(RpcMessage::Heartbeat(heartbeat)).await {
                            debug!(error = %e, "failed to broadcast heartbeat");
                        }
                    }
                }
            }
        });

        *self.heartbeat_task.write().await = Some(task);
    }

    /// Start the failure detection task
    async fn start_failure_detection_task(&self) {
        let registry = self.registry.clone();
        let _migration = self.migration.clone();
        let config = self.config.clone();
        let local_node_id = self.local_node.id.clone();
        let interval = Duration::from_millis(self.config.heartbeat.interval_ms);
        let mut shutdown_rx = self.shutdown_rx.clone();
        let runtime = self.runtime.clone();

        let task = self.runtime.spawn(async move {
            loop {
                // Check if shutdown was already signaled before we started
                if *shutdown_rx.borrow() {
                    debug!("failure detection task shutting down (pre-signaled)");
                    break;
                }

                tokio::select! {
                    biased;  // Prioritize shutdown check

                    result = shutdown_rx.changed() => {
                        // Channel closed or value changed to true
                        if result.is_err() || *shutdown_rx.borrow() {
                            debug!("failure detection task shutting down");
                            break;
                        }
                    }
                    _ = runtime.sleep(interval) => {
                        // Check for failed nodes (if registry supports it)
                        // For MemoryRegistry, we'd need to add a method to check timeouts

                        // Get nodes that are marked as failed
                        if let Ok(failed_nodes) = registry.list_nodes_by_status(NodeStatus::Failed).await {
                            for node in failed_nodes {
                                if node.id == local_node_id {
                                    continue; // Skip self
                                }

                                if config.auto_migrate_on_failure {
                                    // Plan and execute migrations
                                    match plan_migrations(
                                        registry.as_ref(),
                                        &node.id,
                                        config.migration_batch_size,
                                    ).await {
                                        Ok(migrations) => {
                                            for (placement, target) in migrations {
                                                debug!(
                                                    actor = %placement.actor_id,
                                                    from = %node.id,
                                                    to = %target,
                                                    "planning migration due to node failure"
                                                );
                                                // TODO(Phase 6): Execute migration via MigrationCoordinator
                                                // Requires cluster RPC for state transfer
                                            }
                                        }
                                        Err(e) => {
                                            warn!(
                                                error = %e,
                                                node = %node.id,
                                                "failed to plan migrations"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        *self.failure_task.write().await = Some(task);
    }

    /// Drain actors from this node (for graceful shutdown)
    ///
    /// If a state provider is configured, actors are migrated to other nodes:
    /// 1. Select target nodes for each actor
    /// 2. Use MigrationCoordinator to transfer state
    /// 3. Deactivate locally after successful migration
    ///
    /// If no state provider, actors are simply unregistered (state is lost).
    async fn drain_actors(&self) -> ClusterResult<()> {
        info!("draining actors from node");

        let actors = self
            .registry
            .list_actors_on_node(&self.local_node.id)
            .await?;

        if actors.is_empty() {
            return Ok(());
        }

        info!(count = actors.len(), "actors to drain");

        // Get available target nodes (excluding self)
        let available_nodes: Vec<NodeInfo> = self
            .registry
            .list_nodes_by_status(NodeStatus::Active)
            .await?
            .into_iter()
            .filter(|n| n.id != self.local_node.id)
            .collect();

        // If no state provider or no target nodes, fall back to unregister
        if self.state_provider.is_none() || available_nodes.is_empty() {
            if self.state_provider.is_none() {
                warn!(
                    "no state provider configured, actors will be unregistered without migration"
                );
            } else {
                warn!("no available target nodes, actors will be unregistered without migration");
            }

            for placement in actors {
                if let Err(e) = self.registry.unregister_actor(&placement.actor_id).await {
                    warn!(
                        actor = %placement.actor_id,
                        error = %e,
                        "failed to unregister actor during drain"
                    );
                }
            }
            return Ok(());
        }

        let state_provider = self.state_provider.as_ref().unwrap();

        // Migrate each actor
        let mut migrated = 0;
        let mut failed = 0;

        for placement in &actors {
            // Select target node (simple round-robin based on actor index)
            let target_idx = migrated % available_nodes.len();
            let target_node = &available_nodes[target_idx];

            debug!(
                actor = %placement.actor_id,
                target = %target_node.id,
                "migrating actor during drain"
            );

            // Get actor state
            let state = match state_provider.get_actor_state(&placement.actor_id).await {
                Ok(Some(s)) => s,
                Ok(None) => {
                    // Actor not locally active, just unregister
                    debug!(
                        actor = %placement.actor_id,
                        "actor not locally active, skipping migration"
                    );
                    if let Err(e) = self.registry.unregister_actor(&placement.actor_id).await {
                        warn!(
                            actor = %placement.actor_id,
                            error = %e,
                            "failed to unregister inactive actor"
                        );
                    }
                    continue;
                }
                Err(e) => {
                    warn!(
                        actor = %placement.actor_id,
                        error = %e,
                        "failed to get actor state for migration"
                    );
                    failed += 1;
                    continue;
                }
            };

            // Perform migration via MigrationCoordinator
            match self
                .migration
                .migrate(
                    placement.actor_id.clone(),
                    self.local_node.id.clone(),
                    target_node.id.clone(),
                    state,
                    now_ms(),
                )
                .await
            {
                Ok(()) => {
                    // Deactivate locally after successful migration
                    if let Err(e) = state_provider.deactivate_local(&placement.actor_id).await {
                        warn!(
                            actor = %placement.actor_id,
                            error = %e,
                            "failed to deactivate actor locally after migration"
                        );
                    }
                    migrated += 1;
                    debug!(
                        actor = %placement.actor_id,
                        target = %target_node.id,
                        "actor migrated successfully"
                    );
                }
                Err(e) => {
                    warn!(
                        actor = %placement.actor_id,
                        target = %target_node.id,
                        error = %e,
                        "failed to migrate actor"
                    );
                    failed += 1;
                }
            }
        }

        info!(
            total = actors.len(),
            migrated = migrated,
            failed = failed,
            "drain complete"
        );

        Ok(())
    }

    /// Get a placement decision for an actor
    pub async fn get_placement(&self, actor_id: &ActorId) -> ClusterResult<PlacementDecision> {
        let placement = self.registry.get_placement(actor_id).await?;

        if let Some(p) = placement {
            return Ok(PlacementDecision::Existing(p));
        }

        // Select a node for new placement
        let context = kelpie_registry::PlacementContext::new(actor_id.clone());
        let decision = self.registry.select_node_for_placement(context).await?;

        Ok(decision)
    }

    /// Try to claim an actor for the local node
    pub async fn try_claim_local(&self, actor_id: ActorId) -> ClusterResult<PlacementDecision> {
        let decision = self
            .registry
            .try_claim_actor(actor_id, self.local_node.id.clone())
            .await?;

        Ok(decision)
    }

    /// Get migration coordinator
    pub fn migration(&self) -> &MigrationCoordinator<R, T> {
        &self.migration
    }

    /// Get all nodes in the cluster
    pub async fn list_nodes(&self) -> ClusterResult<Vec<NodeInfo>> {
        Ok(self.registry.list_nodes().await?)
    }

    /// Get active nodes in the cluster
    pub async fn list_active_nodes(&self) -> ClusterResult<Vec<NodeInfo>> {
        Ok(self
            .registry
            .list_nodes_by_status(NodeStatus::Active)
            .await?)
    }
}

/// Get current time in milliseconds
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::MemoryTransport;
    use kelpie_core::TokioRuntime;
    use kelpie_registry::MemoryRegistry;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn test_addr(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
    }

    fn test_node_id(n: u32) -> NodeId {
        NodeId::new(format!("node-{}", n)).unwrap()
    }

    #[tokio::test]
    async fn test_cluster_create() {
        let node_id = test_node_id(1);
        let addr = test_addr(9001);
        let mut node = NodeInfo::new(node_id.clone(), addr);
        node.status = NodeStatus::Active;

        let config = ClusterConfig::single_node(addr);
        let registry = Arc::new(MemoryRegistry::new());
        let runtime = TokioRuntime;
        let transport = Arc::new(MemoryTransport::new(node_id.clone(), addr, runtime.clone()));

        let cluster = Cluster::new(node, config, registry, transport, runtime);

        assert_eq!(cluster.local_node_id(), &node_id);
        assert_eq!(cluster.state().await, ClusterState::Stopped);
    }

    #[tokio::test]
    async fn test_cluster_start_stop() {
        let node_id = test_node_id(1);
        let addr = test_addr(9001);
        let mut node = NodeInfo::with_timestamp(node_id.clone(), addr, 1000);
        node.status = NodeStatus::Active;

        let config = ClusterConfig::for_testing();
        let registry = Arc::new(MemoryRegistry::new());
        let runtime = TokioRuntime;
        let transport = Arc::new(MemoryTransport::new(node_id.clone(), addr, runtime.clone()));

        let cluster = Cluster::new(node, config, registry, transport, runtime);

        cluster.start().await.unwrap();
        assert!(cluster.is_running().await);

        cluster.stop().await.unwrap();
        assert_eq!(cluster.state().await, ClusterState::Stopped);
    }

    #[tokio::test]
    async fn test_cluster_list_nodes() {
        let node_id = test_node_id(1);
        let addr = test_addr(9001);
        let mut node = NodeInfo::with_timestamp(node_id.clone(), addr, 1000);
        node.status = NodeStatus::Active;

        let config = ClusterConfig::for_testing();
        let registry = Arc::new(MemoryRegistry::new());
        let runtime = TokioRuntime;
        let transport = Arc::new(MemoryTransport::new(node_id.clone(), addr, runtime.clone()));

        let cluster = Cluster::new(node, config, registry, transport, runtime);
        cluster.start().await.unwrap();

        let nodes = cluster.list_nodes().await.unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, node_id);

        cluster.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_cluster_try_claim() {
        let node_id = test_node_id(1);
        let addr = test_addr(9001);
        let mut node = NodeInfo::with_timestamp(node_id.clone(), addr, 1000);
        node.status = NodeStatus::Active;

        let config = ClusterConfig::for_testing();
        let registry = Arc::new(MemoryRegistry::new());
        let runtime = TokioRuntime;
        let transport = Arc::new(MemoryTransport::new(node_id.clone(), addr, runtime.clone()));

        let cluster = Cluster::new(node, config, registry, transport, runtime);
        cluster.start().await.unwrap();

        let actor_id = ActorId::new("test", "actor-1").unwrap();
        let decision = cluster.try_claim_local(actor_id.clone()).await.unwrap();

        assert!(matches!(decision, PlacementDecision::New(_)));

        // Second claim should return existing
        let decision2 = cluster.try_claim_local(actor_id).await.unwrap();
        assert!(matches!(decision2, PlacementDecision::Existing(_)));

        cluster.stop().await.unwrap();
    }
}
