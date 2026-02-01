//! MCP Servers API endpoints
//!
//! TigerStyle: RESTful MCP server management with explicit validation.
//! Supports stdio, SSE, and streamable HTTP server types.

// Allow tokio::spawn and tokio::time::timeout in production server code
// This runs with real tokio runtime, not under DST
#![allow(clippy::disallowed_methods)]

use super::ApiError;
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use kelpie_core::Runtime;
use kelpie_server::models::{MCPServer, MCPServerConfig};
use kelpie_server::state::AppState;
use serde::{Deserialize, Serialize};
use tracing::instrument;

/// Request to create MCP server
#[derive(Debug, Deserialize)]
pub struct CreateMCPServerRequest {
    pub server_name: String,
    pub config: MCPServerConfig,
}

/// Request to update MCP server
#[derive(Debug, Deserialize)]
pub struct UpdateMCPServerRequest {
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub config: Option<MCPServerConfig>,
}

/// MCP server response (flattened for Letta compatibility)
#[derive(Debug, Clone, Serialize)]
pub struct MCPServerResponse {
    pub id: String,
    pub server_name: String,
    #[serde(flatten)]
    pub config: MCPServerConfig,
    pub created_at: String,
    pub updated_at: String,
}

impl From<MCPServer> for MCPServerResponse {
    fn from(server: MCPServer) -> Self {
        Self {
            id: server.id,
            server_name: server.server_name,
            config: server.config,
            created_at: server.created_at.to_rfc3339(),
            updated_at: server.updated_at.to_rfc3339(),
        }
    }
}

/// Create router for MCP servers endpoints
pub fn router<R: Runtime + 'static>() -> Router<AppState<R>> {
    Router::new()
        .route("/", get(list_servers).post(create_server))
        .route(
            "/:server_id",
            get(get_server)
                .put(update_server)
                .patch(update_server)
                .delete(delete_server),
        )
        .route("/:server_id/tools", get(list_server_tools))
        .route("/:server_id/tools/:tool_id", get(get_server_tool))
        .route("/:server_id/tools/:tool_id/run", post(run_server_tool))
}

/// List all MCP servers
///
/// GET /v1/mcp-servers/
#[instrument(skip(state), level = "info")]
async fn list_servers<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
) -> Json<Vec<MCPServerResponse>> {
    let servers = state.list_mcp_servers().await;
    let items: Vec<MCPServerResponse> = servers.into_iter().map(MCPServerResponse::from).collect();

    Json(items)
}

/// Create a new MCP server
///
/// POST /v1/mcp-servers/
#[instrument(skip(state, request), level = "info")]
async fn create_server<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Json(request): Json<CreateMCPServerRequest>,
) -> Result<Json<MCPServerResponse>, ApiError> {
    // Validate server name
    if request.server_name.is_empty() {
        return Err(ApiError::bad_request("Server name cannot be empty"));
    }

    if request.server_name.len() > 128 {
        return Err(ApiError::bad_request(
            "Server name too long (max 128 characters)",
        ));
    }

    // Create the server
    let server = state
        .create_mcp_server(request.server_name.clone(), request.config)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to create MCP server: {}", e)))?;

    tracing::info!(server_id = %server.id, server_name = %server.server_name, "Created MCP server");

    // TigerStyle: Spawn async tool discovery in background to avoid blocking server creation
    // This prevents timeouts when MCP server subprocess is slow to start
    // Convert server config to McpConfig using shared helper
    let mcp_config =
        AppState::<R>::mcp_server_config_to_mcp_config(&server.server_name, &server.config);

    // Spawn background task for async tool discovery with timeout
    // TigerStyle: Non-blocking tool discovery - server returns immediately
    let state_clone = state.clone();
    let server_name = server.server_name.clone();
    let server_id = server.id.clone();

    tokio::spawn(async move {
        use std::time::Duration;

        // TigerStyle: 30-second timeout prevents indefinite blocking
        const TOOL_DISCOVERY_TIMEOUT_MS: u64 = 30_000;

        let discovery_result = tokio::time::timeout(
            Duration::from_millis(TOOL_DISCOVERY_TIMEOUT_MS),
            state_clone
                .tool_registry()
                .connect_mcp_server(&server_name, mcp_config),
        )
        .await;

        match discovery_result {
            Ok(Ok(tool_count)) => {
                tracing::info!(
                    server_id = %server_id,
                    server_name = %server_name,
                    tool_count = tool_count,
                    "Connected to MCP server and registered tools"
                );
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    server_id = %server_id,
                    server_name = %server_name,
                    error = %e,
                    "Failed to connect to MCP server or discover tools"
                );
            }
            Err(_) => {
                tracing::warn!(
                    server_id = %server_id,
                    server_name = %server_name,
                    timeout_ms = TOOL_DISCOVERY_TIMEOUT_MS,
                    "Tool discovery timed out - MCP server may be slow to start or unresponsive"
                );
            }
        }
    });

    Ok(Json(MCPServerResponse::from(server)))
}

