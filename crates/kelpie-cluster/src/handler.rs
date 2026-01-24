//! RPC message handler
//!
//! TigerStyle: Explicit message handling with bounded state.

use crate::rpc::{RpcHandler, RpcMessage};
use async_trait::async_trait;
use bytes::Bytes;
use kelpie_core::actor::ActorId;
use kelpie_registry::{NodeId, PlacementDecision, Registry};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Callback for invoking actors locally
#[async_trait]
pub trait ActorInvoker: Send + Sync {
    /// Invoke an actor operation
    async fn invoke(
        &self,
        actor_id: ActorId,
        operation: String,
        payload: Bytes,
    ) -> Result<Bytes, String>;
}

/// Callback for activating an actor with migrated state
#[async_trait]
pub trait MigrationReceiver: Send + Sync {
    /// Check if we can accept a migrated actor
    async fn can_accept(&self, actor_id: &ActorId) -> Result<bool, String>;

    /// Receive migrated actor state (store temporarily)
    async fn receive_state(&self, actor_id: ActorId, state: Bytes) -> Result<(), String>;

    /// Activate the actor with the received state
    async fn activate_migrated(&self, actor_id: ActorId) -> Result<(), String>;
}

/// Pending migration state
#[derive(Debug)]
struct PendingMigration {
    /// The actor being migrated
    #[allow(dead_code)]
    actor_id: ActorId,
    /// Source node
    from_node: NodeId,
    /// The actor state
    state: Option<Bytes>,
    /// Started at timestamp
    #[allow(dead_code)]
    started_at_ms: u64,
}

/// Handler for incoming cluster RPC messages
pub struct ClusterRpcHandler<R: Registry> {
    /// Local node ID
    local_node_id: NodeId,
    /// Registry for placement
    registry: Arc<R>,
    /// Actor invoker for local invocations
    invoker: Arc<dyn ActorInvoker>,
    /// Migration receiver for handling incoming migrations
    migration_receiver: Arc<dyn MigrationReceiver>,
    /// Pending migrations (actor_id -> state waiting to be activated)
    pending_migrations: RwLock<HashMap<ActorId, PendingMigration>>,
}

impl<R: Registry> ClusterRpcHandler<R> {
    /// Create a new cluster RPC handler
    pub fn new(
        local_node_id: NodeId,
        registry: Arc<R>,
        invoker: Arc<dyn ActorInvoker>,
        migration_receiver: Arc<dyn MigrationReceiver>,
    ) -> Self {
        Self {
            local_node_id,
            registry,
            invoker,
            migration_receiver,
            pending_migrations: RwLock::new(HashMap::new()),
        }
    }

    /// Handle actor invocation request
    async fn handle_actor_invoke(
        &self,
        request_id: u64,
        actor_id: ActorId,
        operation: String,
        payload: Bytes,
    ) -> RpcMessage {
        debug!(
            request_id = request_id,
            actor_id = %actor_id,
            operation = %operation,
            "handling actor invoke"
        );

        // Check if actor is on this node
        match self.registry.get_placement(&actor_id).await {
            Ok(Some(placement)) => {
                if placement.node_id != self.local_node_id {
                    // Actor is not on this node - shouldn't happen if routing is correct
                    return RpcMessage::ActorInvokeResponse {
                        request_id,
                        result: Err(format!(
                            "actor {} is on node {}, not this node {}",
                            actor_id, placement.node_id, self.local_node_id
                        )),
                    };
                }
            }
            Ok(None) => {
                // Actor not registered - try to claim it locally
                match self
                    .registry
                    .try_claim_actor(actor_id.clone(), self.local_node_id.clone())
                    .await
                {
                    Ok(PlacementDecision::New(_)) => {
                        debug!(actor_id = %actor_id, "claimed actor locally");
                    }
                    Ok(PlacementDecision::Existing(p)) => {
                        if p.node_id != self.local_node_id {
                            return RpcMessage::ActorInvokeResponse {
                                request_id,
                                result: Err(format!(
                                    "actor {} claimed by node {}, not this node {}",
                                    actor_id, p.node_id, self.local_node_id
                                )),
                            };
                        }
                    }
                    Ok(PlacementDecision::NoCapacity) => {
                        return RpcMessage::ActorInvokeResponse {
                            request_id,
                            result: Err("no capacity for actor".into()),
                        };
                    }
                    Err(e) => {
                        return RpcMessage::ActorInvokeResponse {
                            request_id,
                            result: Err(format!("failed to claim actor: {}", e)),
                        };
                    }
                }
            }
            Err(e) => {
                return RpcMessage::ActorInvokeResponse {
                    request_id,
                    result: Err(format!("failed to get placement: {}", e)),
                };
            }
        }

        // Invoke the actor locally
        let result = self.invoker.invoke(actor_id, operation, payload).await;

        RpcMessage::ActorInvokeResponse { request_id, result }
    }

