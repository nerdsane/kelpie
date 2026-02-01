//! DST tests for RegistryActor
//!
//! Tests registry operations under fault injection to ensure:
//! - Registry consistency under storage faults
//! - Concurrent registrations don't conflict
//! - Registry state survives actor deactivation/reactivation
//! - Self-registration works under failures
//!
//! ## TLA+ Invariant Alignment (Issue #90)
//!
//! This module now includes tests that verify TLA+ invariants from `KelpieRegistry.tla`:
//!
//! - **SingleActivation**: An actor is placed on at most one node at any time
//! - **PlacementConsistency**: Placed actors are not on failed nodes
//!
//! See: `docs/tla/KelpieRegistry.tla` for the formal specification.

#![cfg(feature = "dst")]

use bytes::Bytes;
use futures::future::join_all;
use kelpie_core::actor::{Actor, ActorContext, ActorId};
use kelpie_core::{Error, Result, Runtime};
use kelpie_dst::invariants::{
    InvariantChecker, InvariantViolation, NodeInfo, NodeState, NodeStatus, PlacementConsistency,
    SingleActivation, SystemState,
};
use kelpie_dst::{FaultConfig, FaultType, SimConfig, Simulation};
use kelpie_server::actor::{
    AgentActor, AgentActorState, GetRequest, GetResponse, ListRequest, ListResponse,
    RegisterRequest, RegisterResponse, RegistryActor, RegistryActorState, UnregisterRequest,
};
use kelpie_server::models::{AgentType, CreateAgentRequest};
use kelpie_server::storage::{AgentMetadata, KvAdapter};
use kelpie_server::tools::UnifiedToolRegistry;
use kelpie_storage::{ActorKV, ScopedKV};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Mock LLM for testing
struct MockLlm;

#[async_trait::async_trait]
impl kelpie_server::actor::LlmClient for MockLlm {
    async fn complete(
        &self,
        _messages: Vec<kelpie_server::actor::LlmMessage>,
    ) -> Result<kelpie_server::actor::LlmResponse> {
        Ok(kelpie_server::actor::LlmResponse {
            content: "Mock response".to_string(),
            tool_calls: vec![],
            prompt_tokens: 10,
            completion_tokens: 10,
            stop_reason: "end_turn".to_string(),
        })
    }

    async fn complete_with_tools(
        &self,
        _messages: Vec<kelpie_server::actor::LlmMessage>,
        _tools: Vec<kelpie_server::llm::ToolDefinition>,
    ) -> Result<kelpie_server::actor::LlmResponse> {
        Ok(kelpie_server::actor::LlmResponse {
            content: "Mock response".to_string(),
            tool_calls: vec![],
            prompt_tokens: 10,
            completion_tokens: 10,
            stop_reason: "end_turn".to_string(),
        })
    }

    async fn continue_with_tool_result(
        &self,
        _messages: Vec<kelpie_server::actor::LlmMessage>,
        _tools: Vec<kelpie_server::llm::ToolDefinition>,
        _assistant_blocks: Vec<kelpie_server::llm::ContentBlock>,
        _tool_results: Vec<(String, String)>,
    ) -> Result<kelpie_server::actor::LlmResponse> {
        Ok(kelpie_server::actor::LlmResponse {
            content: "Continued".to_string(),
            tool_calls: vec![],
            prompt_tokens: 5,
            completion_tokens: 10,
            stop_reason: "end_turn".to_string(),
        })
    }
}

fn create_test_metadata(id: &str, name: &str) -> AgentMetadata {
    AgentMetadata::new(id.to_string(), name.to_string(), AgentType::MemgptAgent)
}

