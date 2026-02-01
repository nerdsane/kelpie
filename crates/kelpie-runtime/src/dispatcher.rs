//! Message dispatcher for actor runtime
//!
//! TigerStyle: Single-threaded per-actor execution, explicit message routing.

use crate::activation::ActiveActor;
use async_trait::async_trait;
use bytes::Bytes;
use kelpie_core::actor::{Actor, ActorId};
use kelpie_core::constants::{ACTOR_CONCURRENT_COUNT_MAX, INVOCATION_PENDING_COUNT_MAX};
use kelpie_core::error::{Error, Result};
use kelpie_core::io::{TimeProvider, WallClockTime};
use kelpie_core::metrics;
use kelpie_registry::{NodeId, PlacementDecision, Registry};
use kelpie_storage::ActorKV;
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, instrument, warn};

// ============================================================================
// Request Forwarding
// ============================================================================

/// Trait for forwarding requests to other nodes
///
/// Implementations should use RpcTransport to send ActorInvoke messages.
#[async_trait]
pub trait RequestForwarder: Send + Sync {
    /// Forward an invocation to another node
    ///
    /// Returns the result from the remote node.
    async fn forward(
        &self,
        target_node: &NodeId,
        actor_id: &ActorId,
        operation: &str,
        payload: Bytes,
    ) -> Result<Bytes>;
}

// ============================================================================
// Dispatcher Config
// ============================================================================

/// Configuration for the dispatcher
#[derive(Debug, Clone)]
pub struct DispatcherConfig {
    /// Maximum number of concurrent actors
    pub max_actors: usize,
    /// Maximum pending messages per actor
    pub max_pending_per_actor: usize,
    /// Channel buffer size for dispatcher commands
    pub command_buffer_size: usize,
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            max_actors: ACTOR_CONCURRENT_COUNT_MAX,
            max_pending_per_actor: INVOCATION_PENDING_COUNT_MAX,
            command_buffer_size: 1024,
        }
    }
}

/// Commands sent to the dispatcher
#[derive(Debug)]
pub enum DispatcherCommand {
    /// Invoke an actor
    Invoke {
        actor_id: ActorId,
        operation: String,
        payload: Bytes,
        reply_tx: oneshot::Sender<Result<Bytes>>,
    },
    /// Deactivate an actor
    Deactivate { actor_id: ActorId },
    /// Shutdown the dispatcher
    Shutdown,
}

/// Guard that decrements a counter on drop
struct PendingGuard {
    counter: Arc<AtomicUsize>,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Handle to send commands to the dispatcher
#[derive(Clone)]
pub struct DispatcherHandle<R: kelpie_core::Runtime> {
    command_tx: mpsc::Sender<DispatcherCommand>,
    #[allow(dead_code)]
    runtime: R,
    /// Pending invocation count per actor (for backpressure)
    pending_counts: Arc<Mutex<HashMap<String, Arc<AtomicUsize>>>>,
    /// Maximum pending invocations per actor
    max_pending_per_actor: usize,
}

impl<R: kelpie_core::Runtime> DispatcherHandle<R> {
    /// Invoke an actor
    ///
    /// Returns an error if the actor has too many pending invocations.
    pub async fn invoke(
        &self,
        actor_id: ActorId,
        operation: String,
        payload: Bytes,
    ) -> Result<Bytes> {
        let key = actor_id.qualified_name();

        // Get or create the pending counter for this actor
        let counter = {
            let mut counts = self.pending_counts.lock().unwrap();
            counts
                .entry(key.clone())
                .or_insert_with(|| Arc::new(AtomicUsize::new(0)))
                .clone()
        };

        // Increment and check limit
        let current = counter.fetch_add(1, Ordering::SeqCst);
        if current >= self.max_pending_per_actor {
            // Over limit - decrement and reject
            counter.fetch_sub(1, Ordering::SeqCst);
            return Err(Error::Internal {
                message: format!(
                    "actor {} has too many pending invocations: {} >= {}",
                    key, current, self.max_pending_per_actor
                ),
            });
        }

        // Create guard to decrement on completion (success or failure)
        let _guard = PendingGuard {
            counter: counter.clone(),
        };

        let (reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(DispatcherCommand::Invoke {
                actor_id: actor_id.clone(),
                operation,
                payload,
                reply_tx,
            })
            .await
            .map_err(|_| Error::Internal {
                message: "dispatcher channel closed".into(),
            })?;

        reply_rx.await.map_err(|_| Error::Internal {
            message: "reply channel closed".into(),
        })?
    }

    /// Deactivate an actor
    pub async fn deactivate(&self, actor_id: ActorId) -> Result<()> {
        self.command_tx
            .send(DispatcherCommand::Deactivate { actor_id })
            .await
            .map_err(|_| Error::Internal {
                message: "dispatcher channel closed".into(),
            })
    }

    /// Shutdown the dispatcher
    pub async fn shutdown(&self) -> Result<()> {
        self.command_tx
            .send(DispatcherCommand::Shutdown)
            .await
            .map_err(|_| Error::Internal {
                message: "dispatcher channel closed".into(),
            })
    }
}

/// Factory for creating actors
pub trait ActorFactory<A>: Send + Sync + 'static
where
    A: Actor,
{
    /// Create a new actor instance
    fn create(&self, id: &ActorId) -> A;
}

/// Simple factory that clones a prototype actor
pub struct CloneFactory<A: Clone + Send + Sync + 'static> {
    prototype: A,
}

impl<A: Clone + Send + Sync + 'static> CloneFactory<A> {
    /// Create a new clone factory
    pub fn new(prototype: A) -> Self {
        Self { prototype }
    }
}

