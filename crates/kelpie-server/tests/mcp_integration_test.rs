//! MCP Integration Tests
//!
//! Tests for MCP execution with all three transports (stdio, HTTP, SSE).
//!
//! TigerStyle: Comprehensive transport testing with real execution.

use kelpie_core::Runtime;
use kelpie_server::tools::UnifiedToolRegistry;
use kelpie_tools::McpConfig;
use serde_json::json;
use std::sync::Arc;

// =============================================================================
// Helper: Test MCP Server Configuration
// =============================================================================

/// Create a test Python MCP server config (stdio)
///
/// Requires Python with mcp package: `pip install mcp`
fn create_stdio_test_config() -> McpConfig {
    McpConfig::stdio(
        "test-stdio",
        "python3",
        vec!["-m".to_string(), "mcp".to_string(), "--stdio".to_string()],
    )
    .with_connection_timeout_ms(5000)
    .with_request_timeout_ms(10000)
}

/// Create a test HTTP MCP server config
fn create_http_test_config(url: &str) -> McpConfig {
    McpConfig::http("test-http", url)
        .with_connection_timeout_ms(5000)
        .with_request_timeout_ms(10000)
}

/// Create a test SSE MCP server config
fn create_sse_test_config(url: &str) -> McpConfig {
    McpConfig::sse("test-sse", url)
        .with_connection_timeout_ms(5000)
        .with_request_timeout_ms(10000)
}

// =============================================================================
// Stdio Transport Tests
// =============================================================================

#[tokio::test]
#[ignore] // Requires Python MCP server installed
async fn test_mcp_stdio_connect_and_discover() {
    let registry = Arc::new(UnifiedToolRegistry::new());
    let config = create_stdio_test_config();

    // Connect to MCP server
    let result = registry.connect_mcp_server("test-server", config).await;

    assert!(
        result.is_ok(),
        "Failed to connect to MCP server: {:?}",
        result
    );

    let tool_count = result.unwrap();
    assert!(
        tool_count > 0,
        "Expected at least one tool to be discovered"
    );

    // List connected servers
    let servers = registry.list_mcp_servers().await;
    assert_eq!(servers.len(), 1);
    assert!(servers.contains(&"test-server".to_string()));

    // Disconnect
    let disconnect_result = registry.disconnect_mcp_server("test-server").await;
    assert!(disconnect_result.is_ok());

    // Verify server removed
    let servers_after = registry.list_mcp_servers().await;
    assert_eq!(servers_after.len(), 0);
}

#[tokio::test]
#[ignore] // Requires Python MCP server with specific tool
async fn test_mcp_stdio_execute_tool() {
    let registry = Arc::new(UnifiedToolRegistry::new());
    let config = create_stdio_test_config();

    // Connect and discover tools
    registry
        .connect_mcp_server("test-server", config)
        .await
        .expect("Failed to connect");

    // Execute a tool (assumes "echo" tool exists)
    let result = registry
        .execute(
            "echo",
            &json!({
                "message": "Hello from MCP!"
            }),
        )
        .await;

    assert!(result.success, "Tool execution failed: {:?}", result.error);
    assert!(
        result.output.contains("Hello from MCP!"),
        "Unexpected output: {}",
        result.output
    );

    // Cleanup
    registry
        .disconnect_mcp_server("test-server")
        .await
        .expect("Failed to disconnect");
}

#[tokio::test]
#[ignore] // Requires Python MCP server
async fn test_mcp_stdio_concurrent_execution() {
    let registry = Arc::new(UnifiedToolRegistry::new());
    let config = create_stdio_test_config();

    registry
        .connect_mcp_server("test-server", config)
        .await
        .expect("Failed to connect");

    // Execute multiple tools concurrently
    let mut handles = vec![];
    for i in 0..5 {
        let registry_clone = Arc::clone(&registry);
        let handle = kelpie_core::current_runtime().spawn(async move {
            registry_clone
                .execute(
                    "echo",
                    &json!({
                        "message": format!("Message {}", i)
                    }),
                )
                .await
        });
        handles.push(handle);
    }

    // Wait for all to complete
    let results = futures::future::join_all(handles).await;

    // Verify all succeeded
    for result in results {
        let exec_result = result.expect("Task panicked");
        assert!(exec_result.success, "Concurrent execution failed");
    }

    // Cleanup
    registry
        .disconnect_mcp_server("test-server")
        .await
        .expect("Failed to disconnect");
}

