//! Projects API endpoints
//!
//! TigerStyle: Letta-compatible project management for organizing agents.
//!
//! Phase 6: Provides API for managing projects (organizational units for agents).
//! Projects help organize agents by use case, team, or environment.

use crate::api::ApiError;
use axum::{extract::Path, extract::Query, routing::get, Router};
use axum::{extract::State, Json};
use kelpie_core::Runtime;
use kelpie_server::models::{CreateProjectRequest, ListResponse, Project, UpdateProjectRequest};
use kelpie_server::state::AppState;
use serde::Deserialize;
use tracing::instrument;

/// TigerStyle: Explicit constants with units
const PROJECTS_COUNT_MAX: usize = 1_000;
const PROJECT_NAME_LENGTH_MAX: usize = 256;

/// Create projects routes
pub fn router<R: Runtime + 'static>() -> Router<AppState<R>> {
    Router::new()
        .route("/projects", get(list_projects).post(create_project))
        .route(
            "/projects/:project_id",
            get(get_project)
                .patch(update_project)
                .delete(delete_project),
        )
        .route("/projects/:project_id/agents", get(list_project_agents))
}

/// Create a project
///
/// POST /v1/projects
#[instrument(skip(state, request), fields(name = %request.name), level = "info")]
async fn create_project<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Json(request): Json<CreateProjectRequest>,
) -> Result<Json<Project>, ApiError> {
    // Validate name
    if request.name.trim().is_empty() {
        return Err(ApiError::bad_request("project name cannot be empty"));
    }

    if request.name.len() > PROJECT_NAME_LENGTH_MAX {
        return Err(ApiError::bad_request(format!(
            "project name too long (max {} characters)",
            PROJECT_NAME_LENGTH_MAX
        )));
    }

    // Check project count limit
    let (existing_projects, _) = state.list_projects(None)?;
    if existing_projects.len() >= PROJECTS_COUNT_MAX {
        return Err(ApiError::bad_request(format!(
            "project limit exceeded (max {})",
            PROJECTS_COUNT_MAX
        )));
    }

    // Create project
    let project = Project::from_request(request);
    state.add_project(project.clone())?;

    tracing::info!(
        project_id = %project.id,
        name = %project.name,
        "created project"
    );

    Ok(Json(project))
}

/// Get a specific project
///
/// GET /v1/projects/{project_id}
#[instrument(skip(state), fields(project_id = %project_id), level = "info")]
async fn get_project<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(project_id): Path<String>,
) -> Result<Json<Project>, ApiError> {
    let project = state
        .get_project(&project_id)?
        .ok_or_else(|| ApiError::not_found("Project", &project_id))?;

    Ok(Json(project))
}

/// List all projects
///
/// GET /v1/projects?cursor={cursor}&limit={limit}
#[instrument(skip(state, query), fields(cursor = ?query.cursor, limit = query.limit), level = "info")]
async fn list_projects<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Query(query): Query<ListProjectsQuery>,
) -> Result<Json<ListProjectsResponse>, ApiError> {
    let limit = query.limit.unwrap_or(50).min(100);
    let (projects, next_cursor) = state.list_projects(query.cursor.as_deref())?;

    // Apply limit
    let projects: Vec<_> = projects.into_iter().take(limit).collect();
    let next_cursor = if projects.len() == limit {
        next_cursor
    } else {
        None
    };

    Ok(Json(ListProjectsResponse {
        projects,
        next_cursor,
    }))
}

/// Query parameters for listing projects
#[derive(Debug, Deserialize)]
struct ListProjectsQuery {
    /// Pagination cursor
    cursor: Option<String>,
    /// Number of items to return (max 100)
    limit: Option<usize>,
}

/// Query parameters for listing agents in a project
#[derive(Debug, Deserialize)]
struct ListProjectAgentsQuery {
    /// Pagination cursor
    cursor: Option<String>,
    /// Number of items to return (max 100)
    limit: Option<usize>,
}

