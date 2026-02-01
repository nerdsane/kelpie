//! Memory block API endpoints
//!
//! TigerStyle: Letta-compatible memory block operations.

use crate::api::ApiError;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use kelpie_core::Runtime;
use kelpie_server::models::{Block, UpdateBlockRequest};
use kelpie_server::state::AppState;
use serde::Deserialize;
use tracing::instrument;
use uuid;

/// Query parameters for listing blocks (Letta SDK compatibility)
#[derive(Debug, Deserialize, Default)]
pub struct ListBlocksParams {
    /// Cursor for pagination (Letta SDK uses 'after' with last item ID)
    pub after: Option<String>,
    /// Limit number of results
    pub limit: Option<usize>,
}

/// List all blocks for an agent
///
/// GET /v1/agents/{agent_id}/blocks
/// GET /v1/agents/{agent_id}/core-memory/blocks
///
/// Supports Letta SDK pagination via `after` parameter.
#[instrument(skip(state), fields(agent_id = %agent_id, after = ?query.after), level = "info")]
pub async fn list_blocks<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(agent_id): Path<String>,
    Query(query): Query<ListBlocksParams>,
) -> Result<Json<Vec<Block>>, ApiError> {
    // Get agent from actor system (AgentService required)
    let agent = state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Agent", &agent_id))?;

    // Handle Letta SDK pagination via 'after' parameter
    // The SDK passes the last item's ID and expects items AFTER that ID
    // When we've returned all items, we should return an empty list
    let blocks = if let Some(after_id) = query.after.as_ref() {
        // Find the position of the 'after' block
        let start_idx = agent
            .blocks
            .iter()
            .position(|b| &b.id == after_id)
            .map(|i| i + 1) // Start after this block
            .unwrap_or(agent.blocks.len()); // If not found, return empty

        agent.blocks.into_iter().skip(start_idx).collect()
    } else {
        agent.blocks
    };

    // Apply limit if specified
    let blocks = if let Some(limit) = query.limit {
        blocks.into_iter().take(limit).collect()
    } else {
        blocks
    };

    Ok(Json(blocks))
}

/// Get a specific block
///
/// GET /v1/agents/{agent_id}/blocks/{block_id}
#[instrument(skip(state), fields(agent_id = %agent_id, block_id = %block_id), level = "info")]
pub async fn get_block<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path((agent_id, block_id)): Path<(String, String)>,
) -> Result<Json<Block>, ApiError> {
    // Get agent from actor system (AgentService required)
    let agent = state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Agent", &agent_id))?;

    // Find the block by ID
    let block = agent
        .blocks
        .iter()
        .find(|b| b.id == block_id)
        .cloned()
        .ok_or_else(|| ApiError::not_found("Block", &block_id))?;

    Ok(Json(block))
}

/// Update a block
///
/// PATCH /v1/agents/{agent_id}/blocks/{block_id}
#[instrument(skip(state, request), fields(agent_id = %agent_id, block_id = %block_id), level = "info")]
pub async fn update_block<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path((agent_id, block_id)): Path<(String, String)>,
    Json(request): Json<UpdateBlockRequest>,
) -> Result<Json<Block>, ApiError> {
    // Get agent from actor system (AgentService required)
    let agent = state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Agent", &agent_id))?;

    // Find the block by ID to get its label
    let block = agent
        .blocks
        .iter()
        .find(|b| b.id == block_id)
        .ok_or_else(|| ApiError::not_found("Block", &block_id))?;

    let label = block.label.clone();

    // Validate value size if provided
    if let Some(ref value) = request.value {
        if let Some(limit) = request.limit.or(block.limit) {
            if value.len() > limit {
                return Err(ApiError::bad_request(format!(
                    "value exceeds limit ({} > {})",
                    value.len(),
                    limit
                )));
            }
        }
    }

    // Single source of truth: Require AgentService
    let service = state
        .agent_service()
        .ok_or_else(|| ApiError::internal("AgentService not configured"))?;

    // Use value from request, or keep current value
    let new_value = request.value.unwrap_or_else(|| block.value.clone());

    service
        .update_block_by_label(&agent_id, &label, new_value)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to update block: {}", e)))?;

    // Get updated agent to return the updated block
    let updated_agent = state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::internal("Agent not found after update"))?;

    let updated_block = updated_agent
        .blocks
        .iter()
        .find(|b| b.id == block_id)
        .cloned()
        .ok_or_else(|| ApiError::internal("Block not found after update"))?;

    tracing::info!(agent_id = %agent_id, block_id = %block_id, "updated block");
    Ok(Json(updated_block))
}