/// Test: Registry operations under virtual time (DST basic functionality)
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_registry_operations_dst() {
    let config = SimConfig::new(9001);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            let kv = Arc::new(sim_env.storage.clone());
            let storage = Arc::new(KvAdapter::new(kv.clone()));
            let registry_actor = RegistryActor::new(storage.clone());

            let registry_id = ActorId::new("system", "agent_registry")?;
            let scoped_kv = ScopedKV::new(registry_id.clone(), kv as Arc<dyn ActorKV>);
            let mut ctx = ActorContext::new(
                registry_id,
                RegistryActorState::default(),
                Box::new(scoped_kv),
            );

            // Register 3 agents
            for i in 1..=3 {
                let metadata =
                    create_test_metadata(&format!("agent-{}", i), &format!("Agent {}", i));
                let request = RegisterRequest { metadata };
                let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
                    message: format!("Failed to serialize RegisterRequest: {}", e),
                })?;
                let response_bytes = registry_actor
                    .invoke(&mut ctx, "register", Bytes::from(payload))
                    .await?;
                let _response: RegisterResponse =
                    serde_json::from_slice(&response_bytes).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize RegisterResponse: {}", e),
                    })?;
            }

            // List all agents
            let list_request = ListRequest { filter: None };
            let payload = serde_json::to_vec(&list_request).map_err(|e| Error::Internal {
                message: format!("Failed to serialize ListRequest: {}", e),
            })?;
            let response_bytes = registry_actor
                .invoke(&mut ctx, "list", Bytes::from(payload))
                .await?;
            let response: ListResponse =
                serde_json::from_slice(&response_bytes).map_err(|e| Error::Internal {
                    message: format!("Failed to deserialize ListResponse: {}", e),
                })?;
            assert_eq!(response.agents.len(), 3);

            // Get specific agent
            let get_request = GetRequest {
                agent_id: "agent-2".to_string(),
            };
            let payload = serde_json::to_vec(&get_request).map_err(|e| Error::Internal {
                message: format!("Failed to serialize GetRequest: {}", e),
            })?;
            let response_bytes = registry_actor
                .invoke(&mut ctx, "get", Bytes::from(payload))
                .await?;
            let response: GetResponse =
                serde_json::from_slice(&response_bytes).map_err(|e| Error::Internal {
                    message: format!("Failed to deserialize GetResponse: {}", e),
                })?;
            assert!(response.agent.is_some());
            assert_eq!(response.agent.unwrap().name, "Agent 2");

            tracing::info!("✅ Registry operations work under DST");
            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test: Registry state survives actor deactivation/reactivation
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_registry_survives_deactivation_dst() {
    let config = SimConfig::new(9002);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            let kv = Arc::new(sim_env.storage.clone());
            let storage = Arc::new(KvAdapter::new(kv.clone()));
            let registry_actor = RegistryActor::new(storage.clone());

            let registry_id = ActorId::new("system", "agent_registry")?;
            let scoped_kv = ScopedKV::new(registry_id.clone(), kv.clone() as Arc<dyn ActorKV>);
            let mut ctx = ActorContext::new(
                registry_id.clone(),
                RegistryActorState::default(),
                Box::new(scoped_kv),
            );

            // Register agents
            for i in 1..=5 {
                let metadata =
                    create_test_metadata(&format!("agent-{}", i), &format!("Agent {}", i));
                let request = RegisterRequest { metadata };
                let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
                    message: format!("Failed to serialize RegisterRequest: {}", e),
                })?;
                let _response_bytes = registry_actor
                    .invoke(&mut ctx, "register", Bytes::from(payload))
                    .await?;
            }

            assert_eq!(ctx.state.agent_count, 5);

            // Deactivate actor (simulates actor being evicted from memory)
            registry_actor.on_deactivate(&mut ctx).await?;

            // Persist state (normally done by runtime)
            let state_bytes = serde_json::to_vec(&ctx.state).map_err(|e| Error::Internal {
                message: format!("Failed to serialize state: {}", e),
            })?;
            ctx.kv_set(b"registry_state", &state_bytes).await?;

            // Simulate reactivation - load state from storage
            let loaded_state_bytes = ctx.kv_get(b"registry_state").await?.unwrap();
            let loaded_state: RegistryActorState = serde_json::from_slice(&loaded_state_bytes)
                .map_err(|e| Error::Internal {
                    message: format!("Failed to deserialize state: {}", e),
                })?;
            ctx.state = loaded_state;

            // Verify state preserved
            assert_eq!(ctx.state.agent_count, 5);

            // Verify agents still accessible via storage
            let list_request = ListRequest { filter: None };
            let payload = serde_json::to_vec(&list_request).map_err(|e| Error::Internal {
                message: format!("Failed to serialize ListRequest: {}", e),
            })?;
            let response_bytes = registry_actor
                .invoke(&mut ctx, "list", Bytes::from(payload))
                .await?;
            let response: ListResponse =
                serde_json::from_slice(&response_bytes).map_err(|e| Error::Internal {
                    message: format!("Failed to deserialize ListResponse: {}", e),
                })?;
            assert_eq!(response.agents.len(), 5);

            tracing::info!("✅ Registry state survives deactivation/reactivation");
            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test: Multiple agents registering concurrently
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_concurrent_registrations_dst() {
    let config = SimConfig::new(9003);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            let kv = Arc::new(sim_env.storage.clone());
            let storage = Arc::new(KvAdapter::new(kv.clone()));
            let registry_actor = RegistryActor::new(storage.clone());

            let registry_id = ActorId::new("system", "agent_registry")?;
            let scoped_kv = ScopedKV::new(registry_id.clone(), kv as Arc<dyn ActorKV>);
            let mut ctx = ActorContext::new(
                registry_id,
                RegistryActorState::default(),
                Box::new(scoped_kv),
            );

            // Simulate concurrent registrations by registering multiple agents rapidly
            // In a real system, these would be concurrent invocations
            // Here we test that sequential rapid registrations maintain consistency
            for i in 1..=10 {
                let metadata =
                    create_test_metadata(&format!("agent-{}", i), &format!("Agent {}", i));
                let request = RegisterRequest { metadata };
                let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
                    message: format!("Failed to serialize RegisterRequest: {}", e),
                })?;
                let _response_bytes = registry_actor
                    .invoke(&mut ctx, "register", Bytes::from(payload))
                    .await?;
            }

            // Verify all registered
            let list_request = ListRequest { filter: None };
            let payload = serde_json::to_vec(&list_request).map_err(|e| Error::Internal {
                message: format!("Failed to serialize ListRequest: {}", e),
            })?;
            let response_bytes = registry_actor
                .invoke(&mut ctx, "list", Bytes::from(payload))
                .await?;
            let response: ListResponse =
                serde_json::from_slice(&response_bytes).map_err(|e| Error::Internal {
                    message: format!("Failed to deserialize ListResponse: {}", e),
                })?;
            assert_eq!(response.agents.len(), 10);
            assert_eq!(ctx.state.agent_count, 10);

            // Verify no duplicates
            let mut ids: Vec<_> = response.agents.iter().map(|a| a.id.as_str()).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), 10, "Found duplicate agent IDs");

            tracing::info!("✅ Concurrent registrations maintain consistency");
            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test: Registry operations with agent lifecycle (create + self-register)
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_agent_lifecycle_with_registry_dst() {
    let config = SimConfig::new(9004);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            let kv = Arc::new(sim_env.storage.clone());
            let storage = Arc::new(KvAdapter::new(kv.clone()));

            // Create RegistryActor
            let registry_actor = RegistryActor::new(storage.clone());
            let registry_id = ActorId::new("system", "agent_registry")?;
            let registry_scoped_kv =
                ScopedKV::new(registry_id.clone(), kv.clone() as Arc<dyn ActorKV>);
            let mut registry_ctx = ActorContext::new(
                registry_id.clone(),
                RegistryActorState::default(),
                Box::new(registry_scoped_kv),
            );

            registry_actor.on_activate(&mut registry_ctx).await?;

            // Create AgentActor (without dispatcher for this test)
            let llm = Arc::new(MockLlm);
            let tool_registry = Arc::new(UnifiedToolRegistry::new());
            let agent_actor = AgentActor::new(llm, tool_registry);

            let agent_id = ActorId::new("agents", "test-agent")?;
            let agent_scoped_kv = ScopedKV::new(agent_id.clone(), kv.clone() as Arc<dyn ActorKV>);
            let mut agent_ctx = ActorContext::new(
                agent_id.clone(),
                AgentActorState::default(),
                Box::new(agent_scoped_kv),
            );

            // Create agent
            let request = CreateAgentRequest {
                name: "Test Agent".to_string(),
                agent_type: AgentType::MemgptAgent,
                model: None,
                embedding: None,
                system: Some("You are helpful".to_string()),
                description: None,
                memory_blocks: vec![],
                block_ids: vec![],
                tool_ids: vec![],
                tags: vec![],
                metadata: serde_json::json!({}),
                project_id: None,
                user_id: None,
                org_id: None,
            };

            let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
                message: format!("Failed to serialize CreateAgentRequest: {}", e),
            })?;
            agent_actor
                .invoke(&mut agent_ctx, "create", Bytes::from(payload))
                .await?;

            // Manually register agent (simulating what dispatcher would do)
            // In real system, AgentActor.on_activate() would send message to RegistryActor
            if let Some(agent) = &agent_ctx.state.agent {
                let metadata = AgentMetadata {
                    id: agent.id.clone(),
                    name: agent.name.clone(),
                    agent_type: agent.agent_type.clone(),
                    model: agent.model.clone(),
                    embedding: agent.embedding.clone(),
                    system: agent.system.clone(),
                    description: agent.description.clone(),
                    tool_ids: agent.tool_ids.clone(),
                    tags: agent.tags.clone(),
                    metadata: agent.metadata.clone(),
                    created_at: agent.created_at,
                    updated_at: agent.updated_at,
                };

                let register_request = RegisterRequest { metadata };
                let payload =
                    serde_json::to_vec(&register_request).map_err(|e| Error::Internal {
                        message: format!("Failed to serialize RegisterRequest: {}", e),
                    })?;
                let _response_bytes = registry_actor
                    .invoke(&mut registry_ctx, "register", Bytes::from(payload))
                    .await?;
            }

            // Verify agent is registered
            let list_request = ListRequest { filter: None };
            let payload = serde_json::to_vec(&list_request).map_err(|e| Error::Internal {
                message: format!("Failed to serialize ListRequest: {}", e),
            })?;
            let response_bytes = registry_actor
                .invoke(&mut registry_ctx, "list", Bytes::from(payload))
                .await?;
            let response: ListResponse =
                serde_json::from_slice(&response_bytes).map_err(|e| Error::Internal {
                    message: format!("Failed to deserialize ListResponse: {}", e),
                })?;
            assert_eq!(response.agents.len(), 1);
            assert_eq!(response.agents[0].id, "test-agent");
            assert_eq!(response.agents[0].name, "Test Agent");

            tracing::info!("✅ Agent lifecycle with registry works in DST");
            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test: Registry unregister operation
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_registry_unregister_dst() {
    let config = SimConfig::new(9005);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            let kv = Arc::new(sim_env.storage.clone());
            let storage = Arc::new(KvAdapter::new(kv.clone()));
            let registry_actor = RegistryActor::new(storage.clone());

            let registry_id = ActorId::new("system", "agent_registry")?;
            let scoped_kv = ScopedKV::new(registry_id.clone(), kv as Arc<dyn ActorKV>);
            let mut ctx = ActorContext::new(
                registry_id,
                RegistryActorState::default(),
                Box::new(scoped_kv),
            );

            // Register 5 agents
            for i in 1..=5 {
                let metadata =
                    create_test_metadata(&format!("agent-{}", i), &format!("Agent {}", i));
                let request = RegisterRequest { metadata };
                let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
                    message: format!("Failed to serialize RegisterRequest: {}", e),
                })?;
                let _response_bytes = registry_actor
                    .invoke(&mut ctx, "register", Bytes::from(payload))
                    .await?;
            }

            assert_eq!(ctx.state.agent_count, 5);

            // Unregister 2 agents
            for i in [2, 4] {
                let unregister_request = UnregisterRequest {
                    agent_id: format!("agent-{}", i),
                };
                let payload =
                    serde_json::to_vec(&unregister_request).map_err(|e| Error::Internal {
                        message: format!("Failed to serialize UnregisterRequest: {}", e),
                    })?;
                let _response_bytes = registry_actor
                    .invoke(&mut ctx, "unregister", Bytes::from(payload))
                    .await?;
            }

            // Verify count updated
            assert_eq!(ctx.state.agent_count, 3);

            // Verify correct agents remain
            let list_request = ListRequest { filter: None };
            let payload = serde_json::to_vec(&list_request).map_err(|e| Error::Internal {
                message: format!("Failed to serialize ListRequest: {}", e),
            })?;
            let response_bytes = registry_actor
                .invoke(&mut ctx, "list", Bytes::from(payload))
                .await?;
            let response: ListResponse =
                serde_json::from_slice(&response_bytes).map_err(|e| Error::Internal {
                    message: format!("Failed to deserialize ListResponse: {}", e),
                })?;
            assert_eq!(response.agents.len(), 3);

            let remaining_ids: Vec<_> = response.agents.iter().map(|a| a.id.as_str()).collect();
            assert!(remaining_ids.contains(&"agent-1"));
            assert!(remaining_ids.contains(&"agent-3"));
            assert!(remaining_ids.contains(&"agent-5"));
            assert!(!remaining_ids.contains(&"agent-2"));
            assert!(!remaining_ids.contains(&"agent-4"));

            tracing::info!("✅ Registry unregister works correctly");
            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// TLA+ Invariant Aligned Tests (Issue #90)
