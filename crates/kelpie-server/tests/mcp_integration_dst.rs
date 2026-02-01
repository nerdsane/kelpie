//! DST tests for MCP tool integration
//!
//! TigerStyle: DST-first development - write tests before implementation.
//!
//! These tests verify that MCP tools work correctly under fault conditions.
#![cfg(feature = "dst")]

use kelpie_core::error::Error as CoreError;
use kelpie_dst::fault::{FaultConfig, FaultType};
use kelpie_dst::simulation::{SimConfig, Simulation};
use kelpie_tools::{
    ConnectionState, McpToolDefinition, SimMcpClient, SimMcpEnvironment, SimMcpServerConfig,
};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// =============================================================================
// Test Helpers
// =============================================================================

/// Convert any error to CoreError for test compatibility
fn to_core_error<E: std::fmt::Display>(e: E) -> CoreError {
    CoreError::Internal {
        message: e.to_string(),
    }
}

/// Create a standard test MCP server configuration
fn create_test_server(name: &str) -> SimMcpServerConfig {
    SimMcpServerConfig::new(name)
        .with_tool(McpToolDefinition {
            name: format!("{}_echo", name),
            description: "Echo back input".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                },
                "required": ["message"]
            }),
        })
        .with_tool(McpToolDefinition {
            name: format!("{}_process", name),
            description: "Process data".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "data": { "type": "string" }
                },
                "required": ["data"]
            }),
        })
}

