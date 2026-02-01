//! Tools API endpoints
//!
//! TigerStyle: RESTful tool management with explicit validation.
//! Supports Letta SDK compatibility including upsert and client-side tools.

use crate::api::ApiError;
use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use kelpie_core::Runtime;
use kelpie_server::state::{AppState, ToolInfo};
use serde::{Deserialize, Serialize};
use tracing::instrument;
use uuid::Uuid;

/// Query parameters for listing tools
#[derive(Debug, Deserialize, Default)]
pub struct ListToolsQuery {
    /// Filter by tool name (exact match)
    pub name: Option<String>,
    /// Filter by tool ID
    pub id: Option<String>,
    /// Cursor for pagination - return tools after this ID
    pub after: Option<String>,
}

/// Tool definition for API responses (Letta-compatible)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResponse {
    /// Unique tool ID
    pub id: String,
    /// Tool name (used for invocation)
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// JSON schema for tool input
    #[serde(alias = "json_schema")]
    pub input_schema: serde_json::Value,
    /// Source code (for custom tools) - serialize as source_code for Letta compatibility
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "source_code",
        alias = "source"
    )]
    pub source: Option<String>,
    /// Whether tool requires user approval before execution
    #[serde(default)]
    pub default_requires_approval: bool,
    /// Tool type (builtin, custom, client)
    #[serde(default = "default_tool_type")]
    pub tool_type: String,
    /// Tags for categorization (Letta compatibility)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Character limit for return value (Letta compatibility)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_char_limit: Option<u32>,
}

fn default_tool_type() -> String {
    "custom".to_string()
}

impl From<ToolInfo> for ToolResponse {
    fn from(info: ToolInfo) -> Self {
        Self {
            id: info.id.clone(),
            name: info.name,
            description: info.description,
            input_schema: info.input_schema,
            source: info.source,
            default_requires_approval: info.default_requires_approval,
            tool_type: info.tool_type,
            tags: info.tags,
            return_char_limit: info.return_char_limit,
        }
    }
}

/// List of tools response
#[derive(Debug, Serialize)]
pub struct ToolListResponse {
    pub items: Vec<ToolResponse>,
    pub count: usize,
}

/// Request to register a tool (POST)
#[derive(Debug, Deserialize)]
pub struct RegisterToolRequest {
    /// Tool name (optional - can be extracted from source_code)
    #[serde(default)]
    pub name: Option<String>,
    /// Tool description (optional - will generate default)
    #[serde(default)]
    pub description: Option<String>,
    /// JSON schema for input parameters (optional - will use empty schema)
    #[serde(default, alias = "json_schema")]
    pub input_schema: Option<serde_json::Value>,
    /// Source code (optional for client-side tools)
    #[serde(default)]
    pub source: Option<String>,
    /// Alias for source
    #[serde(default, alias = "source_code")]
    pub source_code: Option<String>,
    /// Runtime (python, etc.)
    #[serde(default)]
    pub runtime: Option<String>,
    /// Package requirements (reserved for future use)
    #[serde(default)]
    #[allow(dead_code)]
    pub requirements: Vec<String>,
    /// Whether tool requires user approval (for client-side tools)
    #[serde(default)]
    pub default_requires_approval: bool,
    /// Tool type: "custom", "client", "builtin"
    #[serde(default)]
    pub tool_type: Option<String>,
    /// Tags for categorization (Letta compatibility)
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Character limit for return value (Letta compatibility)
    #[serde(default)]
    pub return_char_limit: Option<u32>,
}