// =============================================================================
//
// These tests verify the safety invariants from KelpieRegistry.tla:
//
// SingleActivation ==
//     \A a \in Actors :
//         Cardinality({n \in Nodes : placement[a] = n}) <= 1
//
// PlacementConsistency ==
//     \A a \in Actors :
//         placement[a] # NULL => nodeStatus[placement[a]] # Failed
//
// =============================================================================

/// Multi-node registry placement state for TLA+ invariant verification
///
/// This structure models the distributed registry state as specified in
/// KelpieRegistry.tla, tracking:
/// - nodeStatus: Status of each node (Active/Suspect/Failed)
/// - placement: Actor -> Node mapping (authoritative)
/// - heartbeatCount: Missed heartbeat counters
#[derive(Debug)]
struct RegistryPlacementState {
    /// Node status: node_id -> NodeStatus
    node_status: HashMap<String, NodeStatus>,
    /// Authoritative placements: actor_id -> node_id
    placements: HashMap<String, String>,
    /// Heartbeat miss counters: node_id -> count
    heartbeat_count: HashMap<String, u64>,
    /// Placement version for OCC
    version: u64,
}

impl Clone for RegistryPlacementState {
    fn clone(&self) -> Self {
        Self {
            node_status: self.node_status.clone(),
            placements: self.placements.clone(),
            heartbeat_count: self.heartbeat_count.clone(),
            version: self.version,
        }
    }
}

