//! REST API module
//!
//! TigerStyle: Letta-compatible REST API for agent management.
//!
//! HTTP Linearizability: Idempotency middleware provides exactly-once semantics
//! for mutating operations. See ADR-030 and `docs/tla/KelpieHttpApi.tla`.

pub mod agent_groups;
pub mod agents;
pub mod archival;
pub mod blocks;
pub mod groups;
pub mod idempotency;
pub mod identities;
pub mod import_export;
pub mod mcp_servers;
pub mod messages;
pub mod projects;
pub mod scheduling;
pub mod standalone_blocks;
pub mod streaming;
pub mod summarization;
pub mod teleport;
pub mod testing;
pub mod tools;

use axum::{
    extract::State,
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use kelpie_core::Runtime;
use kelpie_server::models::{ErrorResponse, HealthResponse};
use kelpie_server::state::{AppState, StateError};
use serde::Serialize;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use self::idempotency::IdempotencyCache;

/// Create the API router with all routes
///
/// TLA+ Reference: `docs/tla/KelpieHttpApi.tla`
/// ADR: `docs/adr/030-http-linearizability.md`
///
/// Idempotency middleware is applied to mutating endpoints (POST, PUT, DELETE)
/// to provide exactly-once semantics when clients use the `Idempotency-Key` header.
pub fn router<R: Runtime + 'static>(state: AppState<R>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Create idempotency cache for exactly-once semantics
    let idempotency_cache = Arc::new(IdempotencyCache::new());

    Router::new()
        // Health check (no idempotency needed - read-only)
        .route("/health", get(health_check))
        .route("/v1/health", get(health_check))
        // Metrics endpoint (Prometheus - read-only)
        .route("/metrics", get(metrics))
        // Capabilities (read-only)
        .route("/v1/capabilities", get(capabilities))
        // Agent routes
        .nest(
            "/v1/agents",
            agents::router().merge(summarization::router()),
        )
        // Standalone blocks routes (letta-code compatibility)
        .nest("/v1/blocks", standalone_blocks::router())
        // Tool routes
        .nest("/v1/tools", tools::router())
        // MCP servers routes (Letta compatibility)
        .nest("/v1/mcp-servers", mcp_servers::router())
        // Agent groups routes (Phase 8)
        .nest("/v1", agent_groups::router())
        // Groups routes (Letta compatibility alias for agent_groups)
        .nest("/v1", groups::router())
        // Identities routes
        .nest("/v1", identities::router())
        // Teleport routes
        .nest("/v1/teleport", teleport::router())
        // Scheduling routes (Phase 5)
        .nest("/v1", scheduling::router())
        // Projects routes (Phase 6)
        .nest("/v1", projects::router())
        // Test API routes (E2E testing)
        .nest("/v1/test", testing::router())
        // Idempotency middleware for exactly-once semantics on mutating requests
        // TLA+ Invariant: IdempotencyGuarantee, ExactlyOnceExecution
        .layer(middleware::from_fn_with_state(
            idempotency_cache,
            idempotency::idempotency_middleware,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

/// Server capabilities endpoint
async fn capabilities() -> Json<CapabilitiesResponse> {
    Json(CapabilitiesResponse {
        features: vec![
            "agents".to_string(),
            "memory_blocks".to_string(),
            "messages".to_string(),
            "tools".to_string(),
            "archival_memory".to_string(),
            "semantic_search".to_string(),
            "teleport".to_string(),
        ],
        api_version: "v1".to_string(),
        llm_models: vec![
            "anthropic/claude-sonnet-4".to_string(),
            "openai/gpt-4o".to_string(),
        ],
    })
}

/// Server capabilities response
#[derive(Debug, Serialize)]
struct CapabilitiesResponse {
    features: Vec<String>,
    api_version: String,
    llm_models: Vec<String>,
}

/// Health check endpoint
async fn health_check<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: state.uptime_seconds(),
    })
}