/// Request to upsert a tool (PUT) - Letta SDK uses this
#[derive(Debug, Deserialize)]
pub struct UpsertToolRequest {
    /// Tool name (optional - can be extracted from source_code)
    #[serde(default)]
    pub name: Option<String>,
    /// Tool description (required for new tools)
    #[serde(default)]
    pub description: Option<String>,
    /// JSON schema for input parameters
    #[serde(default, alias = "json_schema", alias = "args_json_schema")]
    pub input_schema: Option<serde_json::Value>,
    /// Source code (optional for client-side tools)
    #[serde(default)]
    pub source: Option<String>,
    /// Alias for source
    #[serde(default, alias = "source_code")]
    pub source_code: Option<String>,
    /// Runtime (python, etc.) - reserved for future use
    #[serde(default)]
    #[allow(dead_code)]
    pub runtime: Option<String>,
    /// Package requirements (reserved for future use)
    #[serde(default)]
    #[allow(dead_code)]
    pub requirements: Option<Vec<String>>,
    /// Whether tool requires user approval (for client-side tools)
    #[serde(default)]
    pub default_requires_approval: bool,
    /// Tool type: "custom", "client", "builtin"
    #[serde(default)]
    pub tool_type: Option<String>,
    /// Tags for categorization (Letta compatibility)
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Character limit for return value (Letta compatibility)
    #[serde(default)]
    pub return_char_limit: Option<u32>,
}

/// Request to execute a tool
#[derive(Debug, Deserialize)]
pub struct ExecuteToolRequest {
    pub arguments: serde_json::Value,
}

/// Tool execution response
#[derive(Debug, Serialize)]
pub struct ExecuteToolResponse {
    pub success: bool,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Create the tools router
pub fn router<R: Runtime + 'static>() -> Router<AppState<R>> {
    Router::new()
        .route("/", get(list_tools).post(register_tool).put(upsert_tool))
        .route(
            "/:name_or_id",
            get(get_tool)
                .put(upsert_tool)
                .patch(update_tool)
                .delete(delete_tool),
        )
        .route("/:name/execute", post(execute_tool))
}

/// List all available tools with optional filtering
///
/// GET /v1/tools
/// GET /v1/tools?name=<name>
/// GET /v1/tools?id=<id>
#[instrument(skip(state), level = "info")]
async fn list_tools<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Query(query): Query<ListToolsQuery>,
) -> Json<ToolListResponse> {
    // Debug: Log the query parameters received
    tracing::info!(?query, "list_tools called with query params");

    let tools = state.list_tools().await;

    // Debug: Log total tools retrieved before filtering
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    tracing::info!(
        total_tools = tools.len(),
        ?tool_names,
        "list_tools retrieved from state"
    );

    // Apply filters
    let mut filtered: Vec<ToolResponse> = tools
        .into_iter()
        .map(ToolResponse::from)
        .filter(|t| {
            // Filter by name if specified
            if let Some(ref name) = query.name {
                if &t.name != name {
                    return false;
                }
            }
            // Filter by id if specified
            if let Some(ref id) = query.id {
                if &t.id != id {
                    return false;
                }
            }
            true
        })
        .collect();

    // Sort by ID for consistent pagination order
    filtered.sort_by(|a, b| a.id.cmp(&b.id));

    // Apply cursor-based pagination if 'after' is specified
    if let Some(ref after_id) = query.after {
        // DEBUG: Log all tool IDs and the cursor we're looking for
        let all_ids: Vec<&str> = filtered.iter().map(|t| t.id.as_str()).collect();
        tracing::info!(
            cursor_id = %after_id,
            ?all_ids,
            filtered_count = filtered.len(),
            "Pagination: searching for cursor in sorted list"
        );

        // Find the position of the cursor ID
        if let Some(cursor_pos) = filtered.iter().position(|t| &t.id == after_id) {
            tracing::info!(cursor_pos, "Found cursor at position");
            // Return only tools after the cursor
            filtered = filtered.into_iter().skip(cursor_pos + 1).collect();
        } else {
            tracing::warn!("Cursor ID not found in filtered list, returning empty");
            filtered.clear();
        }
    }

    let count = filtered.len();
    Json(ToolListResponse {
        items: filtered,
        count,
    })
}