impl RegistryPlacementState {
    fn new() -> Self {
        Self {
            node_status: HashMap::new(),
            placements: HashMap::new(),
            heartbeat_count: HashMap::new(),
            version: 0,
        }
    }

    /// Add a node with Active status
    fn add_node(&mut self, node_id: &str) {
        self.node_status
            .insert(node_id.to_string(), NodeStatus::Active);
        self.heartbeat_count.insert(node_id.to_string(), 0);
    }

    /// Check if a node is healthy (can accept placements)
    fn is_node_healthy(&self, node_id: &str) -> bool {
        self.node_status
            .get(node_id)
            .map(|s| *s == NodeStatus::Active)
            .unwrap_or(false)
    }

    /// Get placement for an actor
    fn get_placement(&self, actor_id: &str) -> Option<String> {
        self.placements.get(actor_id).cloned()
    }

    /// Convert to SystemState for invariant checking
    fn to_system_state(&self) -> SystemState {
        let mut state = SystemState::new();

        // Add nodes with their statuses and actor states
        for (node_id, status) in &self.node_status {
            let mut node_info = NodeInfo::new(node_id.clone()).with_status(*status);

            // For each actor placed on this node, set it as Active
            for (actor_id, placed_node) in &self.placements {
                if placed_node == node_id {
                    node_info = node_info.with_actor_state(actor_id.clone(), NodeState::Active);
                }
            }

            state = state.with_node(node_info);
        }

        // Add placements
        for (actor_id, node_id) in &self.placements {
            state = state.with_placement(actor_id.clone(), node_id.clone());
            // Also set FDB holder for ConsistentHolder checks
            state = state.with_fdb_holder(actor_id.clone(), Some(node_id.clone()));
        }

        state
    }
}

/// Thread-safe wrapper for registry placement protocol
struct RegistryPlacementProtocol {
    state: Arc<RwLock<RegistryPlacementState>>,
}

