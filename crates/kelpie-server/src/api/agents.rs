//! Agent API endpoints
//!
//! TigerStyle: Letta-compatible agent CRUD operations.

use crate::api::ApiError;
use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use kelpie_core::Runtime;
use kelpie_server::models::{AgentState, CreateAgentRequest, ListResponse, UpdateAgentRequest};
use kelpie_server::state::AppState;
use serde::{Deserialize, Serialize};
use tracing::instrument;

/// Query parameters for listing agents
#[derive(Debug, Deserialize)]
pub struct ListAgentsQuery {
    /// Maximum number of agents to return
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Cursor for pagination (Kelpie's native parameter)
    pub cursor: Option<String>,
    /// Cursor for pagination (Letta SDK compatibility - alias for cursor)
    /// The Letta SDK uses ?after={last_item_id} for pagination
    pub after: Option<String>,
    /// Filter by project ID
    pub project_id: Option<String>,
    /// Filter by agent name (exact match)
    pub name: Option<String>,
}

fn default_limit() -> usize {
    50
}

/// Maximum limit for list operations
const LIST_LIMIT_MAX: usize = 100;

// ============================================================================
// Batch Models
// ============================================================================

#[derive(Debug, Deserialize)]
struct BatchCreateAgentsRequest {
    agents: Vec<CreateAgentRequest>,
}

#[derive(Debug, Serialize)]
struct BatchAgentsResponse {
    results: Vec<BatchAgentResult>,
}

#[derive(Debug, Serialize)]
struct BatchAgentResult {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<AgentState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BatchDeleteAgentsRequest {
    agent_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct BatchDeleteAgentsResponse {
    results: Vec<BatchDeleteAgentResult>,
}

#[derive(Debug, Serialize)]
struct BatchDeleteAgentResult {
    agent_id: String,
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Create agent routes
pub fn router<R: Runtime + 'static>() -> Router<AppState<R>> {
    Router::new()
        .route("/", get(list_agents).post(create_agent))
        .route(
            "/batch",
            axum::routing::post(create_agents_batch).delete(delete_agents_batch),
        )
        .route(
            "/import",
            axum::routing::post(super::import_export::import_agent),
        )
        .route(
            "/:agent_id",
            get(get_agent).patch(update_agent).delete(delete_agent),
        )
        .route("/:agent_id/export", get(super::import_export::export_agent))
        // Nested block routes
        .route("/:agent_id/blocks", get(super::blocks::list_blocks))
        // Smart route: supports both UUID (block_id) and label access
        // This provides Letta compatibility for /blocks/{label} paths
        .route(
            "/:agent_id/blocks/:id_or_label",
            get(super::blocks::get_block_or_label).patch(super::blocks::update_block_or_label),
        )
        // Core memory routes (Letta SDK compatibility)
        // The SDK uses /core-memory/blocks for listing and /core-memory/blocks/{label} for access
        .route(
            "/:agent_id/core-memory/blocks",
            get(super::blocks::list_blocks),
        )
        .route(
            "/:agent_id/core-memory/blocks/:label",
            get(super::blocks::get_block_by_label).patch(super::blocks::update_block_by_label),
        )
        // Nested message routes
        .route(
            "/:agent_id/messages",
            get(super::messages::list_messages).post(super::messages::send_message),
        )
        .route(
            "/:agent_id/messages/batch",
            axum::routing::post(super::messages::send_messages_batch),
        )
        .route(
            "/:agent_id/messages/batch/:batch_id",
            axum::routing::get(super::messages::get_batch_status),
        )
        // Streaming message route (letta-code SSE)
        .route(
            "/:agent_id/messages/stream",
            axum::routing::post(super::streaming::send_message_stream),
        )
        // Nested archival routes
        .route(
            "/:agent_id/archival",
            get(super::archival::search_archival).post(super::archival::add_archival),
        )
        .route(
            "/:agent_id/archival/:entry_id",
            get(super::archival::get_archival_entry).delete(super::archival::delete_archival_entry),
        )
        // Tool attachment routes (Letta SDK compatibility)
        .route("/:agent_id/tools", get(list_agent_tools))
        .route(
            "/:agent_id/tools/:tool_id",
            axum::routing::post(attach_tool).delete(detach_tool),
        )
}

/// Create a new agent
///
/// POST /v1/agents
#[instrument(skip(state, request), fields(agent_name = %request.name), level = "info")]
async fn create_agent<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Json(request): Json<CreateAgentRequest>,
) -> Result<Json<AgentState>, ApiError> {
    // Validate request
    if request.name.is_empty() {
        return Err(ApiError::bad_request("agent name cannot be empty"));
    }

    if request.name.len() > 256 {
        return Err(ApiError::bad_request(
            "agent name too long (max 256 characters)",
        ));
    }

    // Extract block_ids before consuming request
    let block_ids = request.block_ids.clone();

    // Create agent via dual-mode method
    let mut created = state.create_agent_async(request).await?;

    // TigerStyle: Persist agent metadata to storage BEFORE returning
    // This ensures the agent is visible to list operations immediately after creation
    // Fix for race condition: persist must complete before returning
    state
        .persist_agent(&created)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to persist agent metadata: {}", e)))?;

    // Look up and attach standalone blocks by ID (letta-code compatibility)
    // Note: This is a temporary workaround until standalone blocks are integrated into the actor model
    for block_id in block_ids {
        if let Ok(Some(block)) = state.get_standalone_block(&block_id) {
            created.blocks.push(block);
        } else {
            tracing::warn!(block_id = %block_id, "standalone block not found, skipping");
        }
    }

    // TigerStyle: Validate tool references (MCP tools already registered at server creation)
    if !created.tool_ids.is_empty() {
        tracing::debug!(
            agent_id = %created.id,
            tool_ids = ?created.tool_ids,
            "Agent created with tool references"
        );

        // Validation: Check if MCP tools exist in registry
        let registry = state.tool_registry();
        for tool_id in &created.tool_ids {
            if tool_id.starts_with("mcp_") {
                if registry.has_tool(tool_id).await {
                    tracing::debug!(
                        agent_id = %created.id,
                        tool_id = %tool_id,
                        "MCP tool reference validated"
                    );
                } else {
                    tracing::warn!(
                        agent_id = %created.id,
                        tool_id = %tool_id,
                        "Referenced MCP tool not found in registry (server may need to be created first)"
                    );
                }
            }
        }
    }

    tracing::info!(agent_id = %created.id, name = %created.name, block_count = created.blocks.len(), "created agent");
    Ok(Json(created))
}

/// Query parameters for getting an agent
#[derive(Debug, Deserialize, Default)]
pub struct GetAgentQuery {
    /// Include related resources (e.g., "agent.tools")
    #[serde(default)]
    pub include: Option<String>,
}

/// Get an agent by ID
///
/// GET /v1/agents/{agent_id}
/// GET /v1/agents/{agent_id}?include=agent.tools
#[instrument(skip(state, query), fields(agent_id = %agent_id), level = "info")]
async fn get_agent<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(agent_id): Path<String>,
    Query(query): Query<GetAgentQuery>,
) -> Result<Json<AgentStateWithTools>, ApiError> {
    let agent = state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Agent", &agent_id))?;

    // If include=agent.tools, fetch and attach tool details
    let tools = if query.include.as_deref() == Some("agent.tools") {
        let mut tool_list = Vec::new();
        for tool_id in &agent.tool_ids {
            // Try to find tool by ID first, then by name
            if let Some(tool) = state.get_tool_by_id(tool_id).await {
                tool_list.push(super::tools::ToolResponse::from(tool));
            } else if let Some(tool) = state.get_tool(tool_id).await {
                tool_list.push(super::tools::ToolResponse::from(tool));
            }
        }
        Some(tool_list)
    } else {
        None
    };

    Ok(Json(AgentStateWithTools { agent, tools }))
}

/// Agent state with optional tool details
#[derive(Debug, Serialize)]
pub struct AgentStateWithTools {
    #[serde(flatten)]
    pub agent: AgentState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<super::tools::ToolResponse>>,
}

/// List tools attached to an agent
///
/// GET /v1/agents/{agent_id}/tools
#[instrument(skip(state), fields(agent_id = %agent_id), level = "info")]
async fn list_agent_tools<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(agent_id): Path<String>,
) -> Result<Json<Vec<super::tools::ToolResponse>>, ApiError> {
    let agent = state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Agent", &agent_id))?;

    let mut tools = Vec::new();
    for tool_id in &agent.tool_ids {
        if let Some(tool) = state.get_tool_by_id(tool_id).await {
            tools.push(super::tools::ToolResponse::from(tool));
        } else if let Some(tool) = state.get_tool(tool_id).await {
            tools.push(super::tools::ToolResponse::from(tool));
        }
    }

    Ok(Json(tools))
}

