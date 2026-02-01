//! Scheduling API endpoints
//!
//! TigerStyle: Letta-compatible job scheduling for periodic agent tasks.
//!
//! Phase 5: Provides API for managing scheduled jobs (cron-like tasks)
//! for agents. Jobs are stored but execution is deferred to background workers.

use crate::api::ApiError;
use axum::{extract::Path, extract::Query, routing::get, Router};
use axum::{extract::State, Json};
use kelpie_core::Runtime;
use kelpie_server::models::{CreateJobRequest, Job, UpdateJobRequest};
use kelpie_server::state::AppState;
use serde::Deserialize;
use tracing::instrument;

/// TigerStyle: Explicit constants with units
const JOBS_PER_AGENT_MAX: usize = 100;
const SCHEDULE_PATTERN_LENGTH_MAX: usize = 256;

/// Create scheduling routes
pub fn router<R: Runtime + 'static>() -> Router<AppState<R>> {
    Router::new()
        .route("/jobs", get(list_jobs).post(create_job))
        .route(
            "/jobs/:job_id",
            get(get_job).patch(update_job).delete(delete_job),
        )
}

/// Create a scheduled job
///
/// POST /v1/jobs
#[instrument(skip(state, request), fields(agent_id = %request.agent_id, action = ?request.action), level = "info")]
async fn create_job<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Json(request): Json<CreateJobRequest>,
) -> Result<Json<Job>, ApiError> {
    // Validate agent exists
    let _agent = state
        .get_agent_async(&request.agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Agent", &request.agent_id))?;

    // Validate schedule pattern length
    if request.schedule.len() > SCHEDULE_PATTERN_LENGTH_MAX {
        return Err(ApiError::bad_request(format!(
            "schedule pattern too long (max {} characters)",
            SCHEDULE_PATTERN_LENGTH_MAX
        )));
    }

    // Validate schedule pattern
    if request.schedule.trim().is_empty() {
        return Err(ApiError::bad_request("schedule pattern cannot be empty"));
    }

    // Check job count limit per agent
    let existing_jobs = state.list_jobs_for_agent(&request.agent_id)?;
    if existing_jobs.len() >= JOBS_PER_AGENT_MAX {
        return Err(ApiError::bad_request(format!(
            "job limit exceeded for agent (max {})",
            JOBS_PER_AGENT_MAX
        )));
    }

    // Create job
    let job = Job::from_request(request);
    state.add_job(job.clone())?;

    tracing::info!(
        job_id = %job.id,
        agent_id = %job.agent_id,
        action = ?job.action,
        schedule_type = ?job.schedule_type,
        "created scheduled job"
    );

    Ok(Json(job))
}

/// Get a specific job
///
/// GET /v1/jobs/{job_id}
#[instrument(skip(state), fields(job_id = %job_id), level = "info")]
async fn get_job<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(job_id): Path<String>,
) -> Result<Json<Job>, ApiError> {
    let job = state
        .get_job(&job_id)?
        .ok_or_else(|| ApiError::not_found("Job", &job_id))?;

    Ok(Json(job))
}

/// List all jobs (optionally filtered by agent_id)
///
/// GET /v1/jobs?agent_id={agent_id}
#[instrument(skip(state, query), fields(agent_id = ?query.agent_id), level = "info")]
async fn list_jobs<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Query(query): Query<ListJobsQuery>,
) -> Result<Json<Vec<Job>>, ApiError> {
    let jobs = state.list_all_jobs(query.agent_id.as_deref())?;

    Ok(Json(jobs))
}

/// Query parameters for listing jobs
#[derive(Debug, Deserialize)]
struct ListJobsQuery {
    /// Filter by agent ID
    agent_id: Option<String>,
}

/// Update a job
///
/// PATCH /v1/jobs/{job_id}
#[instrument(skip(state, request), fields(job_id = %job_id), level = "info")]
async fn update_job<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(job_id): Path<String>,
    Json(request): Json<UpdateJobRequest>,
) -> Result<Json<Job>, ApiError> {
    let mut job = state
        .get_job(&job_id)?
        .ok_or_else(|| ApiError::not_found("Job", &job_id))?;

    job.apply_update(request);
    state.update_job(job.clone())?;

    tracing::info!(
        job_id = %job.id,
        status = ?job.status,
        "updated scheduled job"
    );

    Ok(Json(job))
}

