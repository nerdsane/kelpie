//! Summarization API endpoints
//!
//! TigerStyle: Letta-compatible conversation and memory summarization.
//!
//! Phase 4: Provides LLM-powered summarization of agent conversations
//! and memory blocks for memory management and context compression.

use crate::api::ApiError;
use axum::{extract::Path, routing::post, Router};
use axum::{extract::State, Json};
use kelpie_core::Runtime;
use kelpie_server::llm::ChatMessage;
use kelpie_server::models::MessageRole;
use kelpie_server::state::AppState;
use serde::{Deserialize, Serialize};
use tracing::instrument;

/// TigerStyle: Explicit constants with units
const MESSAGES_TO_SUMMARIZE_MAX: usize = 1000;
const SUMMARY_LENGTH_CHARS_TARGET: usize = 500;
const SUMMARY_LENGTH_CHARS_MAX: usize = 2000;

/// Request to summarize conversation messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizeMessagesRequest {
    /// Number of recent messages to summarize
    #[serde(default = "default_message_count")]
    pub message_count: usize,
    /// Optional custom instruction for summarization
    pub instruction: Option<String>,
}

fn default_message_count() -> usize {
    50
}

/// Request to summarize memory blocks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizeMemoryRequest {
    /// Block labels to include in summary (empty = all blocks)
    #[serde(default)]
    pub block_labels: Vec<String>,
    /// Optional custom instruction for summarization
    pub instruction: Option<String>,
}

/// Summarization response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizationResponse {
    /// Generated summary
    pub summary: String,
    /// Number of messages or blocks summarized
    pub items_summarized: usize,
    /// Original content length in characters
    pub original_length: usize,
    /// Summary length in characters
    pub summary_length: usize,
}

/// Create summarization routes
pub fn router<R: Runtime + 'static>() -> Router<AppState<R>> {
    Router::new()
        .route("/:agent_id/messages/summarize", post(summarize_messages))
        .route("/:agent_id/memory/summarize", post(summarize_memory))
}

/// Summarize recent conversation messages
///
/// POST /v1/agents/{agent_id}/messages/summarize
#[instrument(skip(state, request), fields(agent_id = %agent_id), level = "info")]
async fn summarize_messages<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(agent_id): Path<String>,
    Json(request): Json<SummarizeMessagesRequest>,
) -> Result<Json<SummarizationResponse>, ApiError> {
    // Validate message count
    let message_count = request.message_count.min(MESSAGES_TO_SUMMARIZE_MAX);
    assert!(message_count > 0, "message_count must be positive");

    // Get agent to verify it exists
    let _agent = state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Agent", &agent_id))?;

    // Fetch recent messages
    let messages = state.list_messages(&agent_id, message_count, None)?;

    if messages.is_empty() {
        return Err(ApiError::bad_request(
            "No messages to summarize - agent has no conversation history",
        ));
    }

    // Build conversation text
    let mut conversation_text = String::new();
    for msg in &messages {
        conversation_text.push_str(&format!(
            "{}: {}\n\n",
            role_to_display(&msg.role),
            msg.content
        ));
    }

    let original_length = conversation_text.len();

    // Build summarization prompt
    let instruction = request.instruction.unwrap_or_else(|| {
        format!(
            "Summarize the following conversation in approximately {} characters. \
             Focus on key topics, decisions, and action items. Be concise but preserve important details.",
            SUMMARY_LENGTH_CHARS_TARGET
        )
    });

    let prompt = format!("{}\n\nConversation:\n{}", instruction, conversation_text);

    // Generate summary using LLM
    let llm = state.llm().ok_or_else(|| {
        ApiError::internal("LLM not configured - set ANTHROPIC_API_KEY or OPENAI_API_KEY")
    })?;

    let llm_messages = vec![ChatMessage {
        role: "user".to_string(),
        content: prompt,
    }];

    let response = llm
        .complete(llm_messages)
        .await
        .map_err(|e| ApiError::internal(format!("LLM summarization failed: {}", e)))?;

    let summary = response.content;
    let summary_length = summary.len();

    // Validate summary length
    if summary_length > SUMMARY_LENGTH_CHARS_MAX {
        tracing::warn!(
            agent_id = %agent_id,
            summary_length = summary_length,
            max_length = SUMMARY_LENGTH_CHARS_MAX,
            "summary exceeded maximum length"
        );
    }

    tracing::info!(
        agent_id = %agent_id,
        messages_count = messages.len(),
        original_length = original_length,
        summary_length = summary_length,
        compression_ratio = format!("{:.2}", original_length as f64 / summary_length as f64),
        "summarized messages"
    );

    Ok(Json(SummarizationResponse {
        summary,
        items_summarized: messages.len(),
        original_length,
        summary_length,
    }))
}