// =============================================================================
// Basic Functionality Tests
// =============================================================================

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_tool_discovery_basic() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|env| async move {
            let mut client = SimMcpClient::new(env.faults.clone(), env.fork_rng_raw());

            // Register a server with tools
            client.register_server(create_test_server("server1"));

            // Connect to server
            client.connect("server1").await.map_err(to_core_error)?;
            assert!(client.is_connected("server1").await);

            // Discover tools
            let tools = client
                .discover_tools("server1")
                .await
                .map_err(to_core_error)?;
            assert_eq!(tools.len(), 2);
            assert!(tools.iter().any(|t| t.name == "server1_echo"));
            assert!(tools.iter().any(|t| t.name == "server1_process"));

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_tool_execution_basic() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|env| async move {
            let mut client = SimMcpClient::new(env.faults.clone(), env.fork_rng_raw());
            client.register_server(create_test_server("server1"));
            client.connect("server1").await.map_err(to_core_error)?;

            // Execute tool
            let result = client
                .execute_tool("server1_echo", json!({"message": "hello world"}))
                .await
                .map_err(to_core_error)?;

            assert!(result.get("success").unwrap().as_bool().unwrap());
            assert!(result.get("simulated").unwrap().as_bool().unwrap());

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_multiple_servers() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|env| async move {
            let mut client = SimMcpClient::new(env.faults.clone(), env.fork_rng_raw());

            // Register multiple servers
            client.register_server(create_test_server("server1"));
            client.register_server(create_test_server("server2"));
            client.register_server(create_test_server("server3"));

            // Connect to all
            client.connect("server1").await.map_err(to_core_error)?;
            client.connect("server2").await.map_err(to_core_error)?;
            client.connect("server3").await.map_err(to_core_error)?;

            // Discover all tools
            let all_tools = client.discover_all_tools().await.map_err(to_core_error)?;
            assert_eq!(all_tools.len(), 6); // 2 tools per server * 3 servers

            // Execute tool from each server
            client
                .execute_tool("server1_echo", json!({"message": "1"}))
                .await
                .map_err(to_core_error)?;
            client
                .execute_tool("server2_echo", json!({"message": "2"}))
                .await
                .map_err(to_core_error)?;
            client
                .execute_tool("server3_echo", json!({"message": "3"}))
                .await
                .map_err(to_core_error)?;

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
async fn test_dst_mcp_server_crash_during_connect() {
    let config = SimConfig::new(12345);
    println!("DST seed: {}", config.seed);

    let fault_observed = Arc::new(AtomicUsize::new(0));
    let fo = fault_observed.clone();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::McpServerCrash, 1.0)) // 100% crash rate
        .run_async(move |env| {
            let fo = fo.clone();
            async move {
                let mut client = SimMcpClient::new(env.faults.clone(), env.fork_rng_raw());
                client.register_server(create_test_server("server1"));

                // Connect should fail due to server crash
                match client.connect("server1").await {
                    Ok(()) => {
                        println!("WARNING: Connect succeeded despite 100% crash rate");
                    }
                    Err(e) => {
                        println!("Server crash correctly simulated: {}", e);
                        fo.fetch_add(1, Ordering::SeqCst);
                    }
                }

                // Connection state should be failed
                let state = client.connection_state("server1").await;
                assert_eq!(state, ConnectionState::Failed);

                Ok::<(), CoreError>(())
            }
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
    let observed = fault_observed.load(Ordering::SeqCst);
    println!("Faults observed: {}", observed);
    assert!(observed > 0, "Expected at least one fault to be observed");
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_tool_fail_during_execution() {
    let config = SimConfig::new(54321);
    println!("DST seed: {}", config.seed);

    let fault_observed = Arc::new(AtomicUsize::new(0));
    let fo = fault_observed.clone();

    let result = Simulation::new(config)
        .with_fault(
            FaultConfig::new(FaultType::McpToolFail, 1.0).with_filter("mcp_execute"), // Only during execution
        )
        .run_async(move |env| {
            let fo = fo.clone();
            async move {
                let mut client = SimMcpClient::new(env.faults.clone(), env.fork_rng_raw());
                client.register_server(create_test_server("server1"));

                // Connect should succeed (no fault for connect)
                client.connect("server1").await.map_err(to_core_error)?;
                assert!(client.is_connected("server1").await);

                // Tool execution should fail
                match client
                    .execute_tool("server1_echo", json!({"message": "test"}))
                    .await
                {
                    Ok(_) => {
                        println!("WARNING: Execution succeeded despite 100% fail rate");
                    }
                    Err(e) => {
                        println!("Tool failure correctly simulated: {}", e);
                        fo.fetch_add(1, Ordering::SeqCst);
                    }
                }

                Ok::<(), CoreError>(())
            }
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
    let observed = fault_observed.load(Ordering::SeqCst);
    println!("Faults observed: {}", observed);
    assert!(observed > 0, "Expected at least one fault to be observed");
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_tool_timeout() {
    let config = SimConfig::new(11111);
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::McpToolTimeout, 1.0).with_filter("mcp_execute"))
        .run_async(|env| async move {
            let mut client = SimMcpClient::new(env.faults.clone(), env.fork_rng_raw());
            client.register_server(create_test_server("server1"));
            client.connect("server1").await.map_err(to_core_error)?;

            // Execution should timeout
            let result = client
                .execute_tool("server1_echo", json!({"message": "test"}))
                .await;
            assert!(result.is_err());

            let err = result.unwrap_err();
            let err_str = format!("{}", err);
            assert!(
                err_str.contains("timeout")
                    || err_str.contains("Timeout")
                    || err_str.contains("timed out"),
                "Expected timeout error, got: {}",
                err_str
            );

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_network_partition() {
    let config = SimConfig::new(22222);
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::NetworkPartition, 1.0).with_filter("mcp_connect"))
        .run_async(|env| async move {
            let mut client = SimMcpClient::new(env.faults.clone(), env.fork_rng_raw());
            client.register_server(create_test_server("server1"));

            // Connection should fail due to partition
            let result = client.connect("server1").await;
            assert!(result.is_err());

            let err = result.unwrap_err();
            let err_str = format!("{}", err);
            assert!(
                err_str.contains("partition") || err_str.contains("network"),
                "Expected network partition error, got: {}",
                err_str
            );

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_packet_loss_during_discovery() {
    let config = SimConfig::new(33333);
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::NetworkPacketLoss, 1.0).with_filter("mcp_discover"))
        .run_async(|env| async move {
            let mut client = SimMcpClient::new(env.faults.clone(), env.fork_rng_raw());
            client.register_server(create_test_server("server1"));

            // Connection should succeed
            client.connect("server1").await.map_err(to_core_error)?;

            // Discovery should fail due to packet loss
            let result = client.discover_tools("server1").await;
            assert!(result.is_err());

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Resilience Tests
// =============================================================================

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_graceful_degradation() {
    let config = SimConfig::new(44444);
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|env| async move {
            let mut client = SimMcpClient::new(env.faults.clone(), env.fork_rng_raw());

            // Register multiple servers, one is offline
            client.register_server(create_test_server("server1"));
            client.register_server(create_test_server("server2").online(false));
            client.register_server(create_test_server("server3"));

            // Connect to all - server2 will fail
            let _ = client.connect("server1").await;
            let server2_result = client.connect("server2").await;
            let _ = client.connect("server3").await;

            // Server2 should have failed
            assert!(server2_result.is_err());

            // But we should still be able to discover tools from working servers
            let all_tools = client.discover_all_tools().await.map_err(to_core_error)?;
            assert_eq!(all_tools.len(), 4); // 2 tools from server1 + 2 from server3

            // And execute tools on working servers
            client
                .execute_tool("server1_echo", json!({"message": "works"}))
                .await
                .map_err(to_core_error)?;
            client
                .execute_tool("server3_echo", json!({"message": "works"}))
                .await
                .map_err(to_core_error)?;

            // Tool from offline server should fail
            let result = client
                .execute_tool("server2_echo", json!({"message": "fail"}))
                .await;
            assert!(result.is_err());

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_mixed_tools_with_faults() {
    let config = SimConfig::new(55555);
    println!("DST seed: {}", config.seed);

    let success_count = Arc::new(AtomicUsize::new(0));
    let failure_count = Arc::new(AtomicUsize::new(0));
    let sc = success_count.clone();
    let fc = failure_count.clone();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::McpToolFail, 0.3).with_filter("mcp_execute")) // 30% fail rate
        .run_async(move |env| {
            let sc = sc.clone();
            let fc = fc.clone();
            async move {
                let mut client = SimMcpClient::new(env.faults.clone(), env.fork_rng_raw());

                client.register_server(create_test_server("server1"));
                client.register_server(create_test_server("server2"));

                client.connect("server1").await.map_err(to_core_error)?;
                client.connect("server2").await.map_err(to_core_error)?;

                // Execute many tools - some will fail
                for i in 0..20 {
                    let tool = if i % 2 == 0 {
                        "server1_echo"
                    } else {
                        "server2_echo"
                    };

                    match client.execute_tool(tool, json!({"message": i})).await {
                        Ok(_) => {
                            sc.fetch_add(1, Ordering::SeqCst);
                        }
                        Err(_) => {
                            fc.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                }

                Ok::<(), CoreError>(())
            }
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());

    let successes = success_count.load(Ordering::SeqCst);
    let failures = failure_count.load(Ordering::SeqCst);

    println!("Successes: {}, Failures: {}", successes, failures);

    // With 30% failure rate, we expect roughly 14 successes and 6 failures
    // But due to determinism, the exact numbers depend on the seed
    assert!(successes > 0, "Expected some successes");
    assert!(failures > 0, "Expected some failures with 30% fault rate");
    assert_eq!(successes + failures, 20, "Should have 20 total attempts");
}

// =============================================================================
// Determinism Tests
// =============================================================================

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_determinism() {
    let seed = 99999u64;

    // Run the same test twice with the same seed
    let mut results = Vec::new();

    for run in 0..2 {
        println!("Run {} with seed {}", run + 1, seed);

        let config = SimConfig::new(seed);
        let execution_log = Arc::new(std::sync::RwLock::new(Vec::<String>::new()));
        let log = execution_log.clone();

        let result = Simulation::new(config)
            .with_fault(FaultConfig::new(FaultType::McpToolFail, 0.5).with_filter("mcp_execute"))
            .run_async(move |env| {
                let log = log.clone();
                async move {
                    let mut client = SimMcpClient::new(env.faults.clone(), env.fork_rng_raw());
                    client.register_server(create_test_server("server1"));
                    client.connect("server1").await.map_err(to_core_error)?;

                    for i in 0..10 {
                        let result = client
                            .execute_tool("server1_echo", json!({"message": i}))
                            .await;
                        let outcome = if result.is_ok() { "OK" } else { "FAIL" };
                        log.write().unwrap().push(format!("{}: {}", i, outcome));
                    }

                    Ok::<(), CoreError>(())
                }
            })
            .await;

        assert!(result.is_ok());
        results.push(execution_log.read().unwrap().clone());
    }

    // Both runs should produce identical results
    println!("Run 1: {:?}", results[0]);
    println!("Run 2: {:?}", results[1]);
    assert_eq!(results[0], results[1], "Results should be deterministic");
}

// =============================================================================
// Environment Builder Test
// =============================================================================

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_mcp_environment_builder() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|env| async move {
            // Use the environment builder pattern
            let client = SimMcpEnvironment::new()
                .with_server(create_test_server("alpha"))
                .with_server(create_test_server("beta"))
                .build(env.faults.clone(), env.fork_rng_raw());

            // Verify servers were registered
            let servers = client.servers();
            assert_eq!(servers.len(), 2);
            assert!(servers.contains(&"alpha"));
            assert!(servers.contains(&"beta"));

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}