/// Get a specific MCP server
///
/// GET /v1/mcp-servers/{server_id}
#[instrument(skip(state), fields(server_id = %server_id), level = "info")]
async fn get_server<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(server_id): Path<String>,
) -> Result<Json<MCPServerResponse>, ApiError> {
    let server = state
        .get_mcp_server(&server_id)
        .await
        .ok_or_else(|| ApiError::not_found("MCP server", &server_id))?;

    Ok(Json(MCPServerResponse::from(server)))
}

/// Update an MCP server
///
/// PUT/PATCH /v1/mcp-servers/{server_id}
#[instrument(skip(state, request), fields(server_id = %server_id), level = "info")]
async fn update_server<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(server_id): Path<String>,
    Json(request): Json<UpdateMCPServerRequest>,
) -> Result<Json<MCPServerResponse>, ApiError> {
    let server = state
        .update_mcp_server(&server_id, request.server_name, request.config)
        .await
        .map_err(|e| match e.to_string().as_str() {
            s if s.contains("not found") => ApiError::not_found("MCP server", &server_id),
            _ => ApiError::internal(format!("Failed to update MCP server: {}", e)),
        })?;

    tracing::info!(server_id = %server.id, "Updated MCP server");

    Ok(Json(MCPServerResponse::from(server)))
}

/// Delete an MCP server
///
/// DELETE /v1/mcp-servers/{server_id}
#[instrument(skip(state), fields(server_id = %server_id), level = "info")]
async fn delete_server<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(server_id): Path<String>,
) -> Result<(), ApiError> {
    // TigerStyle: Get server details before deletion for proper cleanup
    let server = state
        .get_mcp_server(&server_id)
        .await
        .ok_or_else(|| ApiError::not_found("MCP server", &server_id))?;

    // TigerStyle: Disconnect MCP client and clean up resources
    // This prevents resource leaks from accumulated connections
    let registry = state.tool_registry();
    if let Err(e) = registry.disconnect_mcp_server(&server.server_name).await {
        tracing::warn!(
            server_id = %server_id,
            server_name = %server.server_name,
            error = %e,
            "Failed to disconnect MCP client during delete (may not be connected)"
        );
    } else {
        tracing::info!(
            server_id = %server_id,
            server_name = %server.server_name,
            "Disconnected MCP client and unregistered tools"
        );
    }

    // Delete the server record from storage
    state
        .delete_mcp_server(&server_id)
        .await
        .map_err(|e| match e.to_string().as_str() {
            s if s.contains("not found") => ApiError::not_found("MCP server", &server_id),
            _ => ApiError::internal(format!("Failed to delete MCP server: {}", e)),
        })?;

    tracing::info!(server_id = %server_id, "Deleted MCP server");

    Ok(())
}

