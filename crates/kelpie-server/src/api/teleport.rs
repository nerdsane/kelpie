//! Teleport API endpoints for agent migration
//!
//! TigerStyle: REST API for teleport package management.
//!
//! This module provides endpoints for:
//! - Listing teleport packages
//! - Getting package details
//! - Deleting packages
//!
//! Note: Full teleport_out/teleport_in operations require sandbox integration
//! which will be added in a future phase when sandbox management is added to the server.

use axum::{
    extract::{Path, State},
    routing::{delete, get},
    Json, Router,
};
use kelpie_core::Runtime;
use kelpie_server::state::AppState;
use serde::Serialize;

use super::ApiError;

/// Create the teleport router
pub fn router<R: Runtime + 'static>() -> Router<AppState<R>> {
    Router::new()
        // Package management
        .route("/packages", get(list_packages))
        .route("/packages/:package_id", get(get_package))
        .route("/packages/:package_id", delete(delete_package))
        // Info
        .route("/info", get(teleport_info))
}

/// Teleport system info response
#[derive(Debug, Serialize)]
pub struct TeleportInfoResponse {
    /// Host architecture
    pub host_arch: String,
    /// Supported teleport features
    pub features: Vec<String>,
    /// Whether teleport is enabled
    pub enabled: bool,
}

/// List teleport packages response
#[derive(Debug, Serialize)]
pub struct ListPackagesResponse {
    /// Package IDs
    pub packages: Vec<String>,
    /// Total count
    pub count: usize,
}

/// Package details response
#[derive(Debug, Serialize)]
pub struct PackageResponse {
    /// Package ID
    pub id: String,
    /// Agent ID
    pub agent_id: String,
    /// Source architecture
    pub source_arch: String,
    /// Snapshot kind
    pub kind: String,
    /// Creation timestamp (ms)
    pub created_at_ms: u64,
    /// Package size in bytes
    pub size_bytes: u64,
}

/// Get teleport system info
///
/// GET /v1/teleport/info
async fn teleport_info() -> Json<TeleportInfoResponse> {
    use kelpie_server::storage::Architecture;

    Json(TeleportInfoResponse {
        host_arch: Architecture::current().to_string(),
        features: vec![
            "packages".to_string(),
            "suspend".to_string(),
            "teleport".to_string(),
            "checkpoint".to_string(),
        ],
        enabled: true,
    })
}

/// List all teleport packages
///
/// GET /v1/teleport/packages
async fn list_packages<R: Runtime + 'static>(
    State(_state): State<AppState<R>>,
) -> Json<ListPackagesResponse> {
    // TODO: When teleport storage is added to AppState, query actual packages
    // For now, return empty list
    Json(ListPackagesResponse {
        packages: vec![],
        count: 0,
    })
}

/// Get package details
///
/// GET /v1/teleport/packages/:package_id
async fn get_package<R: Runtime + 'static>(
    State(_state): State<AppState<R>>,
    Path(package_id): Path<String>,
) -> Result<Json<PackageResponse>, ApiError> {
    // TODO: When teleport storage is added to AppState, query actual package
    // For now, return not found
    Err(ApiError::not_found("teleport_package", &package_id))
}

/// Delete a teleport package
///
/// DELETE /v1/teleport/packages/:package_id
async fn delete_package<R: Runtime + 'static>(
    State(_state): State<AppState<R>>,
    Path(package_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // TODO: When teleport storage is added to AppState, delete actual package
    // For now, return not found
    Err(ApiError::not_found("teleport_package", &package_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn test_app() -> Router {
        let state = AppState::new(kelpie_core::TokioRuntime);
        Router::new()
            .nest("/v1/teleport", router())
            .with_state(state)
    }

    #[tokio::test]
    async fn test_teleport_info() {
        let app = test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/teleport/info")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_list_packages_empty() {
        let app = test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/teleport/packages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_get_package_not_found() {
        let app = test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/teleport/packages/non-existent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