/// Attach a tool to an agent
///
/// POST /v1/agents/{agent_id}/tools/{tool_id}
#[instrument(skip(state), fields(agent_id = %agent_id, tool_id = %tool_id), level = "info")]
async fn attach_tool<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path((agent_id, tool_id)): Path<(String, String)>,
) -> Result<Json<super::tools::ToolResponse>, ApiError> {
    // Get the tool (by ID or name)
    let tool = state
        .get_tool_by_id(&tool_id)
        .await
        .or(state.get_tool(&tool_id).await)
        .ok_or_else(|| ApiError::not_found("Tool", &tool_id))?;

    // Get the agent and check if tool is already attached
    let agent = state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Agent", &agent_id))?;

    // Add tool ID if not already attached
    let tool_id_to_add = tool.id.clone();
    if !agent.tool_ids.contains(&tool_id_to_add) {
        let mut new_tool_ids = agent.tool_ids.clone();
        new_tool_ids.push(tool_id_to_add);

        let update = serde_json::json!({
            "tool_ids": new_tool_ids
        });
        state.update_agent_async(&agent_id, update).await?;
    }

    tracing::info!(agent_id = %agent_id, tool_id = %tool.id, tool_name = %tool.name, "attached tool to agent");
    Ok(Json(super::tools::ToolResponse::from(tool)))
}