impl RegistryPlacementProtocol {
    fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(RegistryPlacementState::new())),
        }
    }

    /// Initialize nodes in the cluster
    async fn init_cluster(&self, node_ids: &[&str]) {
        let mut state = self.state.write().await;
        for node_id in node_ids {
            state.add_node(node_id);
        }
    }

    /// TLA+ ClaimActor action: Attempt to place an actor on a node
    ///
    /// From KelpieRegistry.tla:
    /// ```tla
    /// ClaimActor(a, n) ==
    ///     /\ IsHealthy(n)
    ///     /\ isAlive[n] = TRUE
    ///     /\ IsUnplaced(a)
    ///     /\ placement' = [placement EXCEPT ![a] = n]
    /// ```
    async fn try_place_actor(&self, actor_id: &str, node_id: &str) -> Result<()> {
        // TigerStyle: Preconditions
        assert!(!actor_id.is_empty(), "actor_id cannot be empty");
        assert!(!node_id.is_empty(), "node_id cannot be empty");

        // Phase 1: Read current state (snapshot read for OCC)
        let read_version = {
            let state = self.state.read().await;
            state.version
        };

        // Yield to allow interleaving (critical for deterministic testing)
        tokio::task::yield_now().await;

        // Phase 2: Commit with OCC check
        let mut state = self.state.write().await;

        // OCC version check
        let current_version = state.version;
        if read_version != current_version {
            return Err(Error::Internal {
                message: format!(
                    "OCC conflict: read version {} != current version {}",
                    read_version, current_version
                ),
            });
        }

        // Check node is healthy (TLA+: IsHealthy(n))
        if !state.is_node_healthy(node_id) {
            return Err(Error::Internal {
                message: format!("Node {} is not healthy", node_id),
            });
        }

        // Check actor is not already placed (TLA+: IsUnplaced(a))
        if state.placements.contains_key(actor_id) {
            return Err(Error::ActorAlreadyExists {
                id: actor_id.to_string(),
            });
        }

        // SUCCESS: Place the actor
        state
            .placements
            .insert(actor_id.to_string(), node_id.to_string());
        state.version += 1;

        // TigerStyle: Postcondition
        debug_assert!(state.placements.get(actor_id) == Some(&node_id.to_string()));

        Ok(())
    }

    /// TLA+ ReleaseActor action: Remove actor placement
    async fn release_actor(&self, actor_id: &str) -> Result<()> {
        let mut state = self.state.write().await;

        if state.placements.remove(actor_id).is_none() {
            return Err(Error::ActorNotFound {
                id: actor_id.to_string(),
            });
        }

        state.version += 1;
        Ok(())
    }

    /// TLA+ DetectFailure action: Mark a node as failed and clear its placements
    ///
    /// From KelpieRegistry.tla:
    /// ```tla
    /// DetectFailure(n) ==
    ///     /\ nodeStatus[n] # Failed
    ///     /\ heartbeatCount[n] >= MaxHeartbeatMiss
    ///     /\ nodeStatus' = [nodeStatus EXCEPT ![n] = Failed]
    ///     /\ IF nodeStatus[n] = Suspect
    ///        THEN placement' = [a \in Actors |->
    ///                 IF placement[a] = n THEN NULL ELSE placement[a]]
    /// ```
    async fn fail_node(&self, node_id: &str) -> Result<()> {
        let mut state = self.state.write().await;

        // Check node exists
        if !state.node_status.contains_key(node_id) {
            return Err(Error::Internal {
                message: format!("Node {} does not exist", node_id),
            });
        }

        // Transition through Suspect to Failed (simplified: direct to Failed)
        state
            .node_status
            .insert(node_id.to_string(), NodeStatus::Failed);

        // Clear all placements on the failed node (TLA+ spec requirement)
        state.placements.retain(|_, n| n != node_id);

        state.version += 1;
        Ok(())
    }

    /// Recover a failed node (return to Active status)
    async fn recover_node(&self, node_id: &str) -> Result<()> {
        let mut state = self.state.write().await;

        if !state.node_status.contains_key(node_id) {
            return Err(Error::Internal {
                message: format!("Node {} does not exist", node_id),
            });
        }

        state
            .node_status
            .insert(node_id.to_string(), NodeStatus::Active);
        state.heartbeat_count.insert(node_id.to_string(), 0);
        state.version += 1;

        Ok(())
    }

    /// Verify TLA+ invariants against current state
    async fn verify_invariants(&self) -> std::result::Result<(), InvariantViolation> {
        let state = self.state.read().await;
        let sys_state = state.to_system_state();

        let checker = InvariantChecker::new()
            .with_invariant(SingleActivation)
            .with_invariant(PlacementConsistency);

        checker.verify_all(&sys_state)
    }

    /// Get current state for debugging
    async fn get_state(&self) -> RegistryPlacementState {
        self.state.read().await.clone()
    }
}

/// Verify TLA+ invariants helper function
///
/// This function checks both SingleActivation and PlacementConsistency
/// invariants from KelpieRegistry.tla against the given state.
#[allow(dead_code)] // Exported for use in other test files
fn verify_registry_tla_invariants(
    state: &RegistryPlacementState,
) -> std::result::Result<(), InvariantViolation> {
    let sys_state = state.to_system_state();

    let checker = InvariantChecker::new()
        .with_invariant(SingleActivation)
        .with_invariant(PlacementConsistency);

    checker.verify_all(&sys_state)
}

// =============================================================================
// SingleActivation Invariant Tests
// =============================================================================

