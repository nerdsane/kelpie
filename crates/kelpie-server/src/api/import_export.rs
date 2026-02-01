//! Import/Export API endpoints
//!
//! TigerStyle: Letta-compatible agent import/export for portability.
//!
//! Phase 3: Supports exporting agents with their configuration and conversation history,
//! and importing agents from external sources.

use crate::api::ApiError;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::Utc;
use kelpie_core::Runtime;
use kelpie_server::models::{
    AgentState, CreateAgentRequest, CreateBlockRequest, ExportAgentResponse, ImportAgentRequest,
    Message,
};
use kelpie_server::state::AppState;
use serde::Deserialize;
use tracing::instrument;
use uuid::Uuid;

/// Query parameters for export
#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    /// Include conversation messages in export
    #[serde(default)]
    pub include_messages: bool,
}

/// Maximum messages to export
const EXPORT_MESSAGES_MAX: usize = 10000;

/// Export an agent's configuration and optionally its conversation history
///
/// GET /v1/agents/{agent_id}/export
#[instrument(skip(state), fields(agent_id = %agent_id, include_messages = query.include_messages), level = "info")]
pub async fn export_agent<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(agent_id): Path<String>,
    Query(query): Query<ExportQuery>,
) -> Result<Json<ExportAgentResponse>, ApiError> {
    // Get agent state
    let agent = state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Agent", &agent_id))?;

    // Get messages if requested
    let messages = if query.include_messages {
        // Fetch messages from agent state
        match state.list_messages(&agent_id, EXPORT_MESSAGES_MAX, None) {
            Ok(msgs) => msgs,
            Err(e) => {
                tracing::warn!(agent_id = %agent_id, error = ?e, "failed to fetch messages for export, continuing without them");
                vec![]
            }
        }
    } else {
        vec![]
    };

    tracing::info!(
        agent_id = %agent_id,
        block_count = agent.blocks.len(),
        message_count = messages.len(),
        "exported agent"
    );

    Ok(Json(ExportAgentResponse { agent, messages }))
}

/// Import an agent from external source
///
/// POST /v1/agents/import
#[instrument(skip(state, request), fields(agent_name = %request.agent.name, message_count = request.messages.len()), level = "info")]
pub async fn import_agent<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Json(request): Json<ImportAgentRequest>,
) -> Result<Json<AgentState>, ApiError> {
    let agent_data = request.agent;

    // Validate agent name
    if agent_data.name.is_empty() {
        return Err(ApiError::bad_request("agent name cannot be empty"));
    }

    if agent_data.name.len() > 256 {
        return Err(ApiError::bad_request(
            "agent name too long (max 256 characters)",
        ));
    }

    // Convert imported blocks to create requests
    let memory_blocks: Vec<CreateBlockRequest> = agent_data
        .blocks
        .into_iter()
        .map(|block| CreateBlockRequest {
            label: block.label,
            value: block.value,
            description: block.description,
            limit: block.limit,
        })
        .collect();

    // Create agent via standard creation flow
    let create_request = CreateAgentRequest {
        name: agent_data.name.clone(),
        agent_type: agent_data.agent_type,
        model: agent_data.model,
        embedding: agent_data.embedding,
        system: agent_data.system,
        description: agent_data.description,
        memory_blocks,
        block_ids: vec![], // Don't import block_ids - create fresh blocks
        tool_ids: agent_data.tool_ids,
        tags: agent_data.tags,
        metadata: agent_data.metadata,
        project_id: agent_data.project_id,
        user_id: None,
        org_id: None,
    };

    // Create agent
    let created = state.create_agent_async(create_request).await?;

    // Import messages if provided
    if !request.messages.is_empty() {
        match import_messages(&state, &created.id, request.messages).await {
            Ok(imported_count) => {
                tracing::info!(
                    agent_id = %created.id,
                    message_count = imported_count,
                    "imported messages"
                );
            }
            Err(e) => {
                tracing::warn!(
                    agent_id = %created.id,
                    error = ?e,
                    "failed to import messages, continuing with agent creation"
                );
            }
        }
    }

    tracing::info!(
        agent_id = %created.id,
        name = %created.name,
        block_count = created.blocks.len(),
        "imported agent"
    );

    Ok(Json(created))
}

