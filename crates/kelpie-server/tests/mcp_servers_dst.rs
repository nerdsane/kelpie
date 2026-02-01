//! DST tests for MCP servers CRUD operations
//!
//! TigerStyle: DST-first development - test CRUD operations under faults.
//!
//! These tests verify that MCP server management (create, read, update, delete, list)
//! works correctly under fault conditions. MCP servers are stored in-memory in AppState,
//! so these tests focus on storage write/read faults and concurrent access patterns.

#![cfg(feature = "dst")]

use kelpie_core::error::Error as CoreError;
use kelpie_dst::fault::{FaultConfig, FaultType};
use kelpie_dst::simulation::{SimConfig, Simulation};
use kelpie_server::models::MCPServerConfig;
use kelpie_server::state::AppState;
use serde_json::json;

// =============================================================================
// Test Helpers
// =============================================================================

/// Convert any error to CoreError for test compatibility
fn to_core_error<E: std::fmt::Display>(e: E) -> CoreError {
    CoreError::Internal {
        message: e.to_string(),
    }
}

/// Create a test stdio MCP server config
fn create_stdio_config(name: &str) -> MCPServerConfig {
    MCPServerConfig::Stdio {
        command: "python3".to_string(),
        args: vec!["mock_server.py".to_string()],
        env: Some(json!({"TEST_SERVER": name}).as_object().unwrap().clone()),
    }
}

/// Create a test SSE MCP server config
fn create_sse_config(url: &str) -> MCPServerConfig {
    MCPServerConfig::Sse {
        server_url: url.to_string(),
        auth_header: Some("Authorization".to_string()),
        auth_token: Some("test-token".to_string()),
        custom_headers: None,
    }
}