/// Extract function name from Python source code
fn extract_function_name(source: &str) -> Option<String> {
    // Look for "def name(" or "async def name("
    let patterns = ["def ", "async def "];
    for pattern in patterns {
        if let Some(start) = source.find(pattern) {
            let after_def = &source[start + pattern.len()..];
            if let Some(paren) = after_def.find('(') {
                let name = after_def[..paren].trim();
                if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Upsert a tool (create or update)
///
/// PUT /v1/tools
///
/// Letta SDK uses this for tool registration with upsert semantics.
/// If tool exists, it updates; otherwise creates new.
/// Name can be provided explicitly or extracted from source_code.
#[instrument(skip(state, request), level = "info")]
async fn upsert_tool<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Json(request): Json<UpsertToolRequest>,
) -> Result<Json<ToolResponse>, ApiError> {
    // Determine source code first (needed for name extraction)
    let source_code = request.source_code.clone().or(request.source.clone());

    // Get tool name - either explicit or extracted from source_code
    let tool_name = match request.name.clone() {
        Some(name) if !name.is_empty() => name,
        _ => {
            // Try to extract from source_code
            source_code
                .as_ref()
                .and_then(|src| extract_function_name(src))
                .ok_or_else(|| ApiError::bad_request(
                    "Tool name is required (either provide 'name' field or include 'def <name>(' in source_code)"
                ))?
        }
    };

    // Validate tool name
    if tool_name.is_empty() {
        return Err(ApiError::bad_request("Tool name cannot be empty"));
    }

    if tool_name.len() > 64 {
        return Err(ApiError::bad_request(
            "Tool name too long (max 64 characters)",
        ));
    }

    // Check if tool already exists
    let existing = state.get_tool(&tool_name).await;

    // Determine tool type
    let tool_type = request.tool_type.unwrap_or_else(|| {
        if request.default_requires_approval {
            "client".to_string()
        } else if source_code.is_some() {
            "custom".to_string()
        } else {
            "client".to_string() // No source = client-side tool
        }
    });

    // For client-side tools, source_code is optional (they execute client-side)
    let is_client_tool = tool_type == "client" || request.default_requires_approval;

    if let Some(existing_tool) = existing {
        // Update existing tool
        let description = request.description.unwrap_or(existing_tool.description);
        let input_schema = request.input_schema.unwrap_or(existing_tool.input_schema);
        let source = source_code.or(existing_tool.source);

        let tags = request.tags.or(existing_tool.tags);
        let return_char_limit = request
            .return_char_limit
            .or(existing_tool.return_char_limit);

        let updated = state
            .upsert_tool(
                existing_tool.id,
                tool_name.clone(),
                description,
                input_schema,
                source,
                request.default_requires_approval,
                tool_type,
                tags,
                return_char_limit,
            )
            .await
            .map_err(|e| ApiError::internal(format!("Failed to update tool: {}", e)))?;

        tracing::info!(name = %tool_name, "Updated tool (upsert)");
        Ok(Json(ToolResponse::from(updated)))
    } else {
        // Create new tool
        let description = request
            .description
            .unwrap_or_else(|| format!("Client-side tool: {}", tool_name));

        let input_schema = request.input_schema.unwrap_or_else(|| {
            serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            })
        });

        // For non-client tools, validate source code
        if !is_client_tool {
            let code = source_code.as_ref().ok_or_else(|| {
                ApiError::bad_request("source_code is required for non-client tools")
            })?;

            let def_snippet = format!("def {}", tool_name);
            let async_def_snippet = format!("async def {}", tool_name);
            if !code.contains(&def_snippet) && !code.contains(&async_def_snippet) {
                return Err(ApiError::bad_request(
                    "source_code must define a function with the tool name",
                ));
            }
        }

        // Generate deterministic UUID from tool name for consistent IDs across requests
        let id = Uuid::new_v5(&Uuid::NAMESPACE_DNS, tool_name.as_bytes()).to_string();

        let registered = state
            .upsert_tool(
                id,
                tool_name.clone(),
                description,
                input_schema,
                source_code,
                request.default_requires_approval,
                tool_type,
                request.tags,
                request.return_char_limit,
            )
            .await
            .map_err(|e| ApiError::internal(format!("Failed to register tool: {}", e)))?;

        tracing::info!(name = %tool_name, "Registered tool (upsert)");
        Ok(Json(ToolResponse::from(registered)))
    }
}

/// Update an existing tool (partial update)
///
/// PATCH /v1/tools/:name_or_id
#[instrument(skip(state, request), fields(name_or_id = %name_or_id), level = "info")]
async fn update_tool<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(name_or_id): Path<String>,
    Json(request): Json<UpsertToolRequest>,
) -> Result<Json<ToolResponse>, ApiError> {
    // Resolve ID to name if needed, or use name directly
    let tool_name = if name_or_id.contains('-') && name_or_id.len() == 36 {
        // Looks like a UUID - get tool by ID to find its name
        if let Some(tool) = state.get_tool_by_id(&name_or_id).await {
            tool.name
        } else {
            return Err(ApiError::not_found("tool", &name_or_id));
        }
    } else {
        name_or_id
    };

    // Get existing tool
    let existing = state
        .get_tool(&tool_name)
        .await
        .ok_or_else(|| ApiError::not_found("tool", &tool_name))?;

    // Merge with partial update - use existing values if not provided
    let description = request.description.unwrap_or(existing.description);
    let input_schema = request.input_schema.unwrap_or(existing.input_schema);
    let source_code = request.source_code.or(request.source).or(existing.source);
    let default_requires_approval = request.default_requires_approval;
    let tool_type = request.tool_type.unwrap_or(existing.tool_type);
    let tags = request.tags.or(existing.tags);
    let return_char_limit = request.return_char_limit.or(existing.return_char_limit);

    // Update the tool
    let updated = state
        .upsert_tool(
            existing.id,
            tool_name.clone(),
            description,
            input_schema,
            source_code,
            default_requires_approval,
            tool_type,
            tags,
            return_char_limit,
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to update tool: {}", e)))?;

    tracing::info!(name = %tool_name, "Updated tool (PATCH)");
    Ok(Json(ToolResponse::from(updated)))
}

/// Register a new tool
///
/// POST /v1/tools
#[instrument(skip(state, request), level = "info")]
async fn register_tool<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Json(request): Json<RegisterToolRequest>,
) -> Result<Json<ToolResponse>, ApiError> {
    let source_code = request.source_code.or(request.source);

    // Extract tool name from request or source_code
    let tool_name = if let Some(name) = request.name {
        name
    } else {
        // Try to extract from source_code
        source_code
            .as_ref()
            .and_then(|src| extract_function_name(src))
            .ok_or_else(|| {
                ApiError::bad_request(
                    "Tool name is required (either provide 'name' field or include 'def <name>(' in source_code)",
                )
            })?
    };

    // Validate tool name
    if tool_name.is_empty() {
        return Err(ApiError::bad_request("Tool name cannot be empty"));
    }

    if tool_name.len() > 64 {
        return Err(ApiError::bad_request(
            "Tool name too long (max 64 characters)",
        ));
    }

    // Generate default description if not provided
    let description = request
        .description
        .unwrap_or_else(|| format!("Custom tool: {}", tool_name));

    // Generate default input schema if not provided
    let input_schema = request.input_schema.unwrap_or_else(|| {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    });

    // Determine tool type
    let tool_type = request.tool_type.unwrap_or_else(|| {
        if request.default_requires_approval {
            "client".to_string()
        } else if source_code.is_some() {
            "custom".to_string()
        } else {
            "client".to_string()
        }
    });

    // For non-client tools, validate source code
    let is_client_tool = tool_type == "client" || request.default_requires_approval;

    if !is_client_tool {
        let code = source_code
            .as_ref()
            .ok_or_else(|| ApiError::bad_request("source_code is required for non-client tools"))?;

        let runtime = request.runtime.as_deref().unwrap_or("python");
        let runtime_lc = runtime.to_lowercase();
        if runtime_lc != "python" && runtime_lc != "py" {
            return Err(ApiError::bad_request(format!(
                "unsupported runtime: {}",
                runtime
            )));
        }

        let def_snippet = format!("def {}", tool_name);
        let async_def_snippet = format!("async def {}", tool_name);
        if !code.contains(&def_snippet) && !code.contains(&async_def_snippet) {
            return Err(ApiError::bad_request(
                "source_code must define a function with the tool name",
            ));
        }
    }

    // Generate deterministic UUID from tool name for consistent IDs across requests
    let id = Uuid::new_v5(&Uuid::NAMESPACE_DNS, tool_name.as_bytes()).to_string();

    // Register the tool
    let registered = state
        .upsert_tool(
            id,
            tool_name.clone(),
            description,
            input_schema,
            source_code,
            request.default_requires_approval,
            tool_type,
            request.tags,
            request.return_char_limit,
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to register tool: {}", e)))?;

    tracing::info!(name = %tool_name, "Registered tool");

    Ok(Json(ToolResponse::from(registered)))
}

/// Get a specific tool by name or ID
#[instrument(skip(state), fields(name_or_id = %name_or_id), level = "info")]
async fn get_tool<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(name_or_id): Path<String>,
) -> Result<Json<ToolResponse>, ApiError> {
    // Try by ID first (if it looks like a UUID)
    if name_or_id.contains('-') && name_or_id.len() == 36 {
        if let Some(tool) = state.get_tool_by_id(&name_or_id).await {
            return Ok(Json(ToolResponse::from(tool)));
        }
    }

    // Fall back to name lookup
    let tool = state
        .get_tool(&name_or_id)
        .await
        .ok_or_else(|| ApiError::not_found("tool", &name_or_id))?;

    Ok(Json(ToolResponse::from(tool)))
}

