//! Identity API endpoints
//!
//! TigerStyle: Letta-compatible identity management.

use crate::api::ApiError;
use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use kelpie_core::Runtime;
use kelpie_server::models::{CreateIdentityRequest, Identity, UpdateIdentityRequest};
use kelpie_server::state::AppState;
use serde::{Deserialize, Serialize};
use tracing::instrument;

/// Query parameters for listing identities
#[derive(Debug, Deserialize)]
pub struct ListIdentitiesQuery {
    pub name: Option<String>,
    /// Cursor for pagination (Kelpie's native parameter)
    pub cursor: Option<String>,
    /// Cursor for pagination (Letta SDK compatibility - alias for cursor)
    pub after: Option<String>,
    pub limit: Option<usize>,
}

/// Response for listing identities
#[derive(Debug, Serialize)]
pub struct ListIdentitiesResponse {
    pub identities: Vec<Identity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Create identity routes
pub fn router<R: Runtime + 'static>() -> Router<AppState<R>> {
    Router::new()
        .route("/identities", get(list_identities).post(create_identity))
        .route(
            "/identities/:identity_id",
            get(get_identity)
                .patch(update_identity)
                .delete(delete_identity),
        )
}

/// Create a new identity
#[instrument(skip(state, request), level = "info")]
pub async fn create_identity<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Json(request): Json<CreateIdentityRequest>,
) -> Result<Json<Identity>, ApiError> {
    // Validate name
    if request.name.trim().is_empty() {
        return Err(ApiError::bad_request("identity name cannot be empty"));
    }

    // Validate agent IDs if provided
    for agent_id in &request.agent_ids {
        let exists = state
            .get_agent_async(agent_id)
            .await?
            .ok_or_else(|| ApiError::not_found("Agent", agent_id))?;
        let _ = exists;
    }

    // Note: block_ids are references only, not validated at creation time

    let identity = Identity::from_request(request);
    state.add_identity(identity.clone()).await?;

    tracing::info!(identity_id = %identity.id, "created identity");
    Ok(Json(identity))
}

/// List identities
#[instrument(skip(state, query), level = "info")]
pub async fn list_identities<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Query(query): Query<ListIdentitiesQuery>,
) -> Result<Json<ListIdentitiesResponse>, ApiError> {
    let (mut identities, _) = state.list_identities(None)?;

    if let Some(name_filter) = query.name {
        identities.retain(|i| i.name.contains(&name_filter));
    }

    let limit = query.limit.unwrap_or(50).min(100);
    let cursor = query.cursor.as_deref().or(query.after.as_deref());
    let start_idx = if let Some(cursor) = cursor {
        identities
            .iter()
            .position(|i| i.id == cursor)
            .map(|idx| idx + 1)
            .unwrap_or(0)
    } else {
        0
    };

    let page: Vec<_> = identities
        .into_iter()
        .skip(start_idx)
        .take(limit + 1)
        .collect();
    let (items, next_cursor) = if page.len() > limit {
        let items: Vec<_> = page.into_iter().take(limit).collect();
        let next_cursor = items.last().map(|i| i.id.clone());
        (items, next_cursor)
    } else {
        (page, None)
    };

    Ok(Json(ListIdentitiesResponse {
        identities: items,
        next_cursor,
    }))
}

/// Get identity details
#[instrument(skip(state), fields(identity_id = %identity_id), level = "info")]
pub async fn get_identity<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(identity_id): Path<String>,
) -> Result<Json<Identity>, ApiError> {
    let identity = state
        .get_identity(&identity_id)?
        .ok_or_else(|| ApiError::not_found("Identity", &identity_id))?;
    Ok(Json(identity))
}

/// Update identity
#[instrument(skip(state, request), fields(identity_id = %identity_id), level = "info")]
pub async fn update_identity<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(identity_id): Path<String>,
    Json(request): Json<UpdateIdentityRequest>,
) -> Result<Json<Identity>, ApiError> {
    let mut identity = state
        .get_identity(&identity_id)?
        .ok_or_else(|| ApiError::not_found("Identity", &identity_id))?;

    // Validate name if being updated
    if let Some(ref name) = request.name {
        if name.trim().is_empty() {
            return Err(ApiError::bad_request("identity name cannot be empty"));
        }
    }

    // Validate agent IDs being added
    for agent_id in &request.add_agent_ids {
        let _ = state
            .get_agent_async(agent_id)
            .await?
            .ok_or_else(|| ApiError::not_found("Agent", agent_id))?;
    }

    // Note: block_ids are references only, not validated at update time

    identity.apply_update(request);
    state.update_identity(identity.clone()).await?;

    Ok(Json(identity))
}

/// Delete identity
#[instrument(skip(state), fields(identity_id = %identity_id), level = "info")]
pub async fn delete_identity<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(identity_id): Path<String>,
) -> Result<(), ApiError> {
    state.delete_identity(&identity_id).await?;
    tracing::info!(identity_id = %identity_id, "deleted identity");
    Ok(())
}