/// Test: Concurrent placement attempts - only one succeeds
///
/// TLA+ Invariant: SingleActivation
/// ```tla
/// SingleActivation ==
///     \A a \in Actors :
///         Cardinality({n \in Nodes : placement[a] = n}) <= 1
/// ```
///
/// This test spawns N concurrent placement attempts for the SAME actor.
/// The invariant requires exactly 1 succeeds, N-1 fail.
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_registry_single_activation_invariant() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(seed = config.seed, "Running Registry SingleActivation test");

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let protocol = Arc::new(RegistryPlacementProtocol::new());
            let actor_id = "test/concurrent-target";
            const NUM_NODES: usize = 5;

            // Initialize cluster with N nodes
            let node_ids: Vec<String> = (0..NUM_NODES).map(|i| format!("node-{}", i)).collect();
            let node_refs: Vec<&str> = node_ids.iter().map(|s| s.as_str()).collect();
            protocol.init_cluster(&node_refs).await;

            // Spawn N concurrent placement attempts for the SAME actor
            let handles: Vec<_> = node_ids
                .iter()
                .map(|node_id| {
                    let protocol = protocol.clone();
                    let actor_id = actor_id.to_string();
                    let node_id = node_id.clone();
                    kelpie_core::current_runtime().spawn(async move {
                        let result = protocol.try_place_actor(&actor_id, &node_id).await;
                        (node_id, result)
                    })
                })
                .collect();

            // Wait for all attempts to complete
            let results: Vec<_> = join_all(handles)
                .await
                .into_iter()
                .map(|r| r.expect("task panicked"))
                .collect();

            // Count successes and failures
            let successes: Vec<_> = results
                .iter()
                .filter(|(_, r)| r.is_ok())
                .map(|(node, _)| node.clone())
                .collect();
            let failures: Vec<_> = results
                .iter()
                .filter(|(_, r)| r.is_err())
                .map(|(node, _)| node.clone())
                .collect();

            // TLA+ INVARIANT: SingleActivation - exactly 1 succeeds
            assert_eq!(
                successes.len(),
                1,
                "SingleActivation VIOLATED: {} placements succeeded (expected 1). \
                 Winners: {:?}, Failures: {:?}",
                successes.len(),
                successes,
                failures
            );

            // Verify invariants hold after the operation
            protocol
                .verify_invariants()
                .await
                .map_err(|e| Error::Internal {
                    message: format!("Invariant violation: {}", e),
                })?;

            tracing::info!(
                winner = ?successes.first(),
                "✅ Registry SingleActivation invariant held"
            );

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test: High contention - many nodes racing for same actor
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_registry_single_activation_high_contention() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(
        seed = config.seed,
        "Running high contention SingleActivation test"
    );

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let protocol = Arc::new(RegistryPlacementProtocol::new());
            let actor_id = "test/high-contention";
            const NUM_NODES: usize = 20; // High contention

            // Initialize cluster
            let node_ids: Vec<String> = (0..NUM_NODES).map(|i| format!("node-{}", i)).collect();
            let node_refs: Vec<&str> = node_ids.iter().map(|s| s.as_str()).collect();
            protocol.init_cluster(&node_refs).await;

            // Concurrent placements
            let handles: Vec<_> = node_ids
                .iter()
                .map(|node_id| {
                    let protocol = protocol.clone();
                    let actor_id = actor_id.to_string();
                    let node_id = node_id.clone();
                    kelpie_core::current_runtime()
                        .spawn(async move { protocol.try_place_actor(&actor_id, &node_id).await })
                })
                .collect();

            let results: Vec<_> = join_all(handles)
                .await
                .into_iter()
                .map(|r| r.expect("task panicked"))
                .collect();

            let successes = results.iter().filter(|r| r.is_ok()).count();

            // TLA+ INVARIANT: At most 1 succeeds
            assert!(
                successes <= 1,
                "SingleActivation VIOLATED: {} placements succeeded (expected <= 1)",
                successes
            );

            // With correct OCC, exactly 1 should succeed
            assert_eq!(
                successes, 1,
                "Expected exactly 1 success, got {}",
                successes
            );

            // Verify invariants
            protocol
                .verify_invariants()
                .await
                .map_err(|e| Error::Internal {
                    message: format!("Invariant violation: {}", e),
                })?;

            tracing::info!("✅ High contention SingleActivation test passed");
            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// PlacementConsistency Invariant Tests
// =============================================================================

/// Test: Actors are not placed on failed nodes
///
/// TLA+ Invariant: PlacementConsistency
/// ```tla
/// PlacementConsistency ==
///     \A a \in Actors :
///         placement[a] # NULL => nodeStatus[placement[a]] # Failed
/// ```
///
/// This test verifies that when a node fails, all its placements are cleared.
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_registry_placement_consistency_invariant() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(
        seed = config.seed,
        "Running Registry PlacementConsistency test"
    );

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let protocol = Arc::new(RegistryPlacementProtocol::new());

            // Initialize 3-node cluster
            protocol.init_cluster(&["node-1", "node-2", "node-3"]).await;

            // Place actors on different nodes
            protocol.try_place_actor("actor-1", "node-1").await?;
            protocol.try_place_actor("actor-2", "node-1").await?;
            protocol.try_place_actor("actor-3", "node-2").await?;
            protocol.try_place_actor("actor-4", "node-3").await?;

            // Verify invariants before failure
            protocol
                .verify_invariants()
                .await
                .map_err(|e| Error::Internal {
                    message: format!("Pre-failure invariant violation: {}", e),
                })?;

            // Fail node-1 (should clear actor-1 and actor-2 placements)
            protocol.fail_node("node-1").await?;

            // Verify PlacementConsistency: no actors on failed nodes
            protocol
                .verify_invariants()
                .await
                .map_err(|e| Error::Internal {
                    message: format!("Post-failure invariant violation: {}", e),
                })?;

            // Verify specific placements
            let state = protocol.get_state().await;
            assert!(
                state.get_placement("actor-1").is_none(),
                "actor-1 should be cleared after node-1 failure"
            );
            assert!(
                state.get_placement("actor-2").is_none(),
                "actor-2 should be cleared after node-1 failure"
            );
            assert_eq!(
                state.get_placement("actor-3"),
                Some("node-2".to_string()),
                "actor-3 should remain on node-2"
            );
            assert_eq!(
                state.get_placement("actor-4"),
                Some("node-3".to_string()),
                "actor-4 should remain on node-3"
            );

            tracing::info!("✅ Registry PlacementConsistency invariant held");
            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test: Cannot place actors on failed nodes
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_registry_no_placement_on_failed_node() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let protocol = Arc::new(RegistryPlacementProtocol::new());

            // Initialize cluster
            protocol.init_cluster(&["node-1", "node-2"]).await;

            // Fail node-1
            protocol.fail_node("node-1").await?;

            // Attempt to place actor on failed node should fail
            let result = protocol.try_place_actor("actor-1", "node-1").await;
            assert!(
                result.is_err(),
                "Should not be able to place actor on failed node"
            );

            // Placement on healthy node should succeed
            protocol.try_place_actor("actor-1", "node-2").await?;

            // Verify invariants
            protocol
                .verify_invariants()
                .await
                .map_err(|e| Error::Internal {
                    message: format!("Invariant violation: {}", e),
                })?;

            tracing::info!("✅ No placement on failed node test passed");
            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Node Failure and Recovery Tests