// =============================================================================
// Basic CRUD Tests
// =============================================================================

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_server_create_basic() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let state = AppState::new(kelpie_core::current_runtime());

            // Create MCP server
            let server = state
                .create_mcp_server("test-server", create_stdio_config("test"))
                .await
                .map_err(to_core_error)?;

            assert_eq!(server.server_name, "test-server");
            assert!(server.id.starts_with("mcp_server-"));

            // Verify we can retrieve it
            let retrieved =
                state
                    .get_mcp_server(&server.id)
                    .await
                    .ok_or_else(|| CoreError::Internal {
                        message: "server not found".to_string(),
                    })?;

            assert_eq!(retrieved.id, server.id);
            assert_eq!(retrieved.server_name, "test-server");

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_server_list_empty() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let state = AppState::new(kelpie_core::current_runtime());

            // List should be empty
            let servers = state.list_mcp_servers().await;
            assert_eq!(servers.len(), 0);

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_server_list_multiple() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let state = AppState::new(kelpie_core::current_runtime());

            // Create multiple servers
            let server1 = state
                .create_mcp_server("server1", create_stdio_config("s1"))
                .await
                .map_err(to_core_error)?;

            let server2 = state
                .create_mcp_server("server2", create_sse_config("http://localhost:8080"))
                .await
                .map_err(to_core_error)?;

            // List all servers
            let servers = state.list_mcp_servers().await;
            assert_eq!(servers.len(), 2);

            let ids: Vec<_> = servers.iter().map(|s| s.id.as_str()).collect();
            assert!(ids.contains(&server1.id.as_str()));
            assert!(ids.contains(&server2.id.as_str()));

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_server_update() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let state = AppState::new(kelpie_core::current_runtime());

            // Create server
            let server = state
                .create_mcp_server("original-name", create_stdio_config("test"))
                .await
                .map_err(to_core_error)?;

            // Update name only
            let updated = state
                .update_mcp_server(&server.id, Some("updated-name".to_string()), None)
                .await
                .map_err(to_core_error)?;

            assert_eq!(updated.server_name, "updated-name");
            assert_eq!(updated.id, server.id);

            // Update config only
            let new_config = create_sse_config("http://localhost:9090");
            let updated2 = state
                .update_mcp_server(&server.id, None, Some(new_config))
                .await
                .map_err(to_core_error)?;

            assert_eq!(updated2.server_name, "updated-name"); // Name unchanged
            assert!(matches!(updated2.config, MCPServerConfig::Sse { .. }));

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_server_delete() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let state = AppState::new(kelpie_core::current_runtime());

            // Create server
            let server = state
                .create_mcp_server("test-server", create_stdio_config("test"))
                .await
                .map_err(to_core_error)?;

            // Verify it exists
            assert!(state.get_mcp_server(&server.id).await.is_some());

            // Delete it
            state
                .delete_mcp_server(&server.id)
                .await
                .map_err(to_core_error)?;

            // Verify it's gone
            assert!(state.get_mcp_server(&server.id).await.is_none());

            // List should be empty
            let servers = state.list_mcp_servers().await;
            assert_eq!(servers.len(), 0);

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Fault Injection Tests
// =============================================================================

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_server_create_with_storage_faults() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.1))
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.05))
        .run_async(|_env| async move {
            let state = AppState::new(kelpie_core::current_runtime());

            // Try to create servers - some may fail due to storage faults
            let mut created_count = 0;
            for i in 0..10 {
                match state
                    .create_mcp_server(
                        &format!("server-{}", i),
                        create_stdio_config(&format!("s{}", i)),
                    )
                    .await
                {
                    Ok(_) => created_count += 1,
                    Err(_) => {
                        // Expected due to storage faults
                    }
                }
            }

            // Should have created at least some servers
            assert!(created_count > 0, "Should have created some servers");

            // List should show only successfully created servers
            let servers = state.list_mcp_servers().await;
            assert_eq!(servers.len(), created_count);

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_server_update_with_faults() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.15))
        .run_async(|_env| async move {
            let state = AppState::new(kelpie_core::current_runtime());

            // Create server
            let server = state
                .create_mcp_server("original", create_stdio_config("test"))
                .await
                .map_err(to_core_error)?;

            // Try multiple updates - some may fail
            let mut update_succeeded = false;
            for i in 0..10 {
                if let Ok(updated) = state
                    .update_mcp_server(&server.id, Some(format!("updated-{}", i)), None)
                    .await
                {
                    assert_eq!(updated.id, server.id);
                    update_succeeded = true;
                }
            }

            // Should have at least one successful update
            assert!(update_succeeded, "At least one update should succeed");

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_server_delete_idempotent() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let state = AppState::new(kelpie_core::current_runtime());

            // Create server
            let server = state
                .create_mcp_server("test", create_stdio_config("test"))
                .await
                .map_err(to_core_error)?;

            // Delete once
            state
                .delete_mcp_server(&server.id)
                .await
                .map_err(to_core_error)?;

            // Delete again - should fail with NotFound
            let result = state.delete_mcp_server(&server.id).await;
            assert!(result.is_err(), "Second delete should fail");

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_server_concurrent_creates() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.1))
        .run_async(|_env| async move {
            use kelpie_core::{current_runtime, Runtime};
            let runtime = current_runtime();
            let state = AppState::new(runtime.clone());

            // Create multiple servers concurrently
            let mut handles = vec![];
            for i in 0..5 {
                let state_clone = state.clone();
                let handle = runtime.spawn(async move {
                    state_clone
                        .create_mcp_server(
                            &format!("concurrent-{}", i),
                            create_stdio_config(&format!("c{}", i)),
                        )
                        .await
                        .ok()
                });
                handles.push(handle);
            }

            // Wait for all creates
            let mut success_count = 0;
            for handle in handles {
                if let Ok(Some(_)) = handle.await {
                    success_count += 1;
                }
            }

            // Should have created at least some
            assert!(success_count > 0, "At least one create should succeed");

            // List should be consistent
            let servers = state.list_mcp_servers().await;
            assert_eq!(servers.len(), success_count);

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Edge Cases
// =============================================================================

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_server_update_nonexistent() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let state = AppState::new(kelpie_core::current_runtime());

            // Try to update non-existent server
            let result = state
                .update_mcp_server("nonexistent-id", Some("new-name".to_string()), None)
                .await;

            assert!(
                result.is_err(),
                "Update should fail for non-existent server"
            );

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_server_get_nonexistent() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let state = AppState::new(kelpie_core::current_runtime());

            // Get non-existent server should return None
            let server = state.get_mcp_server("nonexistent-id").await;
            assert!(server.is_none());

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}
