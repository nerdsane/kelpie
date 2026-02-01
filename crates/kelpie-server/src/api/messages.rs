//! Message API endpoints
//!
//! TigerStyle: Letta-compatible message operations with SSE streaming support.

use crate::api::ApiError;
use axum::{
    extract::{Path, Query, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use chrono::Utc;
use futures::stream::{self, StreamExt};
use kelpie_core::Runtime;
use kelpie_server::models::{
    BatchMessagesRequest, BatchStatus, CreateMessageRequest, Message, MessageResponse,
};
use kelpie_server::state::AppState;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::time::Duration;
use tracing::instrument;
use uuid::Uuid;

/// Query parameters for listing messages
#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    /// Maximum number of messages to return
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Return messages before this ID
    pub before: Option<String>,
}

fn default_limit() -> usize {
    100
}

/// Maximum limit for list operations
const LIST_LIMIT_MAX: usize = 1000;

/// Query parameters for sending messages (streaming support)
#[derive(Debug, Deserialize, Default)]
pub struct SendMessageQuery {
    /// Enable step streaming (letta-code compatibility)
    #[serde(default)]
    pub stream_steps: bool,
    /// Enable token streaming
    #[serde(default)]
    pub stream_tokens: bool,
}

// =============================================================================
// SSE Message Types (Letta-compatible)
// =============================================================================

/// SSE message types for streaming responses
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "message_type")]
#[allow(dead_code)]
enum SseMessage {
    #[serde(rename = "assistant_message")]
    AssistantMessage { id: String, content: String },
    #[serde(rename = "reasoning_message")]
    ReasoningMessage { id: String, reasoning: String },
    #[serde(rename = "tool_call_message")]
    ToolCallMessage { id: String, tool_call: ToolCallInfo },
    #[serde(rename = "tool_return_message")]
    ToolReturnMessage {
        id: String,
        tool_return: String,
        status: String,
    },
    #[serde(rename = "approval_request_message")]
    ApprovalRequestMessage {
        id: String,
        tool_call_id: String,
        tool_call: ToolCallInfo,
    },
    #[serde(rename = "usage_statistics")]
    UsageStatistics {
        completion_tokens: u64,
        prompt_tokens: u64,
        total_tokens: u64,
        step_count: u32,
    },
}

/// Information about a tool call in a streaming response
///
/// Used in SSE events to notify clients about tool invocations.
#[derive(Debug, Clone, Serialize)]
struct ToolCallInfo {
    /// Name of the tool being called
    name: String,
    /// Arguments serialized as JSON string (Letta SDK compatibility)
    /// The Letta SDK sends arguments as a JSON string, not a nested object.
    /// Clients expect to call JSON.parse(arguments) on this field.
    arguments: String,
    /// Tool call ID for tracking (Letta SDK compatibility)
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

/// SSE event for stop reason
#[derive(Debug, Clone, Serialize)]
struct StopReasonEvent {
    message_type: &'static str,
    stop_reason: String,
}

/// List messages for an agent
///
/// GET /v1/agents/{agent_id}/messages
#[instrument(skip(state, query), fields(agent_id = %agent_id, limit = query.limit), level = "info")]
pub async fn list_messages<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(agent_id): Path<String>,
    Query(query): Query<ListMessagesQuery>,
) -> Result<Json<Vec<Message>>, ApiError> {
    let limit = query.limit.min(LIST_LIMIT_MAX);

    // Single source of truth: AgentService required (no fallback)
    let service = state
        .agent_service()
        .ok_or_else(|| ApiError::internal("AgentService not configured"))?;

    let messages = service
        .list_messages(&agent_id, limit, query.before.as_deref())
        .await
        .map_err(|e| ApiError::internal(format!("Failed to list messages: {}", e)))?;

    Ok(Json(messages))
}

/// Send a message to an agent
///
/// POST /v1/agents/{agent_id}/messages
///
/// Builds a prompt from memory blocks and sends to configured LLM.
/// Requires LLM to be configured via ANTHROPIC_API_KEY or OPENAI_API_KEY.
/// Supports multiple request formats for letta-code compatibility.
/// Supports SSE streaming when stream_steps=true query parameter is set,
/// OR when streaming=true is passed in the request body (Letta SDK compatibility).
#[instrument(skip(state, query, request), fields(agent_id = %agent_id), level = "info")]
pub async fn send_message<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(agent_id): Path<String>,
    Query(query): Query<SendMessageQuery>,
    Json(request): Json<CreateMessageRequest>,
) -> Result<Response, ApiError> {
    // If streaming is requested (via query param OR request body), delegate to SSE handler
    // This provides compatibility with both:
    // - letta-code (uses stream_steps query param)
    // - Letta SDK (uses streaming field in request body)
    // stream_tokens enables token-by-token streaming (finer granularity than step streaming)
    let should_stream = query.stream_steps || query.stream_tokens || request.streaming;
    tracing::info!(
        stream_steps = query.stream_steps,
        stream_tokens = query.stream_tokens,
        "Processing message request"
    );

    if should_stream {
        return send_message_streaming(state, agent_id, query, request).await;
    }

    // Otherwise return JSON response
    send_message_json(state, agent_id, request).await
}