    /// Handle migration prepare request
    async fn handle_migrate_prepare(
        &self,
        request_id: u64,
        actor_id: ActorId,
        from_node: NodeId,
    ) -> RpcMessage {
        debug!(
            request_id = request_id,
            actor_id = %actor_id,
            from = %from_node,
            "handling migrate prepare"
        );

        // Check if we can accept the migration
        match self.migration_receiver.can_accept(&actor_id).await {
            Ok(true) => {
                // Store pending migration
                let pending = PendingMigration {
                    actor_id: actor_id.clone(),
                    from_node,
                    state: None,
                    started_at_ms: now_ms(),
                };

                let mut pending_migrations = self.pending_migrations.write().await;
                pending_migrations.insert(actor_id, pending);

                RpcMessage::MigratePrepareResponse {
                    request_id,
                    ready: true,
                    reason: None,
                }
            }
            Ok(false) => RpcMessage::MigratePrepareResponse {
                request_id,
                ready: false,
                reason: Some("cannot accept migration".into()),
            },
            Err(e) => RpcMessage::MigratePrepareResponse {
                request_id,
                ready: false,
                reason: Some(e),
            },
        }
    }

    /// Handle migration transfer request
    async fn handle_migrate_transfer(
        &self,
        request_id: u64,
        actor_id: ActorId,
        state: Bytes,
        from_node: NodeId,
    ) -> RpcMessage {
        debug!(
            request_id = request_id,
            actor_id = %actor_id,
            from = %from_node,
            state_bytes = state.len(),
            "handling migrate transfer"
        );

        // Check if we have a pending migration for this actor
        {
            let mut pending = self.pending_migrations.write().await;
            if let Some(migration) = pending.get_mut(&actor_id) {
                if migration.from_node != from_node {
                    return RpcMessage::MigrateTransferResponse {
                        request_id,
                        success: false,
                        reason: Some(format!(
                            "migration source mismatch: expected {}, got {}",
                            migration.from_node, from_node
                        )),
                    };
                }

                // Receive the state
                match self
                    .migration_receiver
                    .receive_state(actor_id.clone(), state.clone())
                    .await
                {
                    Ok(()) => {
                        migration.state = Some(state);
                        return RpcMessage::MigrateTransferResponse {
                            request_id,
                            success: true,
                            reason: None,
                        };
                    }
                    Err(e) => {
                        return RpcMessage::MigrateTransferResponse {
                            request_id,
                            success: false,
                            reason: Some(e),
                        };
                    }
                }
            }
        }

        RpcMessage::MigrateTransferResponse {
            request_id,
            success: false,
            reason: Some("no pending migration for this actor".into()),
        }
    }

    /// Handle migration complete request
    async fn handle_migrate_complete(&self, request_id: u64, actor_id: ActorId) -> RpcMessage {
        debug!(
            request_id = request_id,
            actor_id = %actor_id,
            "handling migrate complete"
        );

        // Check if we have a pending migration with state
        {
            let pending = self.pending_migrations.read().await;
            if let Some(migration) = pending.get(&actor_id) {
                if migration.state.is_none() {
                    return RpcMessage::MigrateCompleteResponse {
                        request_id,
                        success: false,
                        reason: Some("no state received for migration".into()),
                    };
                }
            } else {
                return RpcMessage::MigrateCompleteResponse {
                    request_id,
                    success: false,
                    reason: Some("no pending migration for this actor".into()),
                };
            }
        }

        // Activate the migrated actor
        match self
            .migration_receiver
            .activate_migrated(actor_id.clone())
            .await
        {
            Ok(()) => {
                // Remove from pending
                let mut pending = self.pending_migrations.write().await;
                pending.remove(&actor_id);

                info!(actor_id = %actor_id, "migration complete - actor activated");

                RpcMessage::MigrateCompleteResponse {
                    request_id,
                    success: true,
                    reason: None,
                }
            }
            Err(e) => RpcMessage::MigrateCompleteResponse {
                request_id,
                success: false,
                reason: Some(e),
            },
        }
    }

    /// Handle heartbeat (process and update registry)
    async fn handle_heartbeat(&self, heartbeat: kelpie_registry::Heartbeat) -> Option<RpcMessage> {
        debug!(
            from = %heartbeat.node_id,
            seq = heartbeat.sequence,
            "received heartbeat"
        );

        // Update registry with heartbeat
        if let Err(e) = self.registry.receive_heartbeat(heartbeat.clone()).await {
            warn!(error = %e, "failed to process heartbeat");
        }

        // Send ack
        Some(RpcMessage::HeartbeatAck {
            node_id: self.local_node_id.clone(),
            sequence: heartbeat.sequence,
        })
    }