// =============================================================================
// Core Memory routes (letta-code compatibility)
// These use block labels instead of block IDs
// =============================================================================

/// Get a block by label
///
/// GET /v1/agents/{agent_id}/core-memory/blocks/{label}
#[instrument(skip(state), fields(agent_id = %agent_id, label = %label), level = "info")]
pub async fn get_block_by_label<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path((agent_id, label)): Path<(String, String)>,
) -> Result<Json<Block>, ApiError> {
    // Get agent from actor system (AgentService required)
    let agent = state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Agent", &agent_id))?;

    // Find block by label
    let block = agent
        .blocks
        .iter()
        .find(|b| b.label == label)
        .cloned()
        .ok_or_else(|| ApiError::not_found("Block", &label))?;

    Ok(Json(block))
}

/// Update a block by label
///
/// PATCH /v1/agents/{agent_id}/core-memory/blocks/{label}
#[instrument(skip(state, request), fields(agent_id = %agent_id, label = %label), level = "info")]
pub async fn update_block_by_label<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path((agent_id, label)): Path<(String, String)>,
    Json(request): Json<UpdateBlockRequest>,
) -> Result<Json<Block>, ApiError> {
    // Get agent first to check it exists and get block info
    let agent = state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Agent", &agent_id))?;

    // Find the block to validate
    let block = agent
        .blocks
        .iter()
        .find(|b| b.label == label)
        .ok_or_else(|| ApiError::not_found("Block", &label))?;

    // Validate value size if provided
    if let Some(ref value) = request.value {
        if let Some(limit) = request.limit.or(block.limit) {
            if value.len() > limit {
                return Err(ApiError::bad_request(format!(
                    "value exceeds limit ({} > {})",
                    value.len(),
                    limit
                )));
            }
        }
    }

    // Single source of truth: Require AgentService
    let service = state
        .agent_service()
        .ok_or_else(|| ApiError::internal("AgentService not configured"))?;

    // Use value from request, or keep current value
    let new_value = request.value.unwrap_or_else(|| block.value.clone());

    service
        .update_block_by_label(&agent_id, &label, new_value)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to update block: {}", e)))?;

    // Get updated agent to return the updated block
    let updated_agent = state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::internal("Agent not found after update"))?;

    let updated_block = updated_agent
        .blocks
        .iter()
        .find(|b| b.label == label)
        .cloned()
        .ok_or_else(|| ApiError::internal("Block not found after update"))?;

    tracing::info!(agent_id = %agent_id, label = %label, "updated block by label");
    Ok(Json(updated_block))
}

// =============================================================================
// Smart handlers for Letta compatibility
// These detect whether the parameter is a UUID (block_id) or label (string)
// =============================================================================

/// Get a block by ID or label (smart detection)
///
/// GET /v1/agents/{agent_id}/blocks/{id_or_label}
///
/// This handler supports both UUID-based and label-based access:
/// - If the parameter looks like a UUID, use block ID lookup
/// - Otherwise, treat it as a label
#[instrument(skip(state), fields(agent_id = %agent_id, param = %id_or_label), level = "info")]
pub async fn get_block_or_label<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path((agent_id, id_or_label)): Path<(String, String)>,
) -> Result<Json<Block>, ApiError> {
    // Try to parse as UUID - if successful, it's a block ID
    if uuid::Uuid::parse_str(&id_or_label).is_ok() {
        tracing::debug!("parameter is UUID, using ID lookup");
        get_block(State(state), Path((agent_id, id_or_label))).await
    } else {
        tracing::debug!("parameter is not UUID, using label lookup");
        get_block_by_label(State(state), Path((agent_id, id_or_label))).await
    }
}

