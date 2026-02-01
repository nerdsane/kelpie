//! Standalone blocks API endpoints
//!
//! TigerStyle: Letta-compatible standalone block CRUD operations.
//! These blocks can be created independently and later attached to agents.

use crate::api::ApiError;
use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use kelpie_core::Runtime;
use kelpie_server::models::{Block, CreateBlockRequest, ListResponse, UpdateBlockRequest};
use kelpie_server::state::AppState;
use serde::Deserialize;
use tracing::instrument;

/// Query parameters for listing blocks
#[derive(Debug, Deserialize)]
pub struct ListBlocksQuery {
    /// Maximum number of blocks to return
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Cursor for pagination (Kelpie's native parameter)
    pub cursor: Option<String>,
    /// Cursor for pagination (Letta SDK compatibility - alias for cursor)
    pub after: Option<String>,
    /// Filter by label
    pub label: Option<String>,
}

fn default_limit() -> usize {
    50
}

/// Maximum limit for list operations
const LIST_LIMIT_MAX: usize = 100;

/// Create standalone blocks routes
pub fn router<R: Runtime + 'static>() -> Router<AppState<R>> {
    Router::new()
        .route("/", get(list_blocks).post(create_block))
        .route(
            "/:block_id",
            get(get_block).patch(update_block).delete(delete_block),
        )
}

/// Create a new standalone block
///
/// POST /v1/blocks
#[instrument(skip(state, request), fields(label = %request.label), level = "info")]
async fn create_block<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Json(request): Json<CreateBlockRequest>,
) -> Result<Json<Block>, ApiError> {
    // Validate request
    if request.label.is_empty() {
        return Err(ApiError::bad_request("block label cannot be empty"));
    }

    if request.label.len() > 256 {
        return Err(ApiError::bad_request(
            "block label too long (max 256 characters)",
        ));
    }

    // Check size limit if specified
    if let Some(limit) = request.limit {
        if request.value.len() > limit {
            return Err(ApiError::unprocessable_entity(format!(
                "block value exceeds limit ({} > {})",
                request.value.len(),
                limit
            )));
        }
    }

    let block = Block::from_request(request);
    let created = state.create_standalone_block(block)?;

    tracing::info!(block_id = %created.id, label = %created.label, "created standalone block");
    Ok(Json(created))
}

/// Get a standalone block by ID
///
/// GET /v1/blocks/{block_id}
#[instrument(skip(state), fields(block_id = %block_id), level = "info")]
async fn get_block<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(block_id): Path<String>,
) -> Result<Json<Block>, ApiError> {
    let block = state
        .get_standalone_block(&block_id)?
        .ok_or_else(|| ApiError::not_found("Block", &block_id))?;

    Ok(Json(block))
}

/// List all standalone blocks with pagination
///
/// GET /v1/blocks
#[instrument(skip(state, query), fields(limit = query.limit, label = ?query.label), level = "info")]
async fn list_blocks<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Query(query): Query<ListBlocksQuery>,
) -> Result<Json<ListResponse<Block>>, ApiError> {
    let limit = query.limit.min(LIST_LIMIT_MAX);
    let cursor_param = query.cursor.as_deref().or(query.after.as_deref());
    let (items, cursor) =
        state.list_standalone_blocks(limit, cursor_param, query.label.as_deref())?;
    let total = state.standalone_block_count()?;

    Ok(Json(ListResponse {
        items,
        total,
        cursor,
    }))
}

/// Update a standalone block
///
/// PATCH /v1/blocks/{block_id}
#[instrument(skip(state, request), fields(block_id = %block_id), level = "info")]
async fn update_block<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(block_id): Path<String>,
    Json(request): Json<UpdateBlockRequest>,
) -> Result<Json<Block>, ApiError> {
    // Get current block to check limit
    let current = state
        .get_standalone_block(&block_id)?
        .ok_or_else(|| ApiError::not_found("Block", &block_id))?;

    // Check new value against limit (use new limit if provided, otherwise existing)
    if let Some(ref new_value) = request.value {
        let limit = request.limit.or(current.limit);
        if let Some(l) = limit {
            if new_value.len() > l {
                return Err(ApiError::unprocessable_entity(format!(
                    "block value exceeds limit ({} > {})",
                    new_value.len(),
                    l
                )));
            }
        }
    }

    let updated = state.update_standalone_block(&block_id, |block| {
        block.apply_update(request);
    })?;

    tracing::info!(block_id = %updated.id, "updated standalone block");
    Ok(Json(updated))
}

/// Delete a standalone block
///
/// DELETE /v1/blocks/{block_id}
#[instrument(skip(state), fields(block_id = %block_id), level = "info")]
async fn delete_block<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(block_id): Path<String>,
) -> Result<(), ApiError> {
    state.delete_standalone_block(&block_id)?;
    tracing::info!(block_id = %block_id, "deleted standalone block");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn test_app() -> Router {
        api::router(AppState::new(kelpie_core::TokioRuntime))
    }

    #[tokio::test]
    async fn test_create_standalone_block() {
        let app = test_app().await;

        let body = serde_json::json!({
            "label": "persona",
            "value": "I am a helpful assistant"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/blocks")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let block: Block = serde_json::from_slice(&body).unwrap();
        assert_eq!(block.label, "persona");
        assert!(!block.id.is_empty());
    }

    #[tokio::test]
    async fn test_create_block_empty_label() {
        let app = test_app().await;

        let body = serde_json::json!({
            "label": "",
            "value": "test"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/blocks")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_block_not_found() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/blocks/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_list_blocks() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/blocks")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let list: ListResponse<Block> = serde_json::from_slice(&body).unwrap();
        assert_eq!(list.total, 0);
    }
}
