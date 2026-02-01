//! Groups API endpoints (Letta compatibility alias)
//!
//! This module provides Letta-compatible `/groups` endpoints that map to the
//! agent_groups functionality. Letta SDK expects /v1/groups while Kelpie uses
//! /v1/agent-groups internally.

use super::agent_groups::*;
use axum::Router;
use kelpie_core::Runtime;
use kelpie_server::state::AppState;

/// Create router for groups endpoints (Letta compatibility)
pub fn router<R: Runtime + 'static>() -> Router<AppState<R>> {
    Router::new()
        .route(
            "/groups",
            axum::routing::get(list_groups).post(create_group),
        )
        .route(
            "/groups/:group_id",
            axum::routing::get(get_group)
                .patch(update_group)
                .delete(delete_group),
        )
}