/// Detach a tool from an agent
///
/// DELETE /v1/agents/{agent_id}/tools/{tool_id}
#[instrument(skip(state), fields(agent_id = %agent_id, tool_id = %tool_id), level = "info")]
async fn detach_tool<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path((agent_id, tool_id)): Path<(String, String)>,
) -> Result<(), ApiError> {
    // Verify agent exists
    let agent = state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Agent", &agent_id))?;

    // Remove tool ID
    let new_tool_ids: Vec<String> = agent
        .tool_ids
        .into_iter()
        .filter(|id| id != &tool_id)
        .collect();

    let update = serde_json::json!({
        "tool_ids": new_tool_ids
    });
    state.update_agent_async(&agent_id, update).await?;

    tracing::info!(agent_id = %agent_id, tool_id = %tool_id, "detached tool from agent");
    Ok(())
}

/// List all agents with pagination
///
/// GET /v1/agents
///
/// Supports both Kelpie's `cursor` and Letta SDK's `after` parameters for pagination.
#[instrument(skip(state, query), fields(limit = query.limit, cursor = ?query.cursor, after = ?query.after), level = "info")]
async fn list_agents<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Query(query): Query<ListAgentsQuery>,
) -> Result<Json<ListResponse<AgentState>>, ApiError> {
    let limit = query.limit.min(LIST_LIMIT_MAX);

    // Support both Kelpie's `cursor` and Letta SDK's `after` parameter
    // The `after` parameter is what the Letta SDK uses for pagination
    let pagination_cursor = query.cursor.as_deref().or(query.after.as_deref());

    // TigerStyle: Apply name filter in storage query before pagination
    // This ensures correct results when paginating filtered lists
    let name_filter = query.name.as_deref();

    let (items, cursor, total) = if let Some(project_id) = query.project_id.as_deref() {
        let mut agents = state.list_agents_by_project(project_id)?;

        // Apply name filter BEFORE pagination
        if let Some(name) = name_filter {
            agents.retain(|agent| agent.name == name);
        }

        agents.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        let total = agents.len();

        let start_idx = if let Some(cursor_id) = pagination_cursor {
            agents
                .iter()
                .position(|a| a.id == cursor_id)
                .map(|i| i + 1)
                .unwrap_or(0)
        } else {
            0
        };

        let page: Vec<_> = agents.into_iter().skip(start_idx).take(limit + 1).collect();

        let (items, next_cursor) = if page.len() > limit {
            let items: Vec<_> = page.into_iter().take(limit).collect();
            let next_cursor = items.last().map(|a| a.id.clone());
            (items, next_cursor)
        } else {
            (page, None)
        };

        (items, next_cursor, total)
    } else {
        let (items, cursor) = state
            .list_agents_async(limit, pagination_cursor, name_filter)
            .await?;
        let total = state.agent_count()?;
        (items, cursor, total)
    };

    Ok(Json(ListResponse {
        items,
        total,
        cursor,
    }))
}