// =============================================================================

/// Test: Node recovery allows new placements
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_registry_node_recovery() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let protocol = Arc::new(RegistryPlacementProtocol::new());

            // Initialize cluster
            protocol.init_cluster(&["node-1", "node-2"]).await;

            // Place actor on node-1
            protocol.try_place_actor("actor-1", "node-1").await?;

            // Fail node-1 (clears placement)
            protocol.fail_node("node-1").await?;

            let state = protocol.get_state().await;
            assert!(
                state.get_placement("actor-1").is_none(),
                "Placement should be cleared after node failure"
            );

            // Recover node-1
            protocol.recover_node("node-1").await?;

            // Should be able to place actors on recovered node
            protocol.try_place_actor("actor-1", "node-1").await?;

            let state = protocol.get_state().await;
            assert_eq!(
                state.get_placement("actor-1"),
                Some("node-1".to_string()),
                "Should be able to place on recovered node"
            );

            // Verify invariants
            protocol
                .verify_invariants()
                .await
                .map_err(|e| Error::Internal {
                    message: format!("Invariant violation: {}", e),
                })?;

            tracing::info!("✅ Node recovery test passed");
            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test: Concurrent placement race after node failure
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_registry_placement_race_after_failure() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let protocol = Arc::new(RegistryPlacementProtocol::new());
            const NUM_NODES: usize = 5;

            // Initialize cluster
            let node_ids: Vec<String> = (0..NUM_NODES).map(|i| format!("node-{}", i)).collect();
            let node_refs: Vec<&str> = node_ids.iter().map(|s| s.as_str()).collect();
            protocol.init_cluster(&node_refs).await;

            // Place actor on node-0
            protocol.try_place_actor("actor-1", "node-0").await?;

            // Fail node-0 (clears placement)
            protocol.fail_node("node-0").await?;

            // Multiple nodes race to take over the now-unplaced actor
            let handles: Vec<_> = node_ids[1..] // Skip failed node-0
                .iter()
                .map(|node_id| {
                    let protocol = protocol.clone();
                    let node_id = node_id.clone();
                    kelpie_core::current_runtime()
                        .spawn(async move { protocol.try_place_actor("actor-1", &node_id).await })
                })
                .collect();

            let results: Vec<_> = join_all(handles)
                .await
                .into_iter()
                .map(|r| r.expect("task panicked"))
                .collect();

            let successes = results.iter().filter(|r| r.is_ok()).count();

            // TLA+ INVARIANT: Exactly 1 should succeed in reclaiming
            assert_eq!(
                successes, 1,
                "SingleActivation VIOLATED during recovery: {} succeeded",
                successes
            );

            // Verify invariants
            protocol
                .verify_invariants()
                .await
                .map_err(|e| Error::Internal {
                    message: format!("Invariant violation: {}", e),
                })?;

            tracing::info!("✅ Placement race after failure test passed");
            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Fault Injection Tests
// =============================================================================