/// Prometheus metrics endpoint
///
/// Returns metrics in Prometheus text format.
/// This is scraped by Prometheus servers for monitoring.
async fn metrics<R: Runtime + 'static>(State(state): State<AppState<R>>) -> Response {
    // Calculate and record memory metrics
    let _ = state.record_memory_metrics();

    let agent_count = state.agent_count().unwrap_or(0);
    let uptime_seconds = state.uptime_seconds();

    #[cfg(feature = "otel")]
    {
        // If we have a Prometheus registry, combine its metrics with manual metrics
        if let Some(registry) = state.prometheus_registry() {
            use prometheus::Encoder;
            let encoder = prometheus::TextEncoder::new();
            let metric_families = registry.gather();
            let mut buffer = Vec::new();

            // Encode OpenTelemetry metrics
            if encoder.encode(&metric_families, &mut buffer).is_ok() {
                // Append manual metrics that aren't in OpenTelemetry yet
                let manual_metrics = format!(
                    "\n# HELP kelpie_agents_active_count Current number of active agents\n\
                     # TYPE kelpie_agents_active_count gauge\n\
                     kelpie_agents_active_count {}\n\
                     \n\
                     # HELP kelpie_server_uptime_seconds Server uptime in seconds\n\
                     # TYPE kelpie_server_uptime_seconds gauge\n\
                     kelpie_server_uptime_seconds {}\n",
                    agent_count, uptime_seconds
                );
                buffer.extend_from_slice(manual_metrics.as_bytes());

                return (
                    StatusCode::OK,
                    [(
                        axum::http::header::CONTENT_TYPE,
                        "text/plain; version=0.0.4; charset=utf-8",
                    )],
                    buffer,
                )
                    .into_response();
            }
        }
    }

    // Fallback: manual metrics formatting (when otel feature not enabled or registry not configured)
    let metrics_text = format!(
        "# HELP kelpie_agents_active_count Current number of active agents\n\
         # TYPE kelpie_agents_active_count gauge\n\
         kelpie_agents_active_count {}\n\
         \n\
         # HELP kelpie_server_uptime_seconds Server uptime in seconds\n\
         # TYPE kelpie_server_uptime_seconds gauge\n\
         kelpie_server_uptime_seconds {}\n",
        agent_count, uptime_seconds
    );

    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        metrics_text,
    )
        .into_response()
}

/// API error type that converts to HTTP responses
pub struct ApiError {
    status: StatusCode,
    body: ErrorResponse,
}

impl ApiError {
    pub fn not_found(resource: &str, id: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            body: ErrorResponse::not_found(resource, id),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: ErrorResponse::bad_request(message),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            body: ErrorResponse::internal(message),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            body: ErrorResponse::new("conflict", message),
        }
    }

    pub fn unprocessable_entity(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            body: ErrorResponse::new("unprocessable_entity", message),
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.body.code, self.body.message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

impl From<StateError> for ApiError {
    fn from(err: StateError) -> Self {
        match err {
            StateError::NotFound { resource, id } => ApiError::not_found(resource, &id),
            StateError::AlreadyExists { resource, id } => {
                ApiError::conflict(format!("{} with id '{}' already exists", resource, id))
            }
            StateError::LimitExceeded { resource, limit } => {
                ApiError::bad_request(format!("{} limit ({}) exceeded", resource, limit))
            }
            StateError::LockPoisoned => ApiError::internal("internal state error"),
            StateError::FaultInjected { operation } => {
                // DST fault injection - return as internal error
                ApiError::internal(format!("operation failed: {}", operation))
            }
            StateError::Internal { message } => {
                // Service errors or other internal errors
                ApiError::internal(message)
            }
            StateError::StorageError { message } => {
                // Storage layer errors (FDB, SimStorage, etc.)
                ApiError::internal(format!("storage error: {}", message))
            }
        }
    }
}