/// Batch create agents
#[instrument(skip(state, request), level = "info")]
async fn create_agents_batch<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Json(request): Json<BatchCreateAgentsRequest>,
) -> Result<Json<BatchAgentsResponse>, ApiError> {
    if request.agents.is_empty() {
        return Err(ApiError::bad_request("agents batch cannot be empty"));
    }

    let mut results = Vec::with_capacity(request.agents.len());
    for agent in request.agents {
        match state.create_agent_async(agent).await {
            Ok(created) => results.push(BatchAgentResult {
                success: true,
                agent_id: Some(created.id.clone()),
                agent: Some(created),
                error: None,
            }),
            Err(e) => results.push(BatchAgentResult {
                success: false,
                agent_id: None,
                agent: None,
                error: Some(e.to_string()),
            }),
        }
    }

    Ok(Json(BatchAgentsResponse { results }))
}

/// Batch delete agents
#[instrument(skip(state, request), level = "info")]
async fn delete_agents_batch<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Json(request): Json<BatchDeleteAgentsRequest>,
) -> Result<Json<BatchDeleteAgentsResponse>, ApiError> {
    if request.agent_ids.is_empty() {
        return Err(ApiError::bad_request("agent_ids cannot be empty"));
    }

    let mut results = Vec::with_capacity(request.agent_ids.len());
    for agent_id in request.agent_ids {
        match state.delete_agent_async(&agent_id).await {
            Ok(_) => results.push(BatchDeleteAgentResult {
                agent_id,
                success: true,
                error: None,
            }),
            Err(e) => results.push(BatchDeleteAgentResult {
                agent_id,
                success: false,
                error: Some(e.to_string()),
            }),
        }
    }

    Ok(Json(BatchDeleteAgentsResponse { results }))
}

/// Update an agent
///
/// PATCH /v1/agents/{agent_id}
#[instrument(skip(state, request), fields(agent_id = %agent_id), level = "info")]
async fn update_agent<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(agent_id): Path<String>,
    Json(request): Json<UpdateAgentRequest>,
) -> Result<Json<AgentState>, ApiError> {
    // Validate if name is being updated
    if let Some(ref name) = request.name {
        if name.is_empty() {
            return Err(ApiError::bad_request("agent name cannot be empty"));
        }
        if name.len() > 256 {
            return Err(ApiError::bad_request(
                "agent name too long (max 256 characters)",
            ));
        }
    }

    let update_value = serde_json::to_value(request)
        .map_err(|e| ApiError::bad_request(format!("Invalid update request: {}", e)))?;

    let updated = state.update_agent_async(&agent_id, update_value).await?;

    tracing::info!(agent_id = %updated.id, "updated agent");
    Ok(Json(updated))
}