/// Response for listing projects
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ListProjectsResponse {
    projects: Vec<Project>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

/// Update a project
///
/// PATCH /v1/projects/{project_id}
#[instrument(skip(state, request), fields(project_id = %project_id), level = "info")]
async fn update_project<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(project_id): Path<String>,
    Json(request): Json<UpdateProjectRequest>,
) -> Result<Json<Project>, ApiError> {
    let mut project = state
        .get_project(&project_id)?
        .ok_or_else(|| ApiError::not_found("Project", &project_id))?;

    // Validate name if being updated
    if let Some(ref name) = request.name {
        if name.trim().is_empty() {
            return Err(ApiError::bad_request("project name cannot be empty"));
        }
        if name.len() > PROJECT_NAME_LENGTH_MAX {
            return Err(ApiError::bad_request(format!(
                "project name too long (max {} characters)",
                PROJECT_NAME_LENGTH_MAX
            )));
        }
    }

    project.apply_update(request);
    state.update_project(project.clone())?;

    tracing::info!(
        project_id = %project.id,
        name = %project.name,
        "updated project"
    );

    Ok(Json(project))
}

/// Delete a project
///
/// DELETE /v1/projects/{project_id}
#[instrument(skip(state), fields(project_id = %project_id), level = "info")]
async fn delete_project<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(project_id): Path<String>,
) -> Result<(), ApiError> {
    // Check if project has agents
    let agents = state.list_agents_by_project(&project_id)?;
    if !agents.is_empty() {
        return Err(ApiError::bad_request(format!(
            "cannot delete project with {} agents - remove or reassign agents first",
            agents.len()
        )));
    }

    state.delete_project(&project_id)?;

    tracing::info!(project_id = %project_id, "deleted project");

    Ok(())
}

/// List agents in a project
///
/// GET /v1/projects/{project_id}/agents
#[instrument(skip(state, query), fields(project_id = %project_id), level = "info")]
async fn list_project_agents<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(project_id): Path<String>,
    Query(query): Query<ListProjectAgentsQuery>,
) -> Result<Json<ListResponse<kelpie_server::models::AgentState>>, ApiError> {
    let mut agents = state.list_agents_by_project(&project_id)?;
    agents.sort_by(|a, b| a.created_at.cmp(&b.created_at));

    let total = agents.len();
    let limit = query.limit.unwrap_or(50).min(100);

    let start_idx = if let Some(cursor_id) = query.cursor.as_deref() {
        agents
            .iter()
            .position(|a| a.id == cursor_id)
            .map(|i| i + 1)
            .unwrap_or(0)
    } else {
        0
    };

    let page: Vec<_> = agents.into_iter().skip(start_idx).take(limit + 1).collect();

    let (items, next_cursor) = if page.len() > limit {
        let items: Vec<_> = page.into_iter().take(limit).collect();
        let next_cursor = items.last().map(|a| a.id.clone());
        (items, next_cursor)
    } else {
        (page, None)
    };

    Ok(Json(ListResponse {
        items,
        total,
        cursor: next_cursor,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use tower::ServiceExt;

    /// Create test app
    async fn test_app() -> Router {
        let state = AppState::new(kelpie_core::TokioRuntime);
        api::router(state)
    }

    #[tokio::test]
    async fn test_create_project() {
        let app = test_app().await;

        let project_request = serde_json::json!({
            "name": "Test Project",
            "description": "A test project for development"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/projects")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&project_request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let project: Project = serde_json::from_slice(&body).unwrap();

        assert_eq!(project.name, "Test Project");
        assert_eq!(
            project.description,
            Some("A test project for development".to_string())
        );
    }

    #[tokio::test]
    async fn test_create_project_empty_name() {
        let app = test_app().await;

        let project_request = serde_json::json!({
            "name": ""
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/projects")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&project_request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_list_projects_empty() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/projects")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: ListProjectsResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(resp.projects.len(), 0);
        assert!(resp.next_cursor.is_none());
    }

    #[tokio::test]
    async fn test_get_project_not_found() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/projects/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_project() {
        let app = test_app().await;

        // Create project
        let project_request = serde_json::json!({
            "name": "Delete Me"
        });

        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/projects")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&project_request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let project: Project = serde_json::from_slice(&body).unwrap();

        // Delete project
        let delete_response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/v1/projects/{}", project.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(delete_response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_update_project() {
        let app = test_app().await;

        // Create project
        let project_request = serde_json::json!({
            "name": "Original Name"
        });

        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/projects")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&project_request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let project: Project = serde_json::from_slice(&body).unwrap();

        // Update project
        let update_request = serde_json::json!({
            "name": "Updated Name",
            "description": "New description"
        });

        let update_response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/v1/projects/{}", project.id))
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
        let updated_project: Project = serde_json::from_slice(&body).unwrap();

        assert_eq!(updated_project.name, "Updated Name");
        assert_eq!(
            updated_project.description,
            Some("New description".to_string())
        );
    }
}