// =============================================================================
// HTTP Transport Tests
// =============================================================================

#[tokio::test]
#[ignore] // Requires HTTP MCP server running
async fn test_mcp_http_connect_and_execute() {
    let registry = Arc::new(UnifiedToolRegistry::new());
    let config = create_http_test_config("http://localhost:3000");

    // Connect to HTTP MCP server
    let result = registry.connect_mcp_server("http-server", config).await;

    assert!(result.is_ok(), "Failed to connect: {:?}", result);

    // Execute a tool
    let exec_result = registry
        .execute(
            "test_tool",
            &json!({
                "param": "value"
            }),
        )
        .await;

    assert!(
        exec_result.success,
        "HTTP tool execution failed: {:?}",
        exec_result.error
    );

    // Cleanup
    registry
        .disconnect_mcp_server("http-server")
        .await
        .expect("Failed to disconnect");
}

// =============================================================================
// SSE Transport Tests
// =============================================================================

#[tokio::test]
#[ignore] // Requires SSE MCP server running
async fn test_mcp_sse_connect_and_execute() {
    let registry = Arc::new(UnifiedToolRegistry::new());
    let config = create_sse_test_config("http://localhost:3001");

    // Connect to SSE MCP server
    let result = registry.connect_mcp_server("sse-server", config).await;

    assert!(result.is_ok(), "Failed to connect: {:?}", result);

    // Execute a tool
    let exec_result = registry
        .execute(
            "test_tool",
            &json!({
                "param": "value"
            }),
        )
        .await;

    assert!(
        exec_result.success,
        "SSE tool execution failed: {:?}",
        exec_result.error
    );

    // Cleanup
    registry
        .disconnect_mcp_server("sse-server")
        .await
        .expect("Failed to disconnect");
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[tokio::test]
async fn test_mcp_execute_server_not_connected() {
    let registry = Arc::new(UnifiedToolRegistry::new());

    // Register a tool without connecting server
    registry
        .register_mcp_tool("test_tool", "Test tool", json!({}), "nonexistent")
        .await;

    // Try to execute
    let result = registry
        .execute(
            "test_tool",
            &json!({
                "param": "value"
            }),
        )
        .await;

    assert!(!result.success);
    assert!(result.error.is_some());
    let error = result.error.unwrap();
    assert!(error.contains("not connected"));
}

#[tokio::test]
async fn test_mcp_disconnect_nonexistent_server() {
    let registry = Arc::new(UnifiedToolRegistry::new());

    // Disconnect from a server that doesn't exist
    let result = registry.disconnect_mcp_server("nonexistent").await;

    // Should succeed (no-op)
    assert!(result.is_ok());
}

// =============================================================================
// Multiple Server Tests
// =============================================================================

#[tokio::test]
#[ignore] // Requires multiple MCP servers
async fn test_mcp_multiple_servers() {
    let registry = Arc::new(UnifiedToolRegistry::new());

    // Connect to two different servers
    let config1 = create_stdio_test_config();
    let config2 = create_http_test_config("http://localhost:3000");

    registry
        .connect_mcp_server("server1", config1)
        .await
        .expect("Failed to connect to server1");

    registry
        .connect_mcp_server("server2", config2)
        .await
        .expect("Failed to connect to server2");

    // List servers
    let servers = registry.list_mcp_servers().await;
    assert_eq!(servers.len(), 2);
    assert!(servers.contains(&"server1".to_string()));
    assert!(servers.contains(&"server2".to_string()));

    // Disconnect both
    registry
        .disconnect_mcp_server("server1")
        .await
        .expect("Failed to disconnect server1");
    registry
        .disconnect_mcp_server("server2")
        .await
        .expect("Failed to disconnect server2");

    // Verify all removed
    let servers_after = registry.list_mcp_servers().await;
    assert_eq!(servers_after.len(), 0);
}