/// Delete a tool by name or ID
#[instrument(skip(state), fields(name_or_id = %name_or_id), level = "info")]
async fn delete_tool<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(name_or_id): Path<String>,
) -> Result<(), ApiError> {
    // Resolve ID to name if needed
    let tool_name = if name_or_id.contains('-') && name_or_id.len() == 36 {
        // Looks like a UUID - get tool by ID to find its name
        if let Some(tool) = state.get_tool_by_id(&name_or_id).await {
            tool.name
        } else {
            return Err(ApiError::not_found("tool", &name_or_id));
        }
    } else {
        name_or_id
    };

    state
        .delete_tool(&tool_name)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to delete tool: {}", e)))?;
    tracing::info!(name = %tool_name, "Deleted tool");
    Ok(())
}

/// Execute a tool
#[instrument(skip(state, request), fields(name = %name), level = "info")]
async fn execute_tool<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(name): Path<String>,
    Json(request): Json<ExecuteToolRequest>,
) -> Result<Json<ExecuteToolResponse>, ApiError> {
    let result = state.execute_tool(&name, request.arguments).await;

    match result {
        Ok(output) => Ok(Json(ExecuteToolResponse {
            success: true,
            output,
            error: None,
        })),
        Err(e) => Ok(Json(ExecuteToolResponse {
            success: false,
            output: String::new(),
            error: Some(e.to_string()),
        })),
    }
}

#[cfg(test)]
mod tests {

    use crate::api;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use kelpie_server::state::AppState;
    use tower::ServiceExt;

    async fn test_app() -> Router {
        let state = AppState::new(kelpie_core::TokioRuntime);
        api::router(state)
    }

    #[tokio::test]
    async fn test_list_tools() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/tools")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_register_tool() {
        let app = test_app().await;

        let body = serde_json::json!({
            "name": "test_tool",
            "description": "A test tool",
            "input_schema": {
                "type": "object",
                "properties": {
                    "input": {"type": "string"}
                }
            },
            "source_code": "def test_tool(input: str) -> str:\n    return input\n"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/tools")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