    /// Handle leave notification
    async fn handle_leave_notification(&self, node_id: NodeId) -> Option<RpcMessage> {
        info!(node = %node_id, "node leaving cluster");

        // Update node status in registry
        if let Err(e) = self
            .registry
            .update_node_status(&node_id, kelpie_registry::NodeStatus::Leaving)
            .await
        {
            warn!(error = %e, "failed to update leaving node status");
        }

        None // No response for leave notification
    }
}

#[async_trait]
impl<R: Registry + 'static> RpcHandler for ClusterRpcHandler<R> {
    async fn handle(&self, from: &NodeId, message: RpcMessage) -> Option<RpcMessage> {
        match message {
            // Actor invocation forwarding
            RpcMessage::ActorInvoke {
                request_id,
                actor_id,
                operation,
                payload,
            } => Some(
                self.handle_actor_invoke(request_id, actor_id, operation, payload)
                    .await,
            ),

            // Migration protocol
            RpcMessage::MigratePrepare {
                request_id,
                actor_id,
                from_node,
            } => Some(
                self.handle_migrate_prepare(request_id, actor_id, from_node)
                    .await,
            ),

            RpcMessage::MigrateTransfer {
                request_id,
                actor_id,
                state,
                from_node,
            } => Some(
                self.handle_migrate_transfer(request_id, actor_id, state, from_node)
                    .await,
            ),

            RpcMessage::MigrateComplete {
                request_id,
                actor_id,
            } => Some(self.handle_migrate_complete(request_id, actor_id).await),

            // Heartbeat
            RpcMessage::Heartbeat(heartbeat) => self.handle_heartbeat(heartbeat).await,

            // Leave notification
            RpcMessage::LeaveNotification { node_id } => {
                self.handle_leave_notification(node_id).await
            }

            // Cluster management (not handled here, handled by cluster coordinator)
            RpcMessage::JoinRequest { .. } => {
                debug!(from = %from, "ignoring join request (not implemented)");
                None
            }
            RpcMessage::ClusterStateRequest { .. } => {
                debug!(from = %from, "ignoring cluster state request (not implemented)");
                None
            }

            // Response messages should not be received here (they go to pending waiters)
            RpcMessage::ActorInvokeResponse { .. }
            | RpcMessage::MigratePrepareResponse { .. }
            | RpcMessage::MigrateTransferResponse { .. }
            | RpcMessage::MigrateCompleteResponse { .. }
            | RpcMessage::JoinResponse { .. }
            | RpcMessage::ClusterStateResponse { .. }
            | RpcMessage::HeartbeatAck { .. } => {
                warn!(from = %from, "received response message in handler");
                None
            }
        }
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
    use kelpie_registry::MemoryRegistry;
    use std::sync::Mutex;

    fn test_node_id(n: u32) -> NodeId {
        NodeId::new(format!("node-{}", n)).unwrap()
    }

    fn test_actor_id(n: u32) -> ActorId {
        ActorId::new("test", format!("actor-{}", n)).unwrap()
    }

    /// Mock invoker for testing
    struct MockInvoker {
        responses: Mutex<HashMap<String, Result<Bytes, String>>>,
    }

    impl MockInvoker {
        fn new() -> Self {
            Self {
                responses: Mutex::new(HashMap::new()),
            }
        }

        fn set_response(&self, actor_key: &str, result: Result<Bytes, String>) {
            let mut responses = self.responses.lock().unwrap();
            responses.insert(actor_key.to_string(), result);
        }
    }

    #[async_trait]
    impl ActorInvoker for MockInvoker {
        async fn invoke(
            &self,
            actor_id: ActorId,
            _operation: String,
            _payload: Bytes,
        ) -> Result<Bytes, String> {
            let responses = self.responses.lock().unwrap();
            responses
                .get(&actor_id.qualified_name())
                .cloned()
                .unwrap_or(Ok(Bytes::from("default")))
        }
    }

    /// Mock migration receiver for testing
    struct MockMigrationReceiver {
        can_accept: Mutex<bool>,
        received_states: Mutex<HashMap<String, Bytes>>,
        activated: Mutex<Vec<String>>,
    }

    impl MockMigrationReceiver {
        fn new() -> Self {
            Self {
                can_accept: Mutex::new(true),
                received_states: Mutex::new(HashMap::new()),
                activated: Mutex::new(Vec::new()),
            }
        }

        fn set_can_accept(&self, can: bool) {
            *self.can_accept.lock().unwrap() = can;
        }

        fn get_activated(&self) -> Vec<String> {
            self.activated.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl MigrationReceiver for MockMigrationReceiver {
        async fn can_accept(&self, _actor_id: &ActorId) -> Result<bool, String> {
            Ok(*self.can_accept.lock().unwrap())
        }

        async fn receive_state(&self, actor_id: ActorId, state: Bytes) -> Result<(), String> {
            let mut states = self.received_states.lock().unwrap();
            states.insert(actor_id.qualified_name(), state);
            Ok(())
        }

        async fn activate_migrated(&self, actor_id: ActorId) -> Result<(), String> {
            let mut activated = self.activated.lock().unwrap();
            activated.push(actor_id.qualified_name());
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_handle_actor_invoke() {
        let registry = Arc::new(MemoryRegistry::new());
        let invoker = Arc::new(MockInvoker::new());
        let migration_receiver = Arc::new(MockMigrationReceiver::new());
        let local_node_id = test_node_id(1);

        // Register local node
        let mut node = kelpie_registry::NodeInfo::new(
            local_node_id.clone(),
            "127.0.0.1:9001".parse().unwrap(),
        );
        node.status = kelpie_registry::NodeStatus::Active;
        registry.register_node(node).await.unwrap();

        let handler = ClusterRpcHandler::new(
            local_node_id.clone(),
            registry.clone(),
            invoker.clone(),
            migration_receiver,
        );

        let actor_id = test_actor_id(1);
        invoker.set_response(&actor_id.qualified_name(), Ok(Bytes::from("test-result")));

        // Register actor with local node
        registry
            .try_claim_actor(actor_id.clone(), local_node_id.clone())
            .await
            .unwrap();

        let msg = RpcMessage::ActorInvoke {
            request_id: 1,
            actor_id: actor_id.clone(),
            operation: "test".to_string(),
            payload: Bytes::new(),
        };

        let response = handler.handle(&test_node_id(2), msg).await;

        match response {
            Some(RpcMessage::ActorInvokeResponse { result, .. }) => {
                assert_eq!(result.unwrap(), Bytes::from("test-result"));
            }
            _ => panic!("expected ActorInvokeResponse"),
        }
    }

    #[tokio::test]
    async fn test_handle_migration_flow() {
        let registry = Arc::new(MemoryRegistry::new());
        let invoker = Arc::new(MockInvoker::new());
        let migration_receiver = Arc::new(MockMigrationReceiver::new());
        let local_node_id = test_node_id(2); // We are node-2, receiving migration from node-1

        let handler = ClusterRpcHandler::new(
            local_node_id.clone(),
            registry.clone(),
            invoker,
            migration_receiver.clone(),
        );

        let actor_id = test_actor_id(1);
        let from_node = test_node_id(1);

        // Step 1: Prepare
        let prepare_msg = RpcMessage::MigratePrepare {
            request_id: 1,
            actor_id: actor_id.clone(),
            from_node: from_node.clone(),
        };
        let response = handler.handle(&from_node, prepare_msg).await;
        match response {
            Some(RpcMessage::MigratePrepareResponse { ready, .. }) => {
                assert!(ready);
            }
            _ => panic!("expected MigratePrepareResponse"),
        }

        // Step 2: Transfer
        let state = Bytes::from("actor-state-data");
        let transfer_msg = RpcMessage::MigrateTransfer {
            request_id: 2,
            actor_id: actor_id.clone(),
            state,
            from_node: from_node.clone(),
        };
        let response = handler.handle(&from_node, transfer_msg).await;
        match response {
            Some(RpcMessage::MigrateTransferResponse { success, .. }) => {
                assert!(success);
            }
            _ => panic!("expected MigrateTransferResponse"),
        }

        // Step 3: Complete
        let complete_msg = RpcMessage::MigrateComplete {
            request_id: 3,
            actor_id: actor_id.clone(),
        };
        let response = handler.handle(&from_node, complete_msg).await;
        match response {
            Some(RpcMessage::MigrateCompleteResponse { success, .. }) => {
                assert!(success);
            }
            _ => panic!("expected MigrateCompleteResponse"),
        }

        // Verify activation was called
        let activated = migration_receiver.get_activated();
        assert_eq!(activated.len(), 1);
        assert_eq!(activated[0], actor_id.qualified_name());
    }

    #[tokio::test]
    async fn test_handle_migration_prepare_rejected() {
        let registry = Arc::new(MemoryRegistry::new());
        let invoker = Arc::new(MockInvoker::new());
        let migration_receiver = Arc::new(MockMigrationReceiver::new());
        migration_receiver.set_can_accept(false);

        let handler =
            ClusterRpcHandler::new(test_node_id(2), registry, invoker, migration_receiver);

        let msg = RpcMessage::MigratePrepare {
            request_id: 1,
            actor_id: test_actor_id(1),
            from_node: test_node_id(1),
        };

        let response = handler.handle(&test_node_id(1), msg).await;
        match response {
            Some(RpcMessage::MigratePrepareResponse { ready, .. }) => {
                assert!(!ready);
            }
            _ => panic!("expected MigratePrepareResponse"),
        }
    }
}