impl<A> ActorFactory<A> for CloneFactory<A>
where
    A: Actor + Clone,
{
    fn create(&self, _id: &ActorId) -> A {
        self.prototype.clone()
    }
}

/// Dispatcher for routing messages to actors
///
/// Manages actor lifecycle and message routing.
/// Optionally integrates with a distributed registry for single-activation guarantee.
pub struct Dispatcher<A, S, R>
where
    A: Actor<State = S>,
    S: Serialize + DeserializeOwned + Default + Send + Sync,
    R: kelpie_core::Runtime,
{
    /// Actor factory
    factory: Arc<dyn ActorFactory<A>>,
    /// KV store for persistence
    kv: Arc<dyn ActorKV>,
    /// Configuration
    config: DispatcherConfig,
    /// Runtime for spawning tasks
    runtime: R,
    /// Time provider for DST compatibility
    time: Arc<dyn TimeProvider>,
    /// Active actors
    actors: HashMap<String, ActiveActor<A, S>>,
    /// Command receiver
    command_rx: mpsc::Receiver<DispatcherCommand>,
    /// Command sender (for creating handles)
    command_tx: mpsc::Sender<DispatcherCommand>,
    /// Pending invocation counts (shared with handles)
    pending_counts: Arc<Mutex<HashMap<String, Arc<AtomicUsize>>>>,
    /// Optional distributed registry for coordination
    registry: Option<Arc<dyn Registry>>,
    /// Node ID for this dispatcher (required when registry is set)
    node_id: Option<NodeId>,
    /// Optional request forwarder for distributed mode
    forwarder: Option<Arc<dyn RequestForwarder>>,
}