/// Helper function to import messages into an agent
///
/// TigerStyle: Separate function for clarity and error isolation.
async fn import_messages<R: Runtime + 'static>(
    state: &AppState<R>,
    agent_id: &str,
    messages: Vec<kelpie_server::models::MessageImportData>,
) -> Result<usize, String> {
    let mut imported_count = 0;

    for msg_data in messages {
        // Convert imported message to Message
        let message = Message {
            id: Uuid::new_v4().to_string(),
            agent_id: agent_id.to_string(),
            message_type: Message::message_type_from_role(&msg_data.role),
            role: msg_data.role,
            content: msg_data.content,
            tool_call_id: msg_data.tool_call_id,
            tool_calls: msg_data.tool_calls,
            tool_call: None,
            tool_return: None,
            status: None,
            created_at: Utc::now(),
        };

        // Store message in agent state (with storage persistence)
        if let Err(e) = state.add_message_async(agent_id, message).await {
            tracing::warn!(
                agent_id = %agent_id,
                error = ?e,
                "failed to import message, skipping"
            );
            continue;
        }

        imported_count += 1;
    }

    Ok(imported_count)
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
    use kelpie_server::models::{AgentImportData, BlockImportData};
    use kelpie_server::service;
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

    /// Create test app
    async fn test_app() -> Router {
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

        api::router(state)
    }

    #[tokio::test]
    async fn test_export_agent() {
        let app = test_app().await;

        // Create an agent
        let create_body = serde_json::json!({
            "name": "export-test-agent",
            "memory_blocks": [{
                "label": "persona",
                "value": "I am a test agent"
            }]
        });

        let create_response = app
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

        assert_eq!(create_response.status(), StatusCode::OK);

        let create_body = axum::body::to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let agent: AgentState = serde_json::from_slice(&create_body).unwrap();

        // Export the agent
        let export_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/agents/{}/export", agent.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(export_response.status(), StatusCode::OK);

        let export_body = axum::body::to_bytes(export_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let exported: ExportAgentResponse = serde_json::from_slice(&export_body).unwrap();

        assert_eq!(exported.agent.id, agent.id);
        assert_eq!(exported.agent.name, "export-test-agent");
        assert_eq!(exported.agent.blocks.len(), 1);
        assert_eq!(exported.messages.len(), 0); // No messages sent yet
    }

    #[tokio::test]
    async fn test_import_agent() {
        let app = test_app().await;

        // Prepare import data
        let import_body = serde_json::json!({
            "agent": {
                "name": "imported-agent",
                "agent_type": "memgpt_agent",
                "model": "openai/gpt-4o",
                "system": "You are helpful",
                "description": "An imported agent",
                "blocks": [{
                    "label": "persona",
                    "value": "I am imported",
                    "description": "Agent persona",
                    "limit": 1000
                }],
                "tool_ids": [],
                "tags": ["imported"],
                "metadata": {}
            },
            "messages": []
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents/import")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&import_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let imported: AgentState = serde_json::from_slice(&body).unwrap();

        assert_eq!(imported.name, "imported-agent");
        assert_eq!(imported.blocks.len(), 1);
        assert_eq!(imported.blocks[0].label, "persona");
        assert_eq!(imported.blocks[0].value, "I am imported");
        assert!(imported.tags.contains(&"imported".to_string()));
    }

    #[tokio::test]
    async fn test_import_agent_empty_name() {
        let app = test_app().await;

        let import_body = serde_json::json!({
            "agent": {
                "name": "",
                "blocks": []
            },
            "messages": []
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents/import")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&import_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_export_nonexistent_agent() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/agents/nonexistent/export")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_roundtrip_export_import() {
        let app = test_app().await;

        // Create original agent
        let create_body = serde_json::json!({
            "name": "roundtrip-agent",
            "description": "Testing roundtrip",
            "memory_blocks": [{
                "label": "persona",
                "value": "Original persona"
            }],
            "tags": ["test", "roundtrip"]
        });

        let create_response = app
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

        let create_body = axum::body::to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let original: AgentState = serde_json::from_slice(&create_body).unwrap();

        // Export
        let export_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/agents/{}/export", original.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let export_body = axum::body::to_bytes(export_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let exported: ExportAgentResponse = serde_json::from_slice(&export_body).unwrap();

        // Import as new agent
        let import_body = ImportAgentRequest {
            agent: AgentImportData {
                name: format!("{}-imported", exported.agent.name),
                agent_type: exported.agent.agent_type,
                model: exported.agent.model,
                embedding: exported.agent.embedding,
                system: exported.agent.system,
                description: exported.agent.description,
                blocks: exported
                    .agent
                    .blocks
                    .iter()
                    .map(|b| BlockImportData {
                        label: b.label.clone(),
                        value: b.value.clone(),
                        description: b.description.clone(),
                        limit: b.limit,
                    })
                    .collect(),
                tool_ids: exported.agent.tool_ids,
                tags: exported.agent.tags,
                metadata: exported.agent.metadata,
                project_id: exported.agent.project_id,
            },
            messages: vec![],
        };

        let import_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents/import")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&import_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(import_response.status(), StatusCode::OK);

        let import_body = axum::body::to_bytes(import_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let imported: AgentState = serde_json::from_slice(&import_body).unwrap();

        // Verify imported agent matches original (except ID and name)
        assert_ne!(imported.id, original.id); // New ID generated
        assert_eq!(imported.name, "roundtrip-agent-imported");
        assert_eq!(imported.description, original.description);
        assert_eq!(imported.blocks.len(), original.blocks.len());
        assert_eq!(imported.blocks[0].value, "Original persona");
        assert_eq!(imported.tags, original.tags);
    }
}
