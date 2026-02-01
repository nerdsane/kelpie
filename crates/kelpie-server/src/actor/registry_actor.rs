//! RegistryActor - manages global agent registry
//!
//! TigerStyle: Message-based registration, explicit operations, state persistence.
//!
//! This actor provides a clean message-passing API for agent registration,
//! solving the architectural limitation where actors cannot write to other namespaces.
//!
//! ## Operations
//! - `register` - Register an agent in the global registry
//! - `unregister` - Remove an agent from the registry
//! - `list` - List all registered agents
//! - `get` - Get specific agent metadata
//!
//! ## Architecture
//! - Actor ID: "system/agent_registry"
//! - Uses AgentStorage backend for persistence
//! - Maintains in-memory cache for fast lookups
//! - State includes metrics (agent count, last updated)

use crate::storage::{AgentMetadata, AgentStorage};
use async_trait::async_trait;
use bytes::Bytes;
use kelpie_core::actor::{Actor, ActorContext};
use kelpie_core::io::{TimeProvider, WallClockTime};
use kelpie_core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// RegistryActor - manages global agent metadata
///
/// TigerStyle: Clean abstraction, explicit error handling, testable.
#[derive(Clone)]
pub struct RegistryActor {
    /// Storage backend for persistence
    storage: Arc<dyn AgentStorage>,
}

impl RegistryActor {
    /// Create a new RegistryActor with storage backend
    pub fn new(storage: Arc<dyn AgentStorage>) -> Self {
        Self { storage }
    }

    /// Handle "register" operation - register agent metadata
    async fn handle_register(
        &self,
        ctx: &mut ActorContext<RegistryActorState>,
        request: RegisterRequest,
    ) -> Result<RegisterResponse> {
        // TigerStyle: Validate preconditions
        assert!(
            !request.metadata.id.is_empty(),
            "agent id must not be empty"
        );
        assert!(
            !request.metadata.name.is_empty(),
            "agent name must not be empty"
        );

        // Save to storage
        self.storage
            .save_agent(&request.metadata)
            .await
            .map_err(|e| Error::Internal {
                message: format!("Failed to save agent metadata: {}", e),
            })?;

        // Update state metrics
        ctx.state.agent_count += 1;
        ctx.state.last_updated_ms = WallClockTime::new().now_ms();

        tracing::info!(
            agent_id = %request.metadata.id,
            agent_name = %request.metadata.name,
            "Agent registered in global registry"
        );

        Ok(RegisterResponse {
            status: "ok".to_string(),
            agent_id: request.metadata.id.clone(),
        })
    }

    /// Handle "unregister" operation - remove agent from registry
    async fn handle_unregister(
        &self,
        ctx: &mut ActorContext<RegistryActorState>,
        request: UnregisterRequest,
    ) -> Result<UnregisterResponse> {
        // TigerStyle: Validate preconditions
        assert!(!request.agent_id.is_empty(), "agent id must not be empty");

        // Delete from storage
        self.storage
            .delete_agent(&request.agent_id)
            .await
            .map_err(|e| Error::Internal {
                message: format!("Failed to delete agent: {}", e),
            })?;

        // Update state metrics
        if ctx.state.agent_count > 0 {
            ctx.state.agent_count -= 1;
        }
        ctx.state.last_updated_ms = WallClockTime::new().now_ms();

        tracing::info!(
            agent_id = %request.agent_id,
            "Agent unregistered from global registry"
        );

        Ok(UnregisterResponse {
            status: "ok".to_string(),
        })
    }

    /// Handle "list" operation - list all registered agents
    async fn handle_list(
        &self,
        _ctx: &ActorContext<RegistryActorState>,
        _request: ListRequest,
    ) -> Result<ListResponse> {
        // Load all agents from storage
        let agents = self
            .storage
            .list_agents()
            .await
            .map_err(|e| Error::Internal {
                message: format!("Failed to list agents: {}", e),
            })?;

        tracing::debug!(agent_count = agents.len(), "Listed agents from registry");

        Ok(ListResponse { agents })
    }

    /// Handle "get" operation - get specific agent metadata
    async fn handle_get(
        &self,
        _ctx: &ActorContext<RegistryActorState>,
        request: GetRequest,
    ) -> Result<GetResponse> {
        // TigerStyle: Validate preconditions
        assert!(!request.agent_id.is_empty(), "agent id must not be empty");

        // Load from storage
        let agent = self
            .storage
            .load_agent(&request.agent_id)
            .await
            .map_err(|e| Error::Internal {
                message: format!("Failed to load agent: {}", e),
            })?;

        if agent.is_none() {
            tracing::debug!(agent_id = %request.agent_id, "Agent not found in registry");
        }

        Ok(GetResponse { agent })
    }
}