/// List tools provided by an MCP server
///
/// GET /v1/mcp-servers/{server_id}/tools
#[instrument(skip(state), fields(server_id = %server_id), level = "info")]
async fn list_server_tools<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(server_id): Path<String>,
) -> Result<Json<Vec<super::tools::ToolResponse>>, ApiError> {
    // Discover tools from the MCP server (returns JSON Values)
    let tool_values = state
        .list_mcp_server_tools(&server_id)
        .await
        .map_err(|e| match e {
            kelpie_server::state::StateError::NotFound { resource, id } => {
                ApiError::not_found(resource, &id)
            }
            _ => ApiError::internal(format!("Failed to discover MCP server tools: {}", e)),
        })?;

    // Convert JSON Values to ToolResponse
    let tools: Vec<super::tools::ToolResponse> = tool_values
        .into_iter()
        .filter_map(|value| serde_json::from_value(value).ok())
        .collect();

    tracing::info!(server_id = %server_id, tool_count = tools.len(), "Discovered MCP server tools");

    Ok(Json(tools))
}

/// Get a specific tool provided by an MCP server
///
/// GET /v1/mcp-servers/{server_id}/tools/{tool_id}
#[instrument(skip(state), fields(server_id = %server_id, tool_id = %tool_id), level = "info")]
async fn get_server_tool<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path((server_id, tool_id)): Path<(String, String)>,
) -> Result<Json<super::tools::ToolResponse>, ApiError> {
    // Discover tools from the MCP server (returns JSON Values)
    let tool_values = state
        .list_mcp_server_tools(&server_id)
        .await
        .map_err(|e| match e {
            kelpie_server::state::StateError::NotFound { resource, id } => {
                ApiError::not_found(resource, &id)
            }
            _ => ApiError::internal(format!("Failed to discover MCP server tools: {}", e)),
        })?;

    // Convert JSON Values to ToolResponse and find the requested tool
    let tools: Vec<super::tools::ToolResponse> = tool_values
        .into_iter()
        .filter_map(|value| serde_json::from_value(value).ok())
        .collect();

    let tool = tools
        .into_iter()
        .find(|t| t.id == tool_id)
        .ok_or_else(|| ApiError::not_found("MCP server tool", &tool_id))?;

    tracing::info!(server_id = %server_id, tool_id = %tool_id, "Retrieved MCP server tool");

    Ok(Json(tool))
}

/// Request body for running an MCP server tool
#[derive(Debug, Deserialize)]
pub struct RunToolRequest {
    #[serde(default = "default_arguments")]
    pub arguments: serde_json::Value,
}

fn default_arguments() -> serde_json::Value {
    serde_json::json!({})
}

/// Execute a tool on an MCP server
///
/// POST /v1/mcp-servers/{server_id}/tools/{tool_id}/run
#[instrument(skip(state, request), fields(server_id = %server_id, tool_id = %tool_id), level = "info")]
async fn run_server_tool<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path((server_id, tool_id)): Path<(String, String)>,
    Json(request): Json<RunToolRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Extract tool name from tool_id
    // Tool ID format: mcp_{server_id}_{tool_name}
    let tool_name = tool_id
        .strip_prefix(&format!("mcp_{}_", server_id))
        .ok_or_else(|| ApiError::bad_request(format!("Invalid tool ID format: {}", tool_id)))?;

    // Execute the tool
    let result = state
        .execute_mcp_server_tool(&server_id, tool_name, request.arguments)
        .await
        .map_err(|e| match e {
            kelpie_server::state::StateError::NotFound { resource, id } => {
                ApiError::not_found(resource, &id)
            }
            _ => ApiError::internal(format!("Failed to execute MCP server tool: {}", e)),
        })?;

    tracing::info!(server_id = %server_id, tool_id = %tool_id, "Executed MCP server tool");

    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::super::router as api_router;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use kelpie_server::state::AppState;
    use tower::ServiceExt;

    async fn test_app() -> Router {
        let state = AppState::new(kelpie_core::TokioRuntime);
        api_router(state)
    }

    #[tokio::test]
    async fn test_list_mcp_servers_empty() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/mcp-servers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_create_stdio_mcp_server() {
        let app = test_app().await;

        let body = serde_json::json!({
            "server_name": "test-server",
            "config": {
                "mcp_server_type": "stdio",
                "command": "python",
                "args": ["server.py"]
            }
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/mcp-servers")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