/// Update a block by ID or label (smart detection)
///
/// PATCH /v1/agents/{agent_id}/blocks/{id_or_label}
///
/// This handler supports both UUID-based and label-based access:
/// - If the parameter looks like a UUID, use block ID update
/// - Otherwise, treat it as a label
#[instrument(skip(state, request), fields(agent_id = %agent_id, param = %id_or_label), level = "info")]
pub async fn update_block_or_label<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path((agent_id, id_or_label)): Path<(String, String)>,
    Json(request): Json<UpdateBlockRequest>,
) -> Result<Json<Block>, ApiError> {
    // Try to parse as UUID - if successful, it's a block ID
    if uuid::Uuid::parse_str(&id_or_label).is_ok() {
        tracing::debug!("parameter is UUID, using ID update");
        update_block(State(state), Path((agent_id, id_or_label)), Json(request)).await
    } else {
        tracing::debug!("parameter is not UUID, using label update");
        update_block_by_label(State(state), Path((agent_id, id_or_label)), Json(request)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use kelpie_dst::{DeterministicRng, FaultInjector, SimStorage};
    use kelpie_runtime::{CloneFactory, Dispatcher, DispatcherConfig};
    use kelpie_server::actor::{AgentActor, AgentActorState, LlmClient, LlmMessage, LlmResponse};
    use kelpie_server::models::AgentState;
    use kelpie_server::service;
    use kelpie_server::state::AppState;
    use kelpie_server::tools::UnifiedToolRegistry;
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Mock LLM client for testing
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

    async fn test_app_with_agent() -> (Router, String, String) {
        // Create app with AgentService
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient);
        let actor = AgentActor::new(llm, Arc::new(UnifiedToolRegistry::new()));
        let factory = Arc::new(CloneFactory::new(actor));

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

        // Create agent with a block
        let body = serde_json::json!({
            "name": "block-test-agent",
            "memory_blocks": [{
                "label": "persona",
                "value": "I am a test agent",
                "limit": 1000
            }]
        });

        let app = api::router(state.clone());

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

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let agent: AgentState = serde_json::from_slice(&body).unwrap();
        let block_id = agent.blocks[0].id.clone();

        // Return new router with same state
        (api::router(state), agent.id, block_id)
    }

    #[tokio::test]
    async fn test_list_blocks() {
        let (app, agent_id, _) = test_app_with_agent().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/agents/{}/blocks", agent_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let blocks: Vec<kelpie_server::models::Block> = serde_json::from_slice(&body).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].label, "persona");
    }

    #[tokio::test]
    async fn test_update_block() {
        let (app, agent_id, block_id) = test_app_with_agent().await;

        let update = serde_json::json!({
            "value": "Updated persona value"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/v1/agents/{}/blocks/{}", agent_id, block_id))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&update).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let block: kelpie_server::models::Block = serde_json::from_slice(&body).unwrap();
        assert_eq!(block.value, "Updated persona value");
    }

    #[tokio::test]
    async fn test_update_block_by_label_letta_compat() {
        // Test Letta compatibility: /v1/agents/{id}/blocks/{label} path
        let (app, agent_id, _block_id) = test_app_with_agent().await;

        let update = serde_json::json!({
            "value": "Updated via label path"
        });

        // Use label "persona" instead of UUID - this is the Letta-compatible path
        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/v1/agents/{}/blocks/persona", agent_id))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&update).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let block: kelpie_server::models::Block = serde_json::from_slice(&body).unwrap();
        assert_eq!(block.value, "Updated via label path");
        assert_eq!(block.label, "persona");
    }

    #[tokio::test]
    async fn test_get_block_by_label_letta_compat() {
        // Test Letta compatibility: GET /v1/agents/{id}/blocks/{label}
        let (app, agent_id, _block_id) = test_app_with_agent().await;

        // Use label "persona" instead of UUID
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/agents/{}/blocks/persona", agent_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let block: kelpie_server::models::Block = serde_json::from_slice(&body).unwrap();
        assert_eq!(block.label, "persona");
        assert_eq!(block.value, "I am a test agent");
    }
}