#[async_trait]
impl Actor for RegistryActor {
    type State = RegistryActorState;

    async fn invoke(
        &self,
        ctx: &mut ActorContext<Self::State>,
        operation: &str,
        payload: Bytes,
    ) -> Result<Bytes> {
        // TigerStyle: Explicit operation routing with clear error messages
        match operation {
            "register" => {
                let request: RegisterRequest =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize RegisterRequest: {}", e),
                    })?;
                let response = self.handle_register(ctx, request).await?;
                let response_bytes =
                    serde_json::to_vec(&response).map_err(|e| Error::Internal {
                        message: format!("Failed to serialize RegisterResponse: {}", e),
                    })?;
                Ok(Bytes::from(response_bytes))
            }
            "unregister" => {
                let request: UnregisterRequest =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize UnregisterRequest: {}", e),
                    })?;
                let response = self.handle_unregister(ctx, request).await?;
                let response_bytes =
                    serde_json::to_vec(&response).map_err(|e| Error::Internal {
                        message: format!("Failed to serialize UnregisterResponse: {}", e),
                    })?;
                Ok(Bytes::from(response_bytes))
            }
            "list" => {
                let request: ListRequest =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize ListRequest: {}", e),
                    })?;
                let response = self.handle_list(ctx, request).await?;
                let response_bytes =
                    serde_json::to_vec(&response).map_err(|e| Error::Internal {
                        message: format!("Failed to serialize ListResponse: {}", e),
                    })?;
                Ok(Bytes::from(response_bytes))
            }
            "get" => {
                let request: GetRequest =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize GetRequest: {}", e),
                    })?;
                let response = self.handle_get(ctx, request).await?;
                let response_bytes =
                    serde_json::to_vec(&response).map_err(|e| Error::Internal {
                        message: format!("Failed to serialize GetResponse: {}", e),
                    })?;
                Ok(Bytes::from(response_bytes))
            }
            _ => Err(Error::Internal {
                message: format!("Unknown operation: {}", operation),
            }),
        }
    }

    async fn on_activate(&self, _ctx: &mut ActorContext<Self::State>) -> Result<()> {
        tracing::debug!("RegistryActor activated");
        Ok(())
    }

    async fn on_deactivate(&self, ctx: &mut ActorContext<Self::State>) -> Result<()> {
        tracing::debug!(
            agent_count = ctx.state.agent_count,
            "RegistryActor deactivated, state persisted"
        );
        Ok(())
    }
}

// =============================================================================
// Registry Actor State
// =============================================================================

/// State for RegistryActor
///
/// TigerStyle: Serializable state, explicit fields, no complex nested structures.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegistryActorState {
    /// Number of registered agents (for metrics)
    pub agent_count: u64,
    /// Last updated timestamp (ms since epoch)
    pub last_updated_ms: u64,
}

// =============================================================================
// Request/Response Types
// =============================================================================

/// Register agent request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    /// Agent metadata to register
    pub metadata: AgentMetadata,
}

/// Register agent response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterResponse {
    /// Status ("ok" or error)
    pub status: String,
    /// Registered agent ID
    pub agent_id: String,
}

/// Unregister agent request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnregisterRequest {
    /// Agent ID to unregister
    pub agent_id: String,
}

/// Unregister agent response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnregisterResponse {
    /// Status ("ok" or error)
    pub status: String,
}

/// List agents request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListRequest {
    /// Optional filter (future: tags, agent_type, etc.)
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

/// List agents response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse {
    /// List of registered agents
    pub agents: Vec<AgentMetadata>,
}

/// Get agent request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetRequest {
    /// Agent ID to retrieve
    pub agent_id: String,
}