impl<A, S, R> Dispatcher<A, S, R>
where
    A: Actor<State = S>,
    S: Serialize + DeserializeOwned + Default + Send + Sync + Clone + 'static,
    R: kelpie_core::Runtime,
{
    /// Create a new dispatcher (local mode without registry)
    ///
    /// Uses production wall clock. For DST, use `with_time`.
    pub fn new(
        factory: Arc<dyn ActorFactory<A>>,
        kv: Arc<dyn ActorKV>,
        config: DispatcherConfig,
        runtime: R,
    ) -> Self {
        Self::with_time(factory, kv, config, runtime, Arc::new(WallClockTime::new()))
    }

    /// Create a new dispatcher with custom time provider (for DST)
    pub fn with_time(
        factory: Arc<dyn ActorFactory<A>>,
        kv: Arc<dyn ActorKV>,
        config: DispatcherConfig,
        runtime: R,
        time: Arc<dyn TimeProvider>,
    ) -> Self {
        let (command_tx, command_rx) = mpsc::channel(config.command_buffer_size);

        Self {
            factory,
            kv,
            config,
            runtime: runtime.clone(),
            time,
            actors: HashMap::new(),
            command_rx,
            command_tx,
            pending_counts: Arc::new(Mutex::new(HashMap::new())),
            registry: None,
            node_id: None,
            forwarder: None,
        }
    }

    /// Create a new dispatcher with registry integration (distributed mode)
    ///
    /// In distributed mode, the dispatcher will:
    /// - Claim actors in the registry before local activation
    /// - Release actors from the registry on deactivation
    /// - Respect single-activation guarantees
    /// - Forward requests to other nodes when forwarder is provided
    pub fn with_registry(
        factory: Arc<dyn ActorFactory<A>>,
        kv: Arc<dyn ActorKV>,
        config: DispatcherConfig,
        runtime: R,
        registry: Arc<dyn Registry>,
        node_id: NodeId,
    ) -> Self {
        Self::with_registry_and_time(
            factory,
            kv,
            config,
            runtime,
            registry,
            node_id,
            Arc::new(WallClockTime::new()),
        )
    }

    /// Create a new dispatcher with registry and custom time provider (for DST)
    pub fn with_registry_and_time(
        factory: Arc<dyn ActorFactory<A>>,
        kv: Arc<dyn ActorKV>,
        config: DispatcherConfig,
        runtime: R,
        registry: Arc<dyn Registry>,
        node_id: NodeId,
        time: Arc<dyn TimeProvider>,
    ) -> Self {
        let (command_tx, command_rx) = mpsc::channel(config.command_buffer_size);

        Self {
            factory,
            kv,
            config,
            runtime: runtime.clone(),
            time,
            actors: HashMap::new(),
            command_rx,
            command_tx,
            pending_counts: Arc::new(Mutex::new(HashMap::new())),
            registry: Some(registry),
            node_id: Some(node_id),
            forwarder: None,
        }
    }

    /// Set a request forwarder for distributed mode
    ///
    /// When set, requests for actors on other nodes will be forwarded
    /// instead of returning an error.
    pub fn with_forwarder(mut self, forwarder: Arc<dyn RequestForwarder>) -> Self {
        self.forwarder = Some(forwarder);
        self
    }

    /// Get a handle to the dispatcher
    pub fn handle(&self) -> DispatcherHandle<R> {
        DispatcherHandle {
            command_tx: self.command_tx.clone(),
            runtime: self.runtime.clone(),
            pending_counts: self.pending_counts.clone(),
            max_pending_per_actor: self.config.max_pending_per_actor,
        }
    }

    /// Run the dispatcher loop
    #[instrument(skip(self), level = "info")]
    pub async fn run(&mut self) {
        info!("Dispatcher starting");

        while let Some(command) = self.command_rx.recv().await {
            match command {
                DispatcherCommand::Invoke {
                    actor_id,
                    operation,
                    payload,
                    reply_tx,
                } => {
                    let result = self
                        .handle_invoke(actor_id.clone(), &operation, payload)
                        .await;
                    let _ = reply_tx.send(result);
                }
                DispatcherCommand::Deactivate { actor_id } => {
                    self.handle_deactivate(&actor_id).await;
                }
                DispatcherCommand::Shutdown => {
                    info!("Dispatcher shutting down");
                    self.shutdown().await;
                    break;
                }
            }
        }

        info!("Dispatcher stopped");
    }

    /// Handle an invoke command
    ///
    /// In distributed mode, this will:
    /// 1. Check if actor is locally active
    /// 2. If not, check registry for placement
    /// 3. If on another node, forward the request (if forwarder available)
    /// 4. If on this node or new, activate locally and process
    #[instrument(skip(self, payload), fields(actor_id = %actor_id, operation), level = "debug")]
    async fn handle_invoke(
        &mut self,
        actor_id: ActorId,
        operation: &str,
        payload: Bytes,
    ) -> Result<Bytes> {
        // Use time provider for DST determinism
        let start_ms = self.time.monotonic_ms();
        let key = actor_id.qualified_name();

        // Check if actor is locally active
        if !self.actors.contains_key(&key) {
            // In distributed mode, check if actor is on another node
            if let (Some(registry), Some(node_id)) = (&self.registry, &self.node_id) {
                // Check existing placement without claiming
                if let Ok(Some(placement)) = registry.get_placement(&actor_id).await {
                    if &placement.node_id != node_id {
                        // Actor is on another node - forward if we have a forwarder
                        if let Some(forwarder) = &self.forwarder {
                            debug!(
                                actor_id = %actor_id,
                                target_node = %placement.node_id,
                                "Forwarding request to remote node"
                            );
                            let result = forwarder
                                .forward(&placement.node_id, &actor_id, operation, payload)
                                .await;

                            // Record metrics for forwarded request
                            let duration_ms = self.time.monotonic_ms().saturating_sub(start_ms);
                            let duration = duration_ms as f64 / 1000.0;
                            let status = if result.is_ok() {
                                "forwarded"
                            } else {
                                "forward_error"
                            };
                            metrics::record_invocation(operation, status, duration);

                            return result;
                        } else {
                            // No forwarder available - return error with owner info
                            warn!(
                                actor_id = %actor_id,
                                owner_node = %placement.node_id,
                                "Actor on another node, no forwarder configured"
                            );
                            return Err(Error::ActorNotFound {
                                id: format!(
                                    "{} (owned by {}, forwarding not configured)",
                                    actor_id.qualified_name(),
                                    placement.node_id
                                ),
                            });
                        }
                    }
                }
            }

            // Actor not on another node (or no registry) - activate locally
            self.activate_actor(actor_id.clone()).await?;
        }

        // Get the active actor
        let active = self.actors.get_mut(&key).ok_or_else(|| Error::Internal {
            message: "actor not found after activation".into(),
        })?;

        // Process the invocation
        let result = active
            .process_invocation_with_time(operation, payload, self.time.clone())
            .await;

        // Record metrics
        let duration_ms = self.time.monotonic_ms().saturating_sub(start_ms);
        let duration = duration_ms as f64 / 1000.0;
        let status = if result.is_ok() { "success" } else { "error" };
        metrics::record_invocation(operation, status, duration);

        result
    }

    /// Activate an actor
    ///
    /// In distributed mode, claims the actor in the registry first.
    /// Returns an error if the actor is already activated on another node.
    async fn activate_actor(&mut self, actor_id: ActorId) -> Result<()> {
        let key = actor_id.qualified_name();

        // Check capacity
        if self.actors.len() >= self.config.max_actors {
            return Err(Error::Internal {
                message: format!("maximum actor count reached: {}", self.config.max_actors),
            });
        }

        // In distributed mode, claim the actor via the registry first
        if let (Some(registry), Some(node_id)) = (&self.registry, &self.node_id) {
            let decision = registry
                .try_claim_actor(actor_id.clone(), node_id.clone())
                .await
                .map_err(|e| Error::Internal {
                    message: format!("registry claim failed: {}", e),
                })?;

            match decision {
                PlacementDecision::New(claimed_node) => {
                    debug!(
                        actor_id = %actor_id,
                        node_id = %claimed_node,
                        "Actor claimed in registry"
                    );
                }
                PlacementDecision::Existing(placement) => {
                    // Actor is already placed somewhere
                    if &placement.node_id != node_id {
                        // Actor is on a different node - cannot activate here
                        // Note: Forwarding is handled in handle_invoke before calling activate_actor
                        // This branch handles the race condition where placement changed between
                        // get_placement() and try_claim_actor()
                        warn!(
                            actor_id = %actor_id,
                            owner_node = %placement.node_id,
                            "Actor claimed by another node during activation"
                        );
                        return Err(Error::ActorNotFound {
                            id: format!(
                                "{} (owned by {})",
                                actor_id.qualified_name(),
                                placement.node_id
                            ),
                        });
                    }
                    // Already owned by this node, proceed with local activation
                    debug!(actor_id = %actor_id, "Actor already claimed by this node");
                }
                PlacementDecision::NoCapacity => {
                    return Err(Error::Internal {
                        message: "no node has capacity for actor".into(),
                    });
                }
            }
        }

        // Create and activate the actor locally
        let actor = self.factory.create(&actor_id);
        let active = ActiveActor::activate(actor_id.clone(), actor, self.kv.clone()).await?;

        self.actors.insert(key, active);
        debug!(actor_id = %actor_id, "Actor activated locally");

        // Record activation metric
        metrics::record_agent_activated();

        Ok(())
    }

    /// Handle a deactivate command
    ///
    /// In distributed mode, releases the actor from the registry after local deactivation.
    async fn handle_deactivate(&mut self, actor_id: &ActorId) {
        let key = actor_id.qualified_name();

        if let Some(mut active) = self.actors.remove(&key) {
            if let Err(e) = active.deactivate().await {
                error!(actor_id = %actor_id, error = %e, "Failed to deactivate actor");
            } else {
                debug!(actor_id = %actor_id, "Actor deactivated locally");

                // In distributed mode, release from registry
                if let Some(registry) = &self.registry {
                    if let Err(e) = registry.unregister_actor(actor_id).await {
                        error!(
                            actor_id = %actor_id,
                            error = %e,
                            "Failed to unregister actor from registry"
                        );
                    } else {
                        debug!(actor_id = %actor_id, "Actor released from registry");
                    }
                }

                // Record deactivation metric
                metrics::record_agent_deactivated();
            }
        }
    }

    /// Shutdown all actors
    ///
    /// In distributed mode, releases all actors from the registry.
    async fn shutdown(&mut self) {
        let actor_ids: Vec<_> = self.actors.keys().cloned().collect();

        for key in actor_ids {
            if let Some(mut active) = self.actors.remove(&key) {
                let actor_id = active.id.clone();

                if let Err(e) = active.deactivate().await {
                    error!(error = %e, "Failed to deactivate actor during shutdown");
                }

                // In distributed mode, release from registry
                if let Some(registry) = &self.registry {
                    if let Err(e) = registry.unregister_actor(&actor_id).await {
                        error!(
                            actor_id = %actor_id,
                            error = %e,
                            "Failed to unregister actor from registry during shutdown"
                        );
                    }
                }
            }
        }
    }

    /// Get the number of active actors
    pub fn active_actor_count(&self) -> usize {
        self.actors.len()
    }

    /// Check if an actor is active
    pub fn is_active(&self, actor_id: &ActorId) -> bool {
        self.actors.contains_key(&actor_id.qualified_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use kelpie_core::actor::ActorContext;
    use kelpie_core::Runtime;
    use kelpie_storage::MemoryKV;

    #[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
    struct CounterState {
        count: i64,
    }

    #[derive(Clone)]
    struct CounterActor;

    #[async_trait]
    impl Actor for CounterActor {
        type State = CounterState;

        async fn invoke(
            &self,
            ctx: &mut ActorContext<Self::State>,
            operation: &str,
            _payload: Bytes,
        ) -> Result<Bytes> {
            match operation {
                "increment" => {
                    ctx.state.count += 1;
                    Ok(Bytes::from(ctx.state.count.to_string()))
                }
                "get" => Ok(Bytes::from(ctx.state.count.to_string())),
                _ => Err(Error::InvalidOperation {
                    operation: operation.to_string(),
                }),
            }
        }
    }

    #[tokio::test]
    async fn test_dispatcher_basic() {
        use kelpie_core::TokioRuntime;

        let factory = Arc::new(CloneFactory::new(CounterActor));
        let kv = Arc::new(MemoryKV::new());
        let config = DispatcherConfig::default();
        let runtime = TokioRuntime;

        let mut dispatcher = Dispatcher::new(factory, kv, config, runtime.clone());
        let handle = dispatcher.handle();

        // Run dispatcher in background
        let dispatcher_task = runtime.spawn(async move {
            dispatcher.run().await;
        });

        // Invoke actor
        let actor_id = ActorId::new("test", "counter-1").unwrap();
        let result = handle
            .invoke(actor_id.clone(), "increment".to_string(), Bytes::new())
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("1"));

        let result = handle
            .invoke(actor_id.clone(), "get".to_string(), Bytes::new())
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("1"));

        // Shutdown
        handle.shutdown().await.unwrap();
        dispatcher_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_dispatcher_multiple_actors() {
        use kelpie_core::TokioRuntime;

        let factory = Arc::new(CloneFactory::new(CounterActor));
        let kv = Arc::new(MemoryKV::new());
        let config = DispatcherConfig::default();
        let runtime = TokioRuntime;

        let mut dispatcher = Dispatcher::new(factory, kv, config, runtime.clone());
        let handle = dispatcher.handle();

        let dispatcher_task = runtime.spawn(async move {
            dispatcher.run().await;
        });

        let actor1 = ActorId::new("test", "counter-1").unwrap();
        let actor2 = ActorId::new("test", "counter-2").unwrap();

        // Increment actor1 twice
        handle
            .invoke(actor1.clone(), "increment".to_string(), Bytes::new())
            .await
            .unwrap();
        handle
            .invoke(actor1.clone(), "increment".to_string(), Bytes::new())
            .await
            .unwrap();

        // Increment actor2 once
        handle
            .invoke(actor2.clone(), "increment".to_string(), Bytes::new())
            .await
            .unwrap();

        // Verify counts are independent
        let result1 = handle
            .invoke(actor1.clone(), "get".to_string(), Bytes::new())
            .await
            .unwrap();
        assert_eq!(result1, Bytes::from("2"));

        let result2 = handle
            .invoke(actor2.clone(), "get".to_string(), Bytes::new())
            .await
            .unwrap();
        assert_eq!(result2, Bytes::from("1"));

        handle.shutdown().await.unwrap();
        dispatcher_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_dispatcher_deactivate() {
        use kelpie_core::TokioRuntime;

        let factory = Arc::new(CloneFactory::new(CounterActor));
        let kv = Arc::new(MemoryKV::new());
        let config = DispatcherConfig::default();
        let runtime = TokioRuntime;

        let mut dispatcher = Dispatcher::new(factory, kv.clone(), config, runtime.clone());
        let handle = dispatcher.handle();

        let dispatcher_task = runtime.spawn(async move {
            dispatcher.run().await;
        });

        let actor_id = ActorId::new("test", "counter-deactivate").unwrap();

        // Increment
        handle
            .invoke(actor_id.clone(), "increment".to_string(), Bytes::new())
            .await
            .unwrap();
        handle
            .invoke(actor_id.clone(), "increment".to_string(), Bytes::new())
            .await
            .unwrap();

        // Deactivate
        handle.deactivate(actor_id.clone()).await.unwrap();

        // Allow time for deactivation
        runtime.sleep(std::time::Duration::from_millis(10)).await;

        // Invoke again - should reactivate with persisted state
        let result = handle
            .invoke(actor_id.clone(), "get".to_string(), Bytes::new())
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("2"));

        handle.shutdown().await.unwrap();
        dispatcher_task.await.unwrap();
    }

    // =========================================================================
    // Pending Invocation Limit Tests
    // =========================================================================

    #[tokio::test]
    async fn test_dispatcher_max_pending_per_actor() {
        use kelpie_core::TokioRuntime;

        let factory = Arc::new(CloneFactory::new(CounterActor));
        let kv = Arc::new(MemoryKV::new());
        let config = DispatcherConfig {
            max_pending_per_actor: 2, // Low limit for testing
            ..Default::default()
        };
        let runtime = TokioRuntime;

        let mut dispatcher = Dispatcher::new(factory, kv, config, runtime.clone());
        let handle = dispatcher.handle();

        let dispatcher_task = runtime.spawn(async move {
            dispatcher.run().await;
        });

        let actor_id = ActorId::new("test", "pending-limit").unwrap();

        // First invocation - should succeed
        let result1 = handle
            .invoke(actor_id.clone(), "increment".to_string(), Bytes::new())
            .await;
        assert!(result1.is_ok());

        // Sequential invocations are fine (each completes before next starts)
        let result2 = handle
            .invoke(actor_id.clone(), "get".to_string(), Bytes::new())
            .await;
        assert!(result2.is_ok());

        handle.shutdown().await.unwrap();
        dispatcher_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_dispatcher_max_pending_concurrent() {
        use kelpie_core::TokioRuntime;

        let factory = Arc::new(CloneFactory::new(CounterActor));
        let kv = Arc::new(MemoryKV::new());
        let config = DispatcherConfig {
            max_pending_per_actor: 2, // Low limit for testing
            ..Default::default()
        };
        let runtime = TokioRuntime;

        let mut dispatcher = Dispatcher::new(factory, kv, config, runtime.clone());
        let handle = dispatcher.handle();

        let dispatcher_task = runtime.spawn(async move {
            dispatcher.run().await;
        });

        let actor_id = ActorId::new("test", "pending-concurrent").unwrap();

        // Spawn multiple concurrent invocations
        let handle1 = handle.clone();
        let handle2 = handle.clone();
        let handle3 = handle.clone();
        let id1 = actor_id.clone();
        let id2 = actor_id.clone();
        let id3 = actor_id.clone();

        let f1 = runtime.spawn(async move {
            handle1
                .invoke(id1, "increment".to_string(), Bytes::new())
                .await
        });
        let f2 = runtime.spawn(async move {
            handle2
                .invoke(id2, "increment".to_string(), Bytes::new())
                .await
        });
        let f3 = runtime.spawn(async move {
            handle3
                .invoke(id3, "increment".to_string(), Bytes::new())
                .await
        });

        let r1 = f1.await.unwrap();
        let r2 = f2.await.unwrap();
        let r3 = f3.await.unwrap();

        // At least one should fail due to the limit of 2
        let failures = [&r1, &r2, &r3].iter().filter(|r| r.is_err()).count();
        let successes = [&r1, &r2, &r3].iter().filter(|r| r.is_ok()).count();

        // With limit of 2, we expect at least 1 failure when 3 concurrent requests arrive
        assert!(
            failures >= 1,
            "Expected at least 1 failure with limit 2 and 3 concurrent requests, got {} failures",
            failures
        );
        assert!(
            successes >= 1,
            "Expected at least 1 success, got {}",
            successes
        );

        handle.shutdown().await.unwrap();
        dispatcher_task.await.unwrap();
    }

    // =========================================================================
    // Distributed Activation Tests
    // =========================================================================

    #[tokio::test]
    async fn test_dispatcher_with_registry_single_node() {
        use kelpie_core::TokioRuntime;
        use kelpie_registry::MemoryRegistry;
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        let factory = Arc::new(CloneFactory::new(CounterActor));
        let kv = Arc::new(MemoryKV::new());
        let config = DispatcherConfig::default();
        let runtime = TokioRuntime;

        // Create registry and register this node
        let registry = Arc::new(MemoryRegistry::new());
        let node_id = NodeId::new("node-1").unwrap();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);
        let mut info = kelpie_registry::NodeInfo::new(node_id.clone(), addr);
        info.status = kelpie_registry::NodeStatus::Active;
        registry.register_node(info).await.unwrap();

        // Create dispatcher with registry
        let mut dispatcher = Dispatcher::with_registry(
            factory,
            kv,
            config,
            runtime.clone(),
            registry.clone(),
            node_id.clone(),
        );
        let handle = dispatcher.handle();

        let dispatcher_task = runtime.spawn(async move {
            dispatcher.run().await;
        });

        // Invoke actor - should claim in registry and activate
        let actor_id = ActorId::new("test", "counter-reg-1").unwrap();
        let result = handle
            .invoke(actor_id.clone(), "increment".to_string(), Bytes::new())
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("1"));

        // Verify actor is registered in the registry
        let placement = registry.get_placement(&actor_id).await.unwrap();
        assert!(placement.is_some());
        assert_eq!(placement.unwrap().node_id, node_id);

        handle.shutdown().await.unwrap();
        dispatcher_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_dispatcher_distributed_single_activation() {
        use kelpie_core::TokioRuntime;
        use kelpie_registry::MemoryRegistry;
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        let factory = Arc::new(CloneFactory::new(CounterActor));
        let kv = Arc::new(MemoryKV::new());
        let config = DispatcherConfig::default();
        let runtime = TokioRuntime;

        // Create shared registry
        let registry = Arc::new(MemoryRegistry::new());

        // Register node 1
        let node1_id = NodeId::new("node-1").unwrap();
        let addr1 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);
        let mut info1 = kelpie_registry::NodeInfo::new(node1_id.clone(), addr1);
        info1.status = kelpie_registry::NodeStatus::Active;
        registry.register_node(info1).await.unwrap();

        // Register node 2
        let node2_id = NodeId::new("node-2").unwrap();
        let addr2 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8081);
        let mut info2 = kelpie_registry::NodeInfo::new(node2_id.clone(), addr2);
        info2.status = kelpie_registry::NodeStatus::Active;
        registry.register_node(info2).await.unwrap();

        // Create dispatcher 1
        let mut dispatcher1 = Dispatcher::with_registry(
            factory.clone(),
            kv.clone(),
            config.clone(),
            runtime.clone(),
            registry.clone(),
            node1_id.clone(),
        );
        let handle1 = dispatcher1.handle();

        // Create dispatcher 2
        let mut dispatcher2 = Dispatcher::with_registry(
            factory.clone(),
            kv.clone(),
            config.clone(),
            runtime.clone(),
            registry.clone(),
            node2_id.clone(),
        );
        let handle2 = dispatcher2.handle();

        let dispatcher1_task = runtime.spawn(async move {
            dispatcher1.run().await;
        });

        let dispatcher2_task = runtime.spawn(async move {
            dispatcher2.run().await;
        });

        // Node 1 claims the actor first
        let actor_id = ActorId::new("test", "contested-actor").unwrap();
        let result1 = handle1
            .invoke(actor_id.clone(), "increment".to_string(), Bytes::new())
            .await;
        assert!(
            result1.is_ok(),
            "Node 1 should successfully claim the actor"
        );
        assert_eq!(result1.unwrap(), Bytes::from("1"));

        // Node 2 tries to activate the same actor - should fail
        let result2 = handle2
            .invoke(actor_id.clone(), "increment".to_string(), Bytes::new())
            .await;
        assert!(
            result2.is_err(),
            "Node 2 should fail to activate actor owned by node 1"
        );

        // Verify the error message indicates the actor is on another node
        let err = result2.unwrap_err();
        let err_msg = format!("{}", err);
        assert!(
            err_msg.contains("node-1"),
            "Error should mention the owning node"
        );

        // Verify actor is registered to node 1
        let placement = registry.get_placement(&actor_id).await.unwrap();
        assert!(placement.is_some());
        assert_eq!(placement.unwrap().node_id, node1_id);

        handle1.shutdown().await.unwrap();
        handle2.shutdown().await.unwrap();
        dispatcher1_task.await.unwrap();
        dispatcher2_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_dispatcher_deactivate_releases_from_registry() {
        use kelpie_core::TokioRuntime;
        use kelpie_registry::MemoryRegistry;
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        let factory = Arc::new(CloneFactory::new(CounterActor));
        let kv = Arc::new(MemoryKV::new());
        let config = DispatcherConfig::default();
        let runtime = TokioRuntime;

        // Create registry and register this node
        let registry = Arc::new(MemoryRegistry::new());
        let node_id = NodeId::new("node-1").unwrap();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);
        let mut info = kelpie_registry::NodeInfo::new(node_id.clone(), addr);
        info.status = kelpie_registry::NodeStatus::Active;
        registry.register_node(info).await.unwrap();

        // Create dispatcher with registry
        let mut dispatcher = Dispatcher::with_registry(
            factory,
            kv,
            config,
            runtime.clone(),
            registry.clone(),
            node_id.clone(),
        );
        let handle = dispatcher.handle();

        let dispatcher_task = runtime.spawn(async move {
            dispatcher.run().await;
        });

        // Activate actor
        let actor_id = ActorId::new("test", "counter-release").unwrap();
        handle
            .invoke(actor_id.clone(), "increment".to_string(), Bytes::new())
            .await
            .unwrap();

        // Verify actor is in registry
        let placement = registry.get_placement(&actor_id).await.unwrap();
        assert!(placement.is_some());

        // Deactivate
        handle.deactivate(actor_id.clone()).await.unwrap();
        runtime.sleep(std::time::Duration::from_millis(10)).await;

        // Verify actor is no longer in registry
        let placement = registry.get_placement(&actor_id).await.unwrap();
        assert!(
            placement.is_none(),
            "Actor should be unregistered after deactivation"
        );

        handle.shutdown().await.unwrap();
        dispatcher_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_dispatcher_shutdown_releases_all_from_registry() {
        use kelpie_core::TokioRuntime;
        use kelpie_registry::MemoryRegistry;
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        let factory = Arc::new(CloneFactory::new(CounterActor));
        let kv = Arc::new(MemoryKV::new());
        let config = DispatcherConfig::default();
        let runtime = TokioRuntime;

        // Create registry and register this node
        let registry = Arc::new(MemoryRegistry::new());
        let node_id = NodeId::new("node-1").unwrap();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);
        let mut info = kelpie_registry::NodeInfo::new(node_id.clone(), addr);
        info.status = kelpie_registry::NodeStatus::Active;
        registry.register_node(info).await.unwrap();

        // Create dispatcher with registry
        let mut dispatcher = Dispatcher::with_registry(
            factory,
            kv,
            config,
            runtime.clone(),
            registry.clone(),
            node_id.clone(),
        );
        let handle = dispatcher.handle();

        let dispatcher_task = runtime.spawn(async move {
            dispatcher.run().await;
        });

        // Activate multiple actors
        let actor1 = ActorId::new("test", "multi-1").unwrap();
        let actor2 = ActorId::new("test", "multi-2").unwrap();
        let actor3 = ActorId::new("test", "multi-3").unwrap();

        handle
            .invoke(actor1.clone(), "increment".to_string(), Bytes::new())
            .await
            .unwrap();
        handle
            .invoke(actor2.clone(), "increment".to_string(), Bytes::new())
            .await
            .unwrap();
        handle
            .invoke(actor3.clone(), "increment".to_string(), Bytes::new())
            .await
            .unwrap();

        // Verify all actors are in registry
        assert!(registry.get_placement(&actor1).await.unwrap().is_some());
        assert!(registry.get_placement(&actor2).await.unwrap().is_some());
        assert!(registry.get_placement(&actor3).await.unwrap().is_some());

        // Shutdown
        handle.shutdown().await.unwrap();
        dispatcher_task.await.unwrap();

        // Allow time for cleanup
        runtime.sleep(std::time::Duration::from_millis(10)).await;

        // Verify all actors are released from registry
        assert!(
            registry.get_placement(&actor1).await.unwrap().is_none(),
            "Actor 1 should be released after shutdown"
        );
        assert!(
            registry.get_placement(&actor2).await.unwrap().is_none(),
            "Actor 2 should be released after shutdown"
        );
        assert!(
            registry.get_placement(&actor3).await.unwrap().is_none(),
            "Actor 3 should be released after shutdown"
        );
    }
}