/// Delete a job
///
/// DELETE /v1/jobs/{job_id}
#[instrument(skip(state), fields(job_id = %job_id), level = "info")]
async fn delete_job<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(job_id): Path<String>,
) -> Result<(), ApiError> {
    state.delete_job(&job_id)?;

    tracing::info!(job_id = %job_id, "deleted scheduled job");

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
    use kelpie_server::models::{AgentState, JobAction, JobStatus, ScheduleType};
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

    /// Helper: Create a test agent
    async fn create_test_agent(app: &Router) -> String {
        let create_body = serde_json::json!({
            "name": "test-agent",
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

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let agent: AgentState = serde_json::from_slice(&body).unwrap();

        agent.id
    }

    #[tokio::test]
    async fn test_create_job() {
        let app = test_app().await;
        let agent_id = create_test_agent(&app).await;

        let job_request = serde_json::json!({
            "agent_id": agent_id,
            "schedule_type": "interval",
            "schedule": "3600",
            "action": "summarize_conversation",
            "description": "Hourly conversation summary"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/jobs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&job_request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let job: Job = serde_json::from_slice(&body).unwrap();

        assert_eq!(job.agent_id, agent_id);
        assert_eq!(job.schedule_type, ScheduleType::Interval);
        assert_eq!(job.schedule, "3600");
        assert_eq!(job.action, JobAction::SummarizeConversation);
        assert_eq!(job.status, JobStatus::Active);
    }

    #[tokio::test]
    async fn test_create_job_nonexistent_agent() {
        let app = test_app().await;

        let job_request = serde_json::json!({
            "agent_id": "nonexistent",
            "schedule_type": "interval",
            "schedule": "3600",
            "action": "summarize_conversation"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/jobs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&job_request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_create_job_empty_schedule() {
        let app = test_app().await;
        let agent_id = create_test_agent(&app).await;

        let job_request = serde_json::json!({
            "agent_id": agent_id,
            "schedule_type": "interval",
            "schedule": "",
            "action": "summarize_conversation"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/jobs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&job_request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_list_jobs_empty() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/jobs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let jobs: Vec<Job> = serde_json::from_slice(&body).unwrap();

        assert_eq!(jobs.len(), 0);
    }

    #[tokio::test]
    async fn test_get_job_not_found() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/jobs/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_job() {
        let app = test_app().await;
        let agent_id = create_test_agent(&app).await;

        // Create job
        let job_request = serde_json::json!({
            "agent_id": agent_id,
            "schedule_type": "interval",
            "schedule": "3600",
            "action": "summarize_conversation"
        });

        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/jobs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&job_request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let job: Job = serde_json::from_slice(&body).unwrap();

        // Delete job
        let delete_response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/v1/jobs/{}", job.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(delete_response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_update_job() {
        let app = test_app().await;
        let agent_id = create_test_agent(&app).await;

        // Create job
        let job_request = serde_json::json!({
            "agent_id": agent_id,
            "schedule_type": "interval",
            "schedule": "3600",
            "action": "summarize_conversation"
        });

        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/jobs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&job_request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let job: Job = serde_json::from_slice(&body).unwrap();

        // Update job status to paused
        let update_request = serde_json::json!({
            "status": "paused"
        });

        let update_response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/v1/jobs/{}", job.id))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&update_request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(update_response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(update_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let updated_job: Job = serde_json::from_slice(&body).unwrap();

        assert_eq!(updated_job.status, JobStatus::Paused);
    }

    // ============================================================================
    // Phase 5: Job Persistence Verification Tests
    // ============================================================================

    #[tokio::test]
    async fn test_job_delete_removes_from_storage() {
        let app = test_app().await;
        let agent_id = create_test_agent(&app).await;

        // Create job
        let job_request = serde_json::json!({
            "agent_id": agent_id,
            "schedule_type": "interval",
            "schedule": "3600",
            "action": "summarize_conversation"
        });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/jobs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&job_request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let job: Job = serde_json::from_slice(&body).unwrap();
        let job_id = job.id.clone();

        // Verify job exists
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/jobs/{}", job_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Delete job
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/v1/jobs/{}", job_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Verify job is gone
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/jobs/{}", job_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_job_update_persists() {
        let app = test_app().await;
        let agent_id = create_test_agent(&app).await;

        // Create job
        let job_request = serde_json::json!({
            "agent_id": agent_id,
            "schedule_type": "interval",
            "schedule": "3600",
            "action": "summarize_conversation",
            "description": "Original description"
        });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/jobs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&job_request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let job: Job = serde_json::from_slice(&body).unwrap();
        let job_id = job.id.clone();

        // Update job
        let update_request = serde_json::json!({
            "status": "paused",
            "description": "Updated description"
        });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/v1/jobs/{}", job_id))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&update_request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Read job back to verify update persisted
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/jobs/{}", job_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let fetched: Job = serde_json::from_slice(&body).unwrap();

        // Verify updates persisted
        assert_eq!(fetched.status, JobStatus::Paused);
        assert_eq!(fetched.description.as_deref(), Some("Updated description"));
    }
}