/// Get agent response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetResponse {
    /// Agent metadata (None if not found)
    pub agent: Option<AgentMetadata>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AgentType;
    use crate::storage::KvAdapter;
    use kelpie_core::actor::ActorId;
    use kelpie_storage::{MemoryKV, ScopedKV};

    fn create_test_metadata(id: &str, name: &str) -> AgentMetadata {
        AgentMetadata::new(id.to_string(), name.to_string(), AgentType::MemgptAgent)
    }

    #[tokio::test]
    async fn test_registry_register_agent() {
        let kv = Arc::new(MemoryKV::new());
        let storage = Arc::new(KvAdapter::new(kv.clone()));
        let actor = RegistryActor::new(storage.clone());

        let actor_id = ActorId::new("system", "agent_registry").unwrap();
        let scoped_kv = ScopedKV::new(actor_id.clone(), kv);
        let mut ctx =
            ActorContext::new(actor_id, RegistryActorState::default(), Box::new(scoped_kv));

        // Register an agent
        let metadata = create_test_metadata("agent-1", "Test Agent");
        let request = RegisterRequest { metadata };
        let response = actor.handle_register(&mut ctx, request).await.unwrap();

        assert_eq!(response.status, "ok");
        assert_eq!(response.agent_id, "agent-1");
        assert_eq!(ctx.state.agent_count, 1);

        // Verify it was saved to storage
        let loaded = storage.load_agent("agent-1").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().name, "Test Agent");
    }

    #[tokio::test]
    async fn test_registry_list_agents() {
        let kv = Arc::new(MemoryKV::new());
        let storage = Arc::new(KvAdapter::new(kv.clone()));
        let actor = RegistryActor::new(storage.clone());

        let actor_id = ActorId::new("system", "agent_registry").unwrap();
        let scoped_kv = ScopedKV::new(actor_id.clone(), kv);
        let mut ctx =
            ActorContext::new(actor_id, RegistryActorState::default(), Box::new(scoped_kv));

        // Register two agents
        let metadata1 = create_test_metadata("agent-1", "Agent 1");
        actor
            .handle_register(
                &mut ctx,
                RegisterRequest {
                    metadata: metadata1,
                },
            )
            .await
            .unwrap();

        let metadata2 = create_test_metadata("agent-2", "Agent 2");
        actor
            .handle_register(
                &mut ctx,
                RegisterRequest {
                    metadata: metadata2,
                },
            )
            .await
            .unwrap();

        // List all agents
        let request = ListRequest { filter: None };
        let response = actor.handle_list(&ctx, request).await.unwrap();

        assert_eq!(response.agents.len(), 2);
        assert_eq!(ctx.state.agent_count, 2);
    }

    #[tokio::test]
    async fn test_registry_get_agent() {
        let kv = Arc::new(MemoryKV::new());
        let storage = Arc::new(KvAdapter::new(kv.clone()));
        let actor = RegistryActor::new(storage.clone());

        let actor_id = ActorId::new("system", "agent_registry").unwrap();
        let scoped_kv = ScopedKV::new(actor_id.clone(), kv);
        let mut ctx =
            ActorContext::new(actor_id, RegistryActorState::default(), Box::new(scoped_kv));

        // Register an agent
        let metadata = create_test_metadata("agent-1", "Test Agent");
        actor
            .handle_register(&mut ctx, RegisterRequest { metadata })
            .await
            .unwrap();

        // Get the agent
        let request = GetRequest {
            agent_id: "agent-1".to_string(),
        };
        let response = actor.handle_get(&ctx, request).await.unwrap();

        assert!(response.agent.is_some());
        assert_eq!(response.agent.unwrap().name, "Test Agent");

        // Try to get non-existent agent
        let request = GetRequest {
            agent_id: "non-existent".to_string(),
        };
        let response = actor.handle_get(&ctx, request).await.unwrap();
        assert!(response.agent.is_none());
    }

    #[tokio::test]
    async fn test_registry_unregister_agent() {
        let kv = Arc::new(MemoryKV::new());
        let storage = Arc::new(KvAdapter::new(kv.clone()));
        let actor = RegistryActor::new(storage.clone());

        let actor_id = ActorId::new("system", "agent_registry").unwrap();
        let scoped_kv = ScopedKV::new(actor_id.clone(), kv);
        let mut ctx =
            ActorContext::new(actor_id, RegistryActorState::default(), Box::new(scoped_kv));

        // Register an agent
        let metadata = create_test_metadata("agent-1", "Test Agent");
        actor
            .handle_register(&mut ctx, RegisterRequest { metadata })
            .await
            .unwrap();

        assert_eq!(ctx.state.agent_count, 1);

        // Unregister the agent
        let request = UnregisterRequest {
            agent_id: "agent-1".to_string(),
        };
        let response = actor.handle_unregister(&mut ctx, request).await.unwrap();

        assert_eq!(response.status, "ok");
        assert_eq!(ctx.state.agent_count, 0);

        // Verify it was deleted from storage
        let loaded = storage.load_agent("agent-1").await.unwrap();
        assert!(loaded.is_none());
    }
}
