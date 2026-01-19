//! MCP Servers API endpoints
//!
//! TigerStyle: RESTful MCP server management with explicit validation.
//! Supports stdio, SSE, and streamable HTTP server types.

use super::ApiError;
use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
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
pub fn router() -> Router<AppState> {
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
}

/// List all MCP servers
///
/// GET /v1/mcp-servers/
#[instrument(skip(state), level = "info")]
async fn list_servers(State(state): State<AppState>) -> Json<Vec<MCPServerResponse>> {
    let servers = state.list_mcp_servers().await;
    let items: Vec<MCPServerResponse> = servers.into_iter().map(MCPServerResponse::from).collect();

    Json(items)
}

/// Create a new MCP server
///
/// POST /v1/mcp-servers/
#[instrument(skip(state, request), level = "info")]
async fn create_server(
    State(state): State<AppState>,
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
        .create_mcp_server(request.server_name, request.config)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to create MCP server: {}", e)))?;

    tracing::info!(server_id = %server.id, server_name = %server.server_name, "Created MCP server");

    Ok(Json(MCPServerResponse::from(server)))
}

/// Get a specific MCP server
///
/// GET /v1/mcp-servers/{server_id}
#[instrument(skip(state), fields(server_id = %server_id), level = "info")]
async fn get_server(
    State(state): State<AppState>,
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
async fn update_server(
    State(state): State<AppState>,
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
async fn delete_server(
    State(state): State<AppState>,
    Path(server_id): Path<String>,
) -> Result<(), ApiError> {
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
async fn list_server_tools(
    State(state): State<AppState>,
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

#[cfg(test)]
mod tests {
    use super::super::router as api_router;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use kelpie_server::state::AppState;
    use tower::ServiceExt;

    async fn test_app() -> Router {
        let state = AppState::new();
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