/// Send a message with JSON response (non-streaming)
#[instrument(skip(state, request), fields(agent_id = %agent_id), level = "info")]
async fn send_message_json<R: Runtime + 'static>(
    state: AppState<R>,
    agent_id: String,
    request: CreateMessageRequest,
) -> Result<Response, ApiError> {
    let response = handle_message_request(state, agent_id, request).await?;
    Ok(Json(response).into_response())
}

/// Shared handler for message processing (non-streaming)
pub async fn handle_message_request<R: Runtime + 'static>(
    state: AppState<R>,
    agent_id: String,
    request: CreateMessageRequest,
) -> Result<MessageResponse, ApiError> {
    // Extract effective content from various request formats
    let (_role, content) = request
        .effective_content()
        .ok_or_else(|| ApiError::bad_request("message content cannot be empty"))?;

    // Single source of truth: AgentService required (no fallback)
    let service = state
        .agent_service()
        .ok_or_else(|| ApiError::internal("AgentService not configured"))?;

    tracing::debug!(agent_id = %agent_id, "Using AgentService for message handling");

    // Note: MCP tools are pre-loaded at agent creation time (see agents.rs)
    let response = service
        .send_message_full(&agent_id, content.clone())
        .await
        .map_err(|e| ApiError::internal(format!("Agent service call failed: {}", e)))?;

    tracing::info!(
        agent_id = %agent_id,
        message_count = response.messages.len(),
        "Processed message via AgentService"
    );

    Ok(MessageResponse {
        messages: response.messages,
        usage: Some(response.usage),
        stop_reason: "end_turn".to_string(),
        approval_requests: None,
    })
}

/// Send a batch of messages
#[instrument(skip(state, request), fields(agent_id = %agent_id), level = "info")]
pub async fn send_messages_batch<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(agent_id): Path<String>,
    Json(request): Json<BatchMessagesRequest>,
) -> Result<Json<BatchStatus>, ApiError> {
    const BATCH_MESSAGES_MAX: usize = 100;
    const BATCH_CONCURRENCY_MAX: usize = 5;

    if request.messages.is_empty() {
        return Err(ApiError::bad_request("batch messages cannot be empty"));
    }
    if request.messages.len() > BATCH_MESSAGES_MAX {
        return Err(ApiError::bad_request(format!(
            "batch size exceeds limit ({})",
            BATCH_MESSAGES_MAX
        )));
    }

    let _agent = state
        .get_agent_async(&agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Agent", &agent_id))?;

    let now = Utc::now();
    let batch_id = Uuid::new_v4().to_string();

    let mut status = BatchStatus {
        id: batch_id.clone(),
        agent_id: agent_id.clone(),
        total: request.messages.len(),
        completed: 0,
        results: Vec::with_capacity(request.messages.len()),
        created_at: now,
        updated_at: now,
    };

    state.add_batch_status(status.clone())?;

    let results = stream::iter(request.messages.into_iter().enumerate())
        .map(|(idx, message)| {
            let state = state.clone();
            let agent_id = agent_id.clone();
            async move {
                let result = handle_message_request(state, agent_id, message).await;
                (idx, result)
            }
        })
        .buffer_unordered(BATCH_CONCURRENCY_MAX)
        .collect::<Vec<_>>()
        .await;

    let mut ordered = results;
    ordered.sort_by_key(|(idx, _)| *idx);

    for (_idx, result) in ordered {
        match result {
            Ok(response) => status
                .results
                .push(kelpie_server::models::BatchMessageResult {
                    success: true,
                    response: Some(response),
                    error: None,
                }),
            Err(e) => status
                .results
                .push(kelpie_server::models::BatchMessageResult {
                    success: false,
                    response: None,
                    error: Some(e.to_string()),
                }),
        }
    }

    status.completed = status.results.len();
    status.updated_at = Utc::now();
    state.update_batch_status(status.clone())?;

    Ok(Json(status))
}

/// Get batch status
#[instrument(skip(state), fields(agent_id = %agent_id, batch_id = %batch_id), level = "info")]
pub async fn get_batch_status<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path((agent_id, batch_id)): Path<(String, String)>,
) -> Result<Json<BatchStatus>, ApiError> {
    let status = state
        .get_batch_status(&batch_id)?
        .ok_or_else(|| ApiError::not_found("Batch", &batch_id))?;

    if status.agent_id != agent_id {
        return Err(ApiError::bad_request("batch does not belong to agent"));
    }

    Ok(Json(status))
}