/// Delete an agent
///
/// DELETE /v1/agents/{agent_id}
#[instrument(skip(state), fields(agent_id = %agent_id), level = "info")]
async fn delete_agent<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(agent_id): Path<String>,
) -> Result<(), ApiError> {
    state.delete_agent_async(&agent_id).await?;
    tracing::info!(agent_id = %agent_id, "deleted agent");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use kelpie_core::Runtime;
    use kelpie_dst::{DeterministicRng, FaultInjector, SimStorage};
    use kelpie_runtime::{CloneFactory, Dispatcher, DispatcherConfig};
    use kelpie_server::actor::{AgentActor, AgentActorState, LlmClient, LlmMessage, LlmResponse};
    use kelpie_server::service;
    use kelpie_server::tools::UnifiedToolRegistry;
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Mock LLM client for testing that returns simple responses
    struct MockLlmClient;

    #[async_trait]
    impl LlmClient for MockLlmClient {
        async fn complete_with_tools(
            &self,
            _messages: Vec<LlmMessage>,
            _tools: Vec<kelpie_server::llm::ToolDefinition>,
        ) -> kelpie_core::Result<LlmResponse> {
            Ok(LlmResponse {
                content: "Test response".to_string(),
                tool_calls: vec![],
                prompt_tokens: 0,
                completion_tokens: 0,
                stop_reason: "end_turn".to_string(),
            })
        }

        async fn continue_with_tool_result(
            &self,
            _messages: Vec<LlmMessage>,
            _tools: Vec<kelpie_server::llm::ToolDefinition>,
            _assistant_blocks: Vec<kelpie_server::llm::ContentBlock>,
            _tool_results: Vec<(String, String)>,
        ) -> kelpie_core::Result<LlmResponse> {
            Ok(LlmResponse {
                content: "Test response".to_string(),
                tool_calls: vec![],
                prompt_tokens: 0,
                completion_tokens: 0,
                stop_reason: "end_turn".to_string(),
            })
        }
    }

    /// Create a test AppState with AgentService for API testing
    async fn test_app() -> Router {
        // Create a minimal AgentService setup for testing
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient);
        let actor = AgentActor::new(llm, Arc::new(UnifiedToolRegistry::new()));
        let factory = Arc::new(CloneFactory::new(actor));

        // Use SimStorage for testing (in-memory KV store)
        let rng = DeterministicRng::new(42);
        let faults = Arc::new(FaultInjector::new(rng.fork()));
        let storage = SimStorage::new(rng.fork(), faults);
        let kv = Arc::new(storage);

        let runtime = kelpie_core::TokioRuntime;

        let mut dispatcher = Dispatcher::<AgentActor, AgentActorState, _>::new(
            factory,
            kv,
            DispatcherConfig::default(),
            runtime.clone(),
        );
        let handle = dispatcher.handle();

        drop(runtime.spawn(async move {
            dispatcher.run().await;
        }));

        let service = service::AgentService::new(handle.clone());
        let state = AppState::with_agent_service(runtime, service, handle);

        api::router(state)
    }

    #[tokio::test]
    async fn test_create_agent() {
        let app = test_app().await;

        let body = serde_json::json!({
            "name": "test-agent",
            "memory_blocks": [{
                "label": "persona",
                "value": "I am a test agent"
            }]
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let agent: AgentState = serde_json::from_slice(&body).unwrap();
        assert_eq!(agent.name, "test-agent");
        assert!(!agent.id.is_empty());
    }

    #[tokio::test]
    async fn test_create_agent_empty_name() {
        let app = test_app().await;

        let body = serde_json::json!({
            "name": "",
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_agent_not_found() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/agents/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_health_check() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let health: kelpie_server::models::HealthResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(health.status, "ok");
    }

    // ============================================================================
    // Phase 5: Persistence Verification Tests
    // ============================================================================

    #[tokio::test]
    async fn test_agent_roundtrip_all_fields() {
        let app = test_app().await;

        // Create agent with ALL fields populated
        let create_body = serde_json::json!({
            "name": "roundtrip-agent",
            "description": "A test agent for round-trip verification",
            "system": "You are a helpful assistant",
            "agent_type": "letta_v1_agent",
            "memory_blocks": [
                {"label": "persona", "value": "I am a test persona"},
                {"label": "human", "value": "The user is testing"}
            ],
            "tool_ids": ["tool-1", "tool-2"],
            "tags": ["test", "roundtrip", "verification"],
            "metadata": {"key1": "value1", "key2": "value2"}
        });

        // Create the agent
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: AgentState = serde_json::from_slice(&body).unwrap();

        // Verify all fields in create response
        assert_eq!(created.name, "roundtrip-agent");
        assert_eq!(
            created.description.as_deref(),
            Some("A test agent for round-trip verification")
        );
        assert_eq!(
            created.system.as_deref(),
            Some("You are a helpful assistant")
        );
        assert_eq!(created.blocks.len(), 2);
        assert_eq!(created.tool_ids, vec!["tool-1", "tool-2"]);
        assert_eq!(created.tags, vec!["test", "roundtrip", "verification"]);
        assert_eq!(
            created.metadata.get("key1").and_then(|v| v.as_str()),
            Some("value1")
        );

        // Read the agent back
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/agents/{}", created.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let fetched: AgentState = serde_json::from_slice(&body).unwrap();

        // Verify ALL fields match after round-trip
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.name, created.name);
        assert_eq!(fetched.description, created.description);
        assert_eq!(fetched.system, created.system);
        assert_eq!(fetched.blocks.len(), created.blocks.len());
        for (fetched_block, created_block) in fetched.blocks.iter().zip(created.blocks.iter()) {
            assert_eq!(fetched_block.label, created_block.label);
            assert_eq!(fetched_block.value, created_block.value);
        }
        assert_eq!(fetched.tool_ids, created.tool_ids);
        assert_eq!(fetched.tags, created.tags);
        assert_eq!(fetched.metadata, created.metadata);
    }

    #[tokio::test]
    async fn test_agent_update_persists() {
        let app = test_app().await;

        // Create an agent
        let create_body = serde_json::json!({
            "name": "update-test-agent",
            "description": "Original description",
            "tags": ["original"]
        });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: AgentState = serde_json::from_slice(&body).unwrap();
        let agent_id = created.id.clone();

        // Update the agent (uses PATCH, not PUT)
        let update_body = serde_json::json!({
            "name": "updated-agent-name",
            "description": "Updated description",
            "tags": ["updated", "modified"]
        });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/v1/agents/{}", agent_id))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&update_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Read the agent back to verify update persisted
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/agents/{}", agent_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let fetched: AgentState = serde_json::from_slice(&body).unwrap();

        // Verify updates persisted
        assert_eq!(fetched.name, "updated-agent-name");
        assert_eq!(fetched.description.as_deref(), Some("Updated description"));
        assert_eq!(fetched.tags, vec!["updated", "modified"]);
    }

    #[tokio::test]
    async fn test_agent_delete_removes_from_storage() {
        let app = test_app().await;

        // Create an agent
        let create_body = serde_json::json!({
            "name": "delete-test-agent"
        });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: AgentState = serde_json::from_slice(&body).unwrap();
        let agent_id = created.id.clone();

        // Verify agent exists
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/agents/{}", agent_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Delete the agent
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/v1/agents/{}", agent_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Verify agent is gone
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/agents/{}", agent_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
