//! Archival memory API endpoints
//!
//! TigerStyle: RESTful archival memory management with semantic search.

use crate::api::ApiError;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use kelpie_core::Runtime;
use kelpie_server::models::ArchivalEntry;
use kelpie_server::state::AppState;
use serde::{Deserialize, Serialize};
use tracing::instrument;

/// List archival entries response
#[derive(Debug, Serialize)]
pub struct ArchivalListResponse {
    pub items: Vec<ArchivalEntry>,
    pub count: usize,
    pub total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Query parameters for archival search
#[derive(Debug, Deserialize)]
pub struct ArchivalSearchQuery {
    /// Search query text (for semantic search)
    #[serde(default)]
    pub q: Option<String>,
    /// Maximum number of results to return
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Cursor for pagination (reserved for future use)
    #[serde(default)]
    #[allow(dead_code)]
    pub cursor: Option<String>,
    /// Cursor for pagination (Letta SDK compatibility - alias for cursor)
    #[serde(default)]
    #[allow(dead_code)]
    pub after: Option<String>,
}

fn default_limit() -> usize {
    50
}

/// Request to add to archival memory
#[derive(Debug, Deserialize)]
pub struct AddArchivalRequest {
    #[serde(alias = "text")]
    pub content: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Search archival memory
#[instrument(skip(state, query), fields(agent_id = %agent_id, query = ?query.q, limit = query.limit), level = "info")]
pub async fn search_archival<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(agent_id): Path<String>,
    Query(query): Query<ArchivalSearchQuery>,
) -> Result<Json<ArchivalListResponse>, ApiError> {
    // Verify agent exists (using async method)
    state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("agent", &agent_id))?;

    // Single source of truth: Use AgentService
    let service = state
        .agent_service()
        .ok_or_else(|| ApiError::internal("AgentService not configured"))?;

    // Search archival memory via AgentService
    let entries = service
        .archival_search(&agent_id, query.q.as_deref().unwrap_or(""), query.limit)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to search archival: {}", e)))?;

    let count = entries.len();
    Ok(Json(ArchivalListResponse {
        items: entries,
        count,
        total: count, // For now, total equals count since we don't have full pagination
        next_cursor: None,
    }))
}

/// Add entry to archival memory
#[instrument(skip(state, request), fields(agent_id = %agent_id), level = "info")]
pub async fn add_archival<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(agent_id): Path<String>,
    Json(request): Json<AddArchivalRequest>,
) -> Result<Json<ArchivalEntry>, ApiError> {
    // Verify agent exists (using async method)
    state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("agent", &agent_id))?;

    // Validate content
    if request.content.is_empty() {
        return Err(ApiError::bad_request("Content cannot be empty"));
    }

    if request.content.len() > 100_000 {
        return Err(ApiError::bad_request("Content too long (max 100KB)"));
    }

    // Single source of truth: Use AgentService
    let service = state
        .agent_service()
        .ok_or_else(|| ApiError::internal("AgentService not configured"))?;

    // Add to archival memory via AgentService
    let entry_id = service
        .archival_insert(&agent_id, &request.content, request.metadata.clone())
        .await
        .map_err(|e| ApiError::internal(format!("Failed to add archival entry: {}", e)))?;

    // Create entry object for response
    let entry = ArchivalEntry {
        id: entry_id.clone(),
        content: request.content,
        metadata: request.metadata.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    tracing::info!(agent_id = %agent_id, entry_id = %entry_id, "Added archival entry");

    Ok(Json(entry))
}

/// Get a specific archival entry
#[instrument(skip(state), fields(agent_id = %agent_id, entry_id = %entry_id), level = "info")]
pub async fn get_archival_entry<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path((agent_id, entry_id)): Path<(String, String)>,
) -> Result<Json<ArchivalEntry>, ApiError> {
    // Verify agent exists (using async method)
    state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("agent", &agent_id))?;

    // Single source of truth: Use AgentService
    let service = state
        .agent_service()
        .ok_or_else(|| ApiError::internal("AgentService not configured"))?;

    // Search for the specific entry
    let entries = service
        .archival_search(&agent_id, "", 1000)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to search archival: {}", e)))?;

    let entry = entries
        .into_iter()
        .find(|e| e.id == entry_id)
        .ok_or_else(|| ApiError::not_found("archival_entry", &entry_id))?;

    Ok(Json(entry))
}

/// Delete an archival entry
#[instrument(skip(state), fields(agent_id = %agent_id, entry_id = %entry_id), level = "info")]
pub async fn delete_archival_entry<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path((agent_id, entry_id)): Path<(String, String)>,
) -> Result<(), ApiError> {
    // Verify agent exists (using async method)
    state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("agent", &agent_id))?;

    // Single source of truth: Use AgentService
    let service = state
        .agent_service()
        .ok_or_else(|| ApiError::internal("AgentService not configured"))?;

    service
        .archival_delete(&agent_id, &entry_id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to delete archival entry: {}", e)))?;

    tracing::info!(agent_id = %agent_id, entry_id = %entry_id, "Deleted archival entry");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use kelpie_core::Runtime;
    use kelpie_dst::{DeterministicRng, FaultInjector, SimStorage};
    use kelpie_runtime::{CloneFactory, Dispatcher, DispatcherConfig};
    use kelpie_server::actor::{AgentActor, AgentActorState, LlmClient, LlmMessage, LlmResponse};
    use kelpie_server::service::AgentService;
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

    async fn test_app_with_agent() -> (Router, String, AppState<kelpie_core::TokioRuntime>) {
        // Create AppState with AgentService (single source of truth)
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

        let service = AgentService::new(handle.clone());
        let state = AppState::with_agent_service(runtime, service, handle);

        // Create agent
        let body = serde_json::json!({
            "name": "archival-test-agent",
        });

        let app = api::router(state.clone());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), 10000)
            .await
            .unwrap();
        let agent: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let agent_id = agent["id"].as_str().unwrap().to_string();

        // Return router, agent_id, AND state for verification
        (app, agent_id, state)
    }

    #[tokio::test]
    async fn test_search_archival_empty() {
        let (app, agent_id, _state) = test_app_with_agent().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/agents/{}/archival", agent_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_add_archival() {
        let (app, agent_id, _state) = test_app_with_agent().await;

        let body = serde_json::json!({
            "content": "This is a test archival entry",
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/agents/{}/archival", agent_id))
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn test_archival_text_alias() {
        // Test "text" field is accepted (Letta compatibility)
        let json = r#"{"text": "test entry"}"#;
        let parsed: AddArchivalRequest = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.content, "test entry");

        // Test "content" field still works
        let json = r#"{"content": "test entry"}"#;
        let parsed: AddArchivalRequest = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.content, "test entry");
    }
}