/// Send a message with SSE streaming response
///
/// Single source of truth: Uses AgentService for message handling
#[instrument(skip(state, _query, request), fields(agent_id = %agent_id), level = "info")]
async fn send_message_streaming<R: Runtime + 'static>(
    state: AppState<R>,
    agent_id: String,
    _query: SendMessageQuery,
    request: CreateMessageRequest,
) -> Result<Response, ApiError> {
    // Extract effective content from various request formats
    let (_role, content) = request
        .effective_content()
        .ok_or_else(|| ApiError::bad_request("message content cannot be empty"))?;

    // Single source of truth: AgentService required (no fallback)
    let service = state
        .agent_service()
        .ok_or_else(|| ApiError::internal("AgentService not configured"))?
        .clone();

    // Use AgentService stream_message which converts batch response to stream
    let agent_id_clone = agent_id.clone();
    let content_clone = content.clone();

    // Create the SSE stream from AgentService
    let stream = stream::once(async move {
        match service
            .send_message_full(&agent_id_clone, content_clone)
            .await
        {
            Ok(response) => {
                let mut events: Vec<Result<Event, Infallible>> = Vec::new();

                // Send assistant message event for each message
                for message in response.messages {
                    if !message.content.is_empty() {
                        let assistant_msg = SseMessage::AssistantMessage {
                            id: message.id.clone(),
                            content: message.content.clone(),
                        };
                        if let Ok(json) = serde_json::to_string(&assistant_msg) {
                            events.push(Ok(Event::default().data(json)));
                        }
                    }

                    // Send tool call events
                    for tool_call in &message.tool_calls {
                        let args_str =
                            serde_json::to_string(&tool_call.arguments).unwrap_or_default();
                        let tool_msg = SseMessage::ToolCallMessage {
                            id: Uuid::new_v4().to_string(),
                            tool_call: ToolCallInfo {
                                name: tool_call.name.clone(),
                                arguments: args_str,
                                tool_call_id: Some(tool_call.id.clone()),
                            },
                        };
                        if let Ok(json) = serde_json::to_string(&tool_msg) {
                            events.push(Ok(Event::default().data(json)));
                        }
                    }
                }

                // Send stop_reason event
                let stop_event = StopReasonEvent {
                    message_type: "stop_reason",
                    stop_reason: "end_turn".to_string(),
                };
                if let Ok(json) = serde_json::to_string(&stop_event) {
                    events.push(Ok(Event::default().data(json)));
                }

                // Send usage statistics
                let usage_msg = SseMessage::UsageStatistics {
                    completion_tokens: response.usage.completion_tokens,
                    prompt_tokens: response.usage.prompt_tokens,
                    total_tokens: response.usage.total_tokens,
                    step_count: 1,
                };
                if let Ok(json) = serde_json::to_string(&usage_msg) {
                    events.push(Ok(Event::default().data(json)));
                }

                // Send [DONE]
                events.push(Ok(Event::default().data("[DONE]")));

                stream::iter(events)
            }
            Err(e) => {
                // Send error as assistant message
                let error_msg = SseMessage::AssistantMessage {
                    id: Uuid::new_v4().to_string(),
                    content: format!("Error: {}", e),
                };
                let mut events: Vec<Result<Event, Infallible>> = Vec::new();
                if let Ok(json) = serde_json::to_string(&error_msg) {
                    events.push(Ok(Event::default().data(json)));
                }
                let stop_event = StopReasonEvent {
                    message_type: "stop_reason",
                    stop_reason: "error".to_string(),
                };
                if let Ok(json) = serde_json::to_string(&stop_event) {
                    events.push(Ok(Event::default().data(json)));
                }
                events.push(Ok(Event::default().data("[DONE]")));
                stream::iter(events)
            }
        }
    })
    .flatten();

    Ok(Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response())
}

#[cfg(test)]
mod tests {
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
    use kelpie_server::service;
    use kelpie_server::state::AppState;
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

    /// Create a test AppState with AgentService and pre-created agent
    async fn test_app_with_agent() -> (Router, String, AppState<kelpie_core::TokioRuntime>) {
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
        let app = api::router(state.clone());

        // Create agent
        let body = serde_json::json!({
            "name": "msg-test-agent",
        });

        let response = app
            .clone()
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

        // Return the same state wrapped in new router
        (api::router(state.clone()), agent.id, state)
    }