/// Summarize agent's memory blocks
///
/// POST /v1/agents/{agent_id}/memory/summarize
#[instrument(skip(state, request), fields(agent_id = %agent_id), level = "info")]
async fn summarize_memory<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(agent_id): Path<String>,
    Json(request): Json<SummarizeMemoryRequest>,
) -> Result<Json<SummarizationResponse>, ApiError> {
    // Get agent with memory blocks
    let agent = state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Agent", &agent_id))?;

    if agent.blocks.is_empty() {
        return Err(ApiError::bad_request(
            "No memory blocks to summarize - agent has no memory blocks",
        ));
    }

    // Filter blocks if specific labels requested
    let blocks_to_summarize = if request.block_labels.is_empty() {
        agent.blocks.clone()
    } else {
        agent
            .blocks
            .into_iter()
            .filter(|b| request.block_labels.contains(&b.label))
            .collect()
    };

    if blocks_to_summarize.is_empty() {
        return Err(ApiError::bad_request(
            "No matching memory blocks found for the specified labels",
        ));
    }

    // Build memory content text
    let mut memory_text = String::new();
    for block in &blocks_to_summarize {
        memory_text.push_str(&format!(
            "### {}\n{}\n\n",
            block.label.to_uppercase(),
            block.value
        ));
    }

    let original_length = memory_text.len();

    // Build summarization prompt
    let instruction = request.instruction.unwrap_or_else(|| {
        format!(
            "Summarize the following memory blocks in approximately {} characters. \
             Preserve key facts, preferences, and context. Be concise but don't lose critical information.",
            SUMMARY_LENGTH_CHARS_TARGET
        )
    });

    let prompt = format!("{}\n\nMemory Blocks:\n{}", instruction, memory_text);

    // Generate summary using LLM
    let llm = state.llm().ok_or_else(|| {
        ApiError::internal("LLM not configured - set ANTHROPIC_API_KEY or OPENAI_API_KEY")
    })?;

    let llm_messages = vec![ChatMessage {
        role: "user".to_string(),
        content: prompt,
    }];

    let response = llm
        .complete(llm_messages)
        .await
        .map_err(|e| ApiError::internal(format!("LLM summarization failed: {}", e)))?;

    let summary = response.content;
    let summary_length = summary.len();

    // Validate summary length
    if summary_length > SUMMARY_LENGTH_CHARS_MAX {
        tracing::warn!(
            agent_id = %agent_id,
            summary_length = summary_length,
            max_length = SUMMARY_LENGTH_CHARS_MAX,
            "summary exceeded maximum length"
        );
    }

    tracing::info!(
        agent_id = %agent_id,
        blocks_count = blocks_to_summarize.len(),
        original_length = original_length,
        summary_length = summary_length,
        compression_ratio = format!("{:.2}", original_length as f64 / summary_length as f64),
        "summarized memory blocks"
    );

    Ok(Json(SummarizationResponse {
        summary,
        items_summarized: blocks_to_summarize.len(),
        original_length,
        summary_length,
    }))
}

/// Helper: Convert MessageRole to display string
fn role_to_display(role: &MessageRole) -> &str {
    match role {
        MessageRole::User => "User",
        MessageRole::Assistant => "Assistant",
        MessageRole::System => "System",
        MessageRole::Tool => "Tool",
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
    use kelpie_core::Runtime;
    use kelpie_dst::{DeterministicRng, FaultInjector, SimStorage};
    use kelpie_runtime::{CloneFactory, Dispatcher, DispatcherConfig};
    use kelpie_server::actor::{AgentActor, AgentActorState, LlmClient, LlmMessage, LlmResponse};
    use kelpie_server::models::AgentState;
    use kelpie_server::service::AgentService;
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

    /// Create test app with AgentService (single source of truth)
    ///
    /// Note: These tests focus on validation and error handling.
    /// LLM integration is tested separately with real LLM clients in integration tests.
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

        let service = AgentService::new(handle.clone());
        let state = AppState::with_agent_service(runtime, service, handle);
        api::router(state)
    }

    /// Helper: Create agent with memory blocks (using legacy API)
    async fn create_agent_with_blocks(app: &Router) -> String {
        // Create agent
        let create_body = serde_json::json!({
            "name": "test-agent",
            "memory_blocks": [{
                "label": "persona",
                "value": "I am a helpful test agent"
            }]
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

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "Agent creation should succeed"
        );

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let agent: AgentState = serde_json::from_slice(&body).unwrap();

        agent.id
    }

    #[tokio::test]
    async fn test_summarize_messages_no_messages() {
        let app = test_app().await;
        let agent_id = create_agent_with_blocks(&app).await;

        let request_body = serde_json::json!({
            "message_count": 10
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/agents/{}/messages/summarize", agent_id))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return 400 because agent has no messages yet
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_summarize_memory_blocks_no_llm() {
        let app = test_app().await;
        let agent_id = create_agent_with_blocks(&app).await;

        let request_body = serde_json::json!({
            "block_labels": []
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/agents/{}/memory/summarize", agent_id))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return 500 because LLM is not configured
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_summarize_memory_empty_blocks() {
        let app = test_app().await;

        // Create agent without memory blocks
        let create_body = serde_json::json!({
            "name": "empty-agent",
            "memory_blocks": []
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

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let agent: AgentState = serde_json::from_slice(&body).unwrap();

        // Try to summarize - should fail because no blocks
        let request_body = serde_json::json!({
            "block_labels": []
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/agents/{}/memory/summarize", agent.id))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return 400 because agent has no memory blocks
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_summarize_memory_nonexistent_blocks() {
        let app = test_app().await;
        let agent_id = create_agent_with_blocks(&app).await;

        let request_body = serde_json::json!({
            "block_labels": ["nonexistent"]
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/agents/{}/memory/summarize", agent_id))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return 400 because no matching blocks found
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_summarize_nonexistent_agent() {
        let app = test_app().await;

        let request_body = serde_json::json!({
            "message_count": 10
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents/nonexistent/messages/summarize")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