/// Test: SingleActivation under storage faults
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_registry_single_activation_with_storage_faults() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(
        seed = config.seed,
        "Running SingleActivation with storage faults"
    );

    // Note: Storage faults affect the underlying SimStorage, but our
    // RegistryPlacementProtocol uses in-memory state. This test demonstrates
    // the pattern for fault injection even though the protocol itself isn't
    // affected by storage faults.
    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.1))
        .run_async(|_env| async move {
            let protocol = Arc::new(RegistryPlacementProtocol::new());
            let actor_id = "test/storage-fault";
            const NUM_NODES: usize = 5;

            // Initialize cluster
            let node_ids: Vec<String> = (0..NUM_NODES).map(|i| format!("node-{}", i)).collect();
            let node_refs: Vec<&str> = node_ids.iter().map(|s| s.as_str()).collect();
            protocol.init_cluster(&node_refs).await;

            // Concurrent placements
            let handles: Vec<_> = node_ids
                .iter()
                .map(|node_id| {
                    let protocol = protocol.clone();
                    let actor_id = actor_id.to_string();
                    let node_id = node_id.clone();
                    kelpie_core::current_runtime()
                        .spawn(async move { protocol.try_place_actor(&actor_id, &node_id).await })
                })
                .collect();

            let results: Vec<_> = join_all(handles)
                .await
                .into_iter()
                .map(|r| r.expect("task panicked"))
                .collect();

            let successes = results.iter().filter(|r| r.is_ok()).count();

            // TLA+ INVARIANT: At most 1 succeeds (with faults, might be 0)
            assert!(
                successes <= 1,
                "SingleActivation VIOLATED under storage faults: {} succeeded",
                successes
            );

            // Verify invariants
            protocol
                .verify_invariants()
                .await
                .map_err(|e| Error::Internal {
                    message: format!("Invariant violation: {}", e),
                })?;

            tracing::info!(
                successes = successes,
                "✅ SingleActivation held under storage faults"
            );
            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test: PlacementConsistency under network partition
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_registry_placement_consistency_with_partition() {
    let config = SimConfig::from_env_or_random();
    tracing::info!(
        seed = config.seed,
        "Running PlacementConsistency with network partition"
    );

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::NetworkPartition, 0.5))
        .run_async(|_env| async move {
            let protocol = Arc::new(RegistryPlacementProtocol::new());

            // Initialize 3-node cluster
            protocol.init_cluster(&["node-1", "node-2", "node-3"]).await;

            // Place actors
            protocol.try_place_actor("actor-1", "node-1").await?;
            protocol.try_place_actor("actor-2", "node-2").await?;

            // Simulate partition by failing node-2
            protocol.fail_node("node-2").await?;

            // Verify PlacementConsistency holds
            protocol
                .verify_invariants()
                .await
                .map_err(|e| Error::Internal {
                    message: format!("Invariant violation: {}", e),
                })?;

            // actor-2 should be cleared
            let state = protocol.get_state().await;
            assert!(
                state.get_placement("actor-2").is_none(),
                "actor-2 should be cleared after node-2 partition"
            );

            tracing::info!("✅ PlacementConsistency held under network partition");
            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Determinism Tests
// =============================================================================

/// Test: Same seed produces same placement winner
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_registry_placement_deterministic() {
    let seed = 42_u64;

    let run_test = || async {
        let config = SimConfig::new(seed);

        Simulation::new(config)
            .run_async(|_env| async move {
                let protocol = Arc::new(RegistryPlacementProtocol::new());
                const NUM_NODES: usize = 5;

                let node_ids: Vec<String> = (0..NUM_NODES).map(|i| format!("node-{}", i)).collect();
                let node_refs: Vec<&str> = node_ids.iter().map(|s| s.as_str()).collect();
                protocol.init_cluster(&node_refs).await;

                let handles: Vec<_> = node_ids
                    .iter()
                    .map(|node_id| {
                        let protocol = protocol.clone();
                        let node_id = node_id.clone();
                        kelpie_core::current_runtime().spawn(async move {
                            let result = protocol
                                .try_place_actor("test/deterministic", &node_id)
                                .await;
                            (node_id, result.is_ok())
                        })
                    })
                    .collect();

                let results: Vec<_> = join_all(handles)
                    .await
                    .into_iter()
                    .map(|r| r.expect("task panicked"))
                    .collect();

                // Find the winner
                let winner: Option<String> = results
                    .iter()
                    .find(|(_, won)| *won)
                    .map(|(name, _)| name.clone());

                Ok(winner)
            })
            .await
    };

    let result1 = run_test().await.expect("First run failed");
    let result2 = run_test().await.expect("Second run failed");

    assert_eq!(
        result1, result2,
        "Determinism violated: winner differs with same seed. \
         Run 1: {:?}, Run 2: {:?}",
        result1, result2
    );
}

// =============================================================================
// Invariant Verification Every Operation Test
// =============================================================================

/// Test: Verify TLA+ invariants after EVERY operation
///
/// This test demonstrates the pattern of checking invariants after each
/// state-changing operation, as recommended in the issue.
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_registry_invariants_verified_every_operation() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let protocol = Arc::new(RegistryPlacementProtocol::new());

            // Operation 1: Initialize cluster
            protocol.init_cluster(&["node-1", "node-2", "node-3"]).await;
            protocol
                .verify_invariants()
                .await
                .map_err(|e| Error::Internal {
                    message: format!("After init: {}", e),
                })?;

            // Operation 2: Place actor-1
            protocol.try_place_actor("actor-1", "node-1").await?;
            protocol
                .verify_invariants()
                .await
                .map_err(|e| Error::Internal {
                    message: format!("After place actor-1: {}", e),
                })?;

            // Operation 3: Place actor-2
            protocol.try_place_actor("actor-2", "node-2").await?;
            protocol
                .verify_invariants()
                .await
                .map_err(|e| Error::Internal {
                    message: format!("After place actor-2: {}", e),
                })?;

            // Operation 4: Fail node-1
            protocol.fail_node("node-1").await?;
            protocol
                .verify_invariants()
                .await
                .map_err(|e| Error::Internal {
                    message: format!("After fail node-1: {}", e),
                })?;

            // Operation 5: Recover node-1
            protocol.recover_node("node-1").await?;
            protocol
                .verify_invariants()
                .await
                .map_err(|e| Error::Internal {
                    message: format!("After recover node-1: {}", e),
                })?;

            // Operation 6: Place new actor on recovered node
            protocol.try_place_actor("actor-3", "node-1").await?;
            protocol
                .verify_invariants()
                .await
                .map_err(|e| Error::Internal {
                    message: format!("After place actor-3: {}", e),
                })?;

            // Operation 7: Release actor
            protocol.release_actor("actor-3").await?;
            protocol
                .verify_invariants()
                .await
                .map_err(|e| Error::Internal {
                    message: format!("After release actor-3: {}", e),
                })?;

            tracing::info!("✅ All operations verified against TLA+ invariants");
            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}