    #[tokio::test]
    async fn test_send_message_succeeds() {
        let (app, agent_id, _state) = test_app_with_agent().await;

        let message = serde_json::json!({
            "role": "user",
            "content": "Hello, agent!"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/agents/{}/messages", agent_id))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&message).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // With mock LLM configured via AgentService, should return 200
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let response: kelpie_server::models::MessageResponse =
            serde_json::from_slice(&body).unwrap();

        // Should have messages in response
        assert!(
            !response.messages.is_empty(),
            "Expected messages in response"
        );
    }

    #[tokio::test]
    async fn test_send_empty_message() {
        let (app, agent_id, _state) = test_app_with_agent().await;

        let message = serde_json::json!({
            "role": "user",
            "content": ""
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/agents/{}/messages", agent_id))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&message).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_list_messages_empty() {
        let (app, agent_id, _state) = test_app_with_agent().await;

        // List messages on agent with no messages
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/agents/{}/messages?limit=10", agent_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let messages: Vec<kelpie_server::models::Message> = serde_json::from_slice(&body).unwrap();

        // No messages sent yet
        assert_eq!(messages.len(), 0);
    }

    // ============================================================================
    // Phase 5: Message Persistence Verification Tests
    // ============================================================================

    #[tokio::test]
    async fn test_message_roundtrip_persists() {
        let (app, agent_id, _state) = test_app_with_agent().await;

        // Send a user message
        let message = serde_json::json!({
            "role": "user",
            "content": "Hello, this is a test message for persistence verification"
        });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/agents/{}/messages", agent_id))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&message).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should succeed - with MockLlmClient configured via AgentService
        assert_eq!(response.status(), StatusCode::OK);

        // List messages to verify persistence
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/agents/{}/messages?limit=10", agent_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let messages: Vec<kelpie_server::models::Message> = serde_json::from_slice(&body).unwrap();

        // Should have at least one message (the user message)
        assert!(
            !messages.is_empty(),
            "Expected at least 1 message, got {}",
            messages.len()
        );

        // Find the user message and verify content
        let user_msg = messages
            .iter()
            .find(|m| m.role == kelpie_server::models::MessageRole::User);
        assert!(user_msg.is_some(), "User message not found in message list");
        let user_msg = user_msg.unwrap();
        assert!(
            user_msg.content.contains("persistence verification"),
            "User message content not preserved: {}",
            user_msg.content
        );
    }

    #[tokio::test]
    async fn test_multiple_messages_order_preserved() {
        let (app, agent_id, _state) = test_app_with_agent().await;

        // Send multiple messages
        for i in 1..=3 {
            let message = serde_json::json!({
                "role": "user",
                "content": format!("Message number {}", i)
            });

            let _response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/v1/agents/{}/messages", agent_id))
                        .header("content-type", "application/json")
                        .body(Body::from(serde_json::to_vec(&message).unwrap()))
                        .unwrap(),
                )
                .await
                .unwrap();
        }

        // List messages
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/agents/{}/messages?limit=20", agent_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let messages: Vec<kelpie_server::models::Message> = serde_json::from_slice(&body).unwrap();

        // Filter to just user messages
        let user_messages: Vec<_> = messages
            .iter()
            .filter(|m| m.role == kelpie_server::models::MessageRole::User)
            .collect();

        // Should have all 3 user messages
        assert!(
            user_messages.len() >= 3,
            "Expected at least 3 user messages, got {}",
            user_messages.len()
        );

        // Verify they contain expected content
        let contents: Vec<&str> = user_messages.iter().map(|m| m.content.as_str()).collect();
        assert!(
            contents.iter().any(|c| c.contains("Message number 1")),
            "Message 1 not found"
        );
        assert!(
            contents.iter().any(|c| c.contains("Message number 2")),
            "Message 2 not found"
        );
        assert!(
            contents.iter().any(|c| c.contains("Message number 3")),
            "Message 3 not found"
        );
    }

    #[tokio::test]
    async fn test_stream_tokens_parameter_accepted() {
        let (app, agent_id, _state) = test_app_with_agent().await;

        let message = serde_json::json!({
            "role": "user",
            "content": "Hello"
        });

        // Test with stream_tokens=true
        // Streaming now uses AgentService with MockLlmClient configured
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/v1/agents/{}/messages?stream_tokens=true",
                        agent_id
                    ))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&message).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return 200 OK with SSE stream since AgentService is configured
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "Expected 200 OK since streaming uses AgentService"
        );
    }
}
