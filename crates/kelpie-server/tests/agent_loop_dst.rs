//! DST tests for Agent Loop with Tool Registry Integration
//!
//! TigerStyle: DST-first - these tests exercise the FULL agent loop path:
//!   User message → Agent loop → LLM response → Tool call → Registry.execute() → Result
//!
//! Unlike mcp_integration_dst.rs which tests SimMcpClient in isolation,
//! these tests verify the complete integration works under fault conditions.
#![cfg(feature = "dst")]

use kelpie_core::error::Error as CoreError;
use kelpie_dst::fault::{FaultConfig, FaultInjector, FaultType};
use kelpie_dst::simulation::{SimConfig, Simulation};
use kelpie_server::tools::{BuiltinToolHandler, ToolSource, UnifiedToolRegistry};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

#[cfg(feature = "dst")]
use kelpie_tools::{McpToolDefinition, SimMcpClient, SimMcpServerConfig};

// =============================================================================
// Test Helpers
// =============================================================================

#[allow(dead_code)]
fn to_core_error<E: std::fmt::Display>(e: E) -> CoreError {
    CoreError::Internal {
        message: e.to_string(),
    }
}

/// Create a registry with a simple builtin tool for testing
async fn create_registry_with_builtin(
    fault_injector: Option<Arc<FaultInjector>>,
) -> UnifiedToolRegistry {
    let registry = UnifiedToolRegistry::new();

    // Register a builtin echo tool that can be faulted
    let fi = fault_injector.clone();
    let handler: BuiltinToolHandler = Arc::new(move |input: &Value| {
        let input = input.clone();
        let fi = fi.clone();
        Box::pin(async move {
            // Check if we should inject a fault
            if let Some(ref injector) = fi {
                if let Some(fault_type) = injector.should_inject("builtin_execute") {
                    return format!(
                        "Error: simulated fault: builtin tool execution failed ({:?})",
                        fault_type
                    );
                }
            }

            let message = input
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("no message");
            format!("Echo: {}", message)
        })
    });

    registry
        .register_builtin(
            "echo",
            "Echo back the input message",
            json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                },
                "required": ["message"]
            }),
            handler,
        )
        .await;

    registry
}

// =============================================================================
// Registry Integration Tests (No Faults)
// =============================================================================

/// Test basic tool registration and execution through registry
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_registry_basic_execution() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let registry = create_registry_with_builtin(None).await;

            // Verify tool is registered
            assert!(registry.has_tool("echo").await);
            assert_eq!(
                registry.get_tool_source("echo").await,
                Some(ToolSource::Builtin)
            );

            // Execute tool
            let result = registry
                .execute("echo", &json!({"message": "hello world"}))
                .await;

            assert!(result.success, "Tool execution should succeed");
            assert_eq!(result.output, "Echo: hello world");
            assert!(result.duration_ms < 1000, "Execution should be fast");

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test executing non-existent tool
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_registry_tool_not_found() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let registry = UnifiedToolRegistry::new();

            let result = registry
                .execute("nonexistent", &json!({"foo": "bar"}))
                .await;

            assert!(!result.success, "Should fail for missing tool");
            assert!(
                result.output.contains("not found"),
                "Error should mention tool not found: {}",
                result.output
            );

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test getting tool definitions for LLM
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_registry_get_tool_definitions() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let registry = create_registry_with_builtin(None).await;

            // Also register an MCP tool
            registry
                .register_mcp_tool(
                    "mcp_tool",
                    "An MCP tool",
                    json!({"type": "object"}),
                    "test-server",
                )
                .await;

            let definitions = registry.get_tool_definitions().await;
            assert_eq!(definitions.len(), 2);

            // Verify definition fields are populated
            let echo_def = definitions.iter().find(|d| d.name == "echo");
            assert!(echo_def.is_some());
            let echo_def = echo_def.unwrap();
            assert!(!echo_def.description.is_empty());
            assert!(echo_def.input_schema.is_object());

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Fault Injection Tests - Builtin Tools
// =============================================================================

/// Test builtin tool execution with fault injection
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_registry_builtin_with_faults() {
    let config = SimConfig::new(12345);
    println!("DST seed: {}", config.seed);

    let fault_observed = Arc::new(AtomicBool::new(false));
    let fo = fault_observed.clone();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::McpToolFail, 1.0).with_filter("builtin_execute"))
        .run_async(move |env| {
            let fo = fo.clone();
            async move {
                let registry = create_registry_with_builtin(Some(env.faults.clone())).await;

                // Execute tool - should fail due to fault injection
                let result = registry.execute("echo", &json!({"message": "test"})).await;

                if !result.success {
                    println!("Fault correctly injected: {}", result.output);
                    fo.store(true, Ordering::SeqCst);
                } else {
                    println!("WARNING: No fault injected, got: {}", result.output);
                }

                // The tool should report failure
                assert!(!result.success, "Tool should fail under fault injection");
                assert!(
                    result.output.contains("simulated fault"),
                    "Error should indicate fault: {}",
                    result.output
                );

                Ok::<(), CoreError>(())
            }
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
    assert!(
        fault_observed.load(Ordering::SeqCst),
        "Fault should have been observed"
    );
}

/// Test partial fault injection (some succeed, some fail)
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_registry_partial_faults() {
    let config = SimConfig::new(67890);
    println!("DST seed: {}", config.seed);

    let success_count = Arc::new(AtomicUsize::new(0));
    let failure_count = Arc::new(AtomicUsize::new(0));
    let sc = success_count.clone();
    let fc = failure_count.clone();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::McpToolFail, 0.5).with_filter("builtin_execute"))
        .run_async(move |env| {
            let sc = sc.clone();
            let fc = fc.clone();
            async move {
                let registry = create_registry_with_builtin(Some(env.faults.clone())).await;

                // Execute many times
                for i in 0..20 {
                    let result = registry
                        .execute("echo", &json!({"message": format!("test {}", i)}))
                        .await;

                    if result.success {
                        sc.fetch_add(1, Ordering::SeqCst);
                    } else {
                        fc.fetch_add(1, Ordering::SeqCst);
                    }
                }

                Ok::<(), CoreError>(())
            }
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());

    let successes = success_count.load(Ordering::SeqCst);
    let failures = failure_count.load(Ordering::SeqCst);
    println!(
        "Partial faults: {} successes, {} failures out of 20",
        successes, failures
    );

    // With 50% fault rate, we should see both successes and failures
    assert!(successes > 0, "Should have some successes");
    assert!(failures > 0, "Should have some failures");
    assert_eq!(successes + failures, 20);
}

// =============================================================================
// MCP Tool Tests via Registry
// =============================================================================

#[cfg(feature = "dst")]
fn create_test_mcp_server(name: &str) -> SimMcpServerConfig {
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

/// Test MCP tool execution through registry with SimMcpClient
#[cfg(feature = "dst")]
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_registry_mcp_tool_execution() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|env| async move {
            let registry = UnifiedToolRegistry::new();

            // Create SimMcpClient and connect
            let mut mcp_client = SimMcpClient::new(env.faults.clone(), env.fork_rng_raw());
            mcp_client.register_server(create_test_mcp_server("server1"));
            mcp_client.connect("server1").await.map_err(to_core_error)?;

            // Register MCP tools in the registry
            // NOTE: Tool names must match between registry and SimMcpClient!
            let tools = mcp_client
                .discover_tools("server1")
                .await
                .map_err(to_core_error)?;

            for tool in &tools {
                registry
                    .register_mcp_tool(
                        &tool.name,
                        &tool.description,
                        tool.input_schema.clone(),
                        "server1",
                    )
                    .await;
            }

            // Inject SimMcpClient into registry
            registry.set_sim_mcp_client(Arc::new(mcp_client)).await;

            // Verify tools are registered
            assert!(registry.has_tool("server1_echo").await);
            assert!(registry.has_tool("server1_process").await);

            // Execute MCP tool through registry
            let result = registry
                .execute("server1_echo", &json!({"message": "hello from registry"}))
                .await;

            println!(
                "MCP tool result: success={}, output={}",
                result.success, result.output
            );

            assert!(
                result.success,
                "MCP tool execution should succeed: {}",
                result.output
            );

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test MCP tool execution with server crash fault
#[cfg(feature = "dst")]
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_registry_mcp_with_crash_fault() {
    let config = SimConfig::new(11111);
    println!("DST seed: {}", config.seed);

    let fault_observed = Arc::new(AtomicBool::new(false));
    let fo = fault_observed.clone();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::McpToolFail, 1.0).with_filter("mcp_execute"))
        .run_async(move |env| {
            let fo = fo.clone();
            async move {
                let registry = UnifiedToolRegistry::new();

                // Create and configure SimMcpClient
                let mut mcp_client = SimMcpClient::new(env.faults.clone(), env.fork_rng_raw());
                mcp_client.register_server(create_test_mcp_server("server1"));
                mcp_client.connect("server1").await.map_err(to_core_error)?;

                // Register tool in registry
                registry
                    .register_mcp_tool(
                        "server1_echo",
                        "Echo tool",
                        json!({"type": "object"}),
                        "server1",
                    )
                    .await;

                // Inject client
                registry.set_sim_mcp_client(Arc::new(mcp_client)).await;

                // Execute - should fail due to fault
                let result = registry
                    .execute("server1_echo", &json!({"message": "test"}))
                    .await;

                if !result.success {
                    println!("MCP fault correctly injected: {}", result.output);
                    fo.store(true, Ordering::SeqCst);
                }

                assert!(!result.success, "Should fail under fault injection");

                Ok::<(), CoreError>(())
            }
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
    assert!(
        fault_observed.load(Ordering::SeqCst),
        "Fault should have been observed"
    );
}

/// Test mixed builtin and MCP tools under faults
#[cfg(feature = "dst")]
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_registry_mixed_tools_under_faults() {
    let config = SimConfig::new(22222);
    println!("DST seed: {}", config.seed);

    let builtin_success = Arc::new(AtomicUsize::new(0));
    let builtin_fail = Arc::new(AtomicUsize::new(0));
    let mcp_success = Arc::new(AtomicUsize::new(0));
    let mcp_fail = Arc::new(AtomicUsize::new(0));

    let bs = builtin_success.clone();
    let bf = builtin_fail.clone();
    let ms = mcp_success.clone();
    let mf = mcp_fail.clone();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::McpToolFail, 0.3).with_filter("builtin_execute"))
        .with_fault(FaultConfig::new(FaultType::McpToolFail, 0.3).with_filter("mcp_execute"))
        .run_async(move |env| {
            let bs = bs.clone();
            let bf = bf.clone();
            let ms = ms.clone();
            let mf = mf.clone();
            async move {
                // Create registry with builtin tool
                let registry = create_registry_with_builtin(Some(env.faults.clone())).await;

                // Add MCP tool
                let mut mcp_client = SimMcpClient::new(env.faults.clone(), env.fork_rng_raw());
                mcp_client.register_server(create_test_mcp_server("server1"));
                mcp_client.connect("server1").await.map_err(to_core_error)?;

                registry
                    .register_mcp_tool(
                        "server1_echo",
                        "MCP Echo",
                        json!({"type": "object"}),
                        "server1",
                    )
                    .await;

                registry.set_sim_mcp_client(Arc::new(mcp_client)).await;

                // Execute both types of tools multiple times
                for i in 0..10 {
                    // Builtin tool
                    let result = registry
                        .execute("echo", &json!({"message": format!("builtin {}", i)}))
                        .await;
                    if result.success {
                        bs.fetch_add(1, Ordering::SeqCst);
                    } else {
                        bf.fetch_add(1, Ordering::SeqCst);
                    }

                    // MCP tool
                    let result = registry
                        .execute("server1_echo", &json!({"message": format!("mcp {}", i)}))
                        .await;
                    if result.success {
                        ms.fetch_add(1, Ordering::SeqCst);
                    } else {
                        mf.fetch_add(1, Ordering::SeqCst);
                    }
                }

                Ok::<(), CoreError>(())
            }
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());

    println!(
        "Builtin: {} success, {} fail",
        builtin_success.load(Ordering::SeqCst),
        builtin_fail.load(Ordering::SeqCst)
    );
    println!(
        "MCP: {} success, {} fail",
        mcp_success.load(Ordering::SeqCst),
        mcp_fail.load(Ordering::SeqCst)
    );

    // With 30% fault rate, should see mix of successes and failures
    assert!(builtin_success.load(Ordering::SeqCst) + builtin_fail.load(Ordering::SeqCst) == 10);
    assert!(mcp_success.load(Ordering::SeqCst) + mcp_fail.load(Ordering::SeqCst) == 10);
}

// =============================================================================
// BUG HUNTING: Tests to find issues
// =============================================================================

/// BUG HUNT: What happens when MCP tool is called but no SimMcpClient is set?
/// This simulates production where real MCP is not yet implemented.
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_registry_mcp_without_client() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let registry = UnifiedToolRegistry::new();

            // Register MCP tool but DON'T inject SimMcpClient
            registry
                .register_mcp_tool(
                    "orphan_tool",
                    "Tool without MCP client",
                    json!({"type": "object"}),
                    "nonexistent-server",
                )
                .await;

            // Try to execute - should fail gracefully
            let result = registry
                .execute("orphan_tool", &json!({"data": "test"}))
                .await;

            println!(
                "Orphan tool result: success={}, output={}",
                result.success, result.output
            );

            // Should fail, not panic
            assert!(!result.success, "Should fail without MCP client");
            assert!(
                result.output.contains("not yet implemented")
                    || result.output.contains("not found")
                    || result.output.contains("not connected"),
                "Error message should be helpful: {}",
                result.output
            );

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// BUG HUNT: Concurrent tool execution
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_registry_concurrent_execution() {
    let config = SimConfig::new(77777);
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            use kelpie_core::{current_runtime, Runtime};
            let runtime = current_runtime();
            let registry = Arc::new(create_registry_with_builtin(None).await);

            // Spawn multiple concurrent executions
            let mut handles = Vec::new();
            for i in 0..10 {
                let reg = registry.clone();
                let handle = runtime.spawn(async move {
                    let result = reg
                        .execute("echo", &json!({"message": format!("concurrent {}", i)}))
                        .await;
                    (i, result.success, result.output)
                });
                handles.push(handle);
            }

            // Wait for all and verify
            let mut results = Vec::new();
            for handle in handles {
                let (i, success, output) = handle.await.unwrap();
                results.push((i, success, output.clone()));
                assert!(success, "Concurrent execution {} failed: {}", i, output);
            }

            println!("Concurrent results: {} executions completed", results.len());
            assert_eq!(results.len(), 10);

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// BUG HUNT: Registry unregister and re-register
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_registry_unregister_reregister() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let registry = create_registry_with_builtin(None).await;

            // Execute works
            let result = registry.execute("echo", &json!({"message": "test1"})).await;
            assert!(result.success);

            // Unregister
            let removed = registry.unregister("echo").await;
            assert!(removed, "Tool should have been removed");

            // Execute should fail now
            let result = registry.execute("echo", &json!({"message": "test2"})).await;
            assert!(!result.success, "Should fail after unregister");
            assert!(result.output.contains("not found"));

            // Re-register with different handler
            let handler: BuiltinToolHandler =
                Arc::new(|_| Box::pin(async { "new handler".to_string() }));
            registry
                .register_builtin("echo", "Re-registered echo", json!({}), handler)
                .await;

            // Execute works again with new handler
            let result = registry.execute("echo", &json!({})).await;
            assert!(result.success);
            assert_eq!(result.output, "new handler");

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// BUG HUNT: What happens with very large input?
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_registry_large_input() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let registry = create_registry_with_builtin(None).await;

            // Create large input (1MB string)
            let large_message = "x".repeat(1024 * 1024);
            let result = registry
                .execute("echo", &json!({"message": large_message}))
                .await;

            // Should handle gracefully (either succeed or fail with clear error)
            println!(
                "Large input: success={}, output_len={}",
                result.success,
                result.output.len()
            );

            // We expect it to work since our echo handler just echoes
            assert!(result.success);

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Determinism Tests
// =============================================================================

/// Test that same seed produces same results
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_registry_determinism() {
    let seed = 33333u64;

    let mut results: Vec<Vec<String>> = Vec::new();

    for run in 0..2 {
        println!("Run {} with seed {}", run + 1, seed);

        let config = SimConfig::new(seed);
        let execution_log = Arc::new(std::sync::RwLock::new(Vec::<String>::new()));
        let log = execution_log.clone();

        let result = Simulation::new(config)
            .with_fault(
                FaultConfig::new(FaultType::McpToolFail, 0.5).with_filter("builtin_execute"),
            )
            .run_async(move |env| {
                let log = log.clone();
                async move {
                    let registry = create_registry_with_builtin(Some(env.faults.clone())).await;

                    for i in 0..10 {
                        let result = registry
                            .execute("echo", &json!({"message": format!("test {}", i)}))
                            .await;
                        let outcome = if result.success { "OK" } else { "FAIL" };
                        log.write().unwrap().push(format!("{}: {}", i, outcome));
                    }

                    Ok::<(), CoreError>(())
                }
            })
            .await;

        assert!(result.is_ok());
        results.push(execution_log.read().unwrap().clone());
    }

    println!("Run 1: {:?}", results[0]);
    println!("Run 2: {:?}", results[1]);
    assert_eq!(results[0], results[1], "Results should be deterministic");
}

// =============================================================================
// Stress Tests
// =============================================================================

/// High-load test with mixed faults
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_registry_high_load() {
    let config = SimConfig::new(44444);
    println!("DST seed: {}", config.seed);

    let total_success = Arc::new(AtomicUsize::new(0));
    let total_fail = Arc::new(AtomicUsize::new(0));
    let ts = total_success.clone();
    let tf = total_fail.clone();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::McpToolFail, 0.1).with_filter("builtin_execute"))
        .run_async(move |env| {
            let ts = ts.clone();
            let tf = tf.clone();
            async move {
                let registry = create_registry_with_builtin(Some(env.faults.clone())).await;

                // Simulate high load: 100 tool executions
                for i in 0..100 {
                    let result = registry
                        .execute("echo", &json!({"message": format!("load test {}", i)}))
                        .await;
                    if result.success {
                        ts.fetch_add(1, Ordering::SeqCst);
                    } else {
                        tf.fetch_add(1, Ordering::SeqCst);
                    }
                }

                Ok::<(), CoreError>(())
            }
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());

    let successes = total_success.load(Ordering::SeqCst);
    let failures = total_fail.load(Ordering::SeqCst);

    println!(
        "High load: {} successes, {} failures out of 100",
        successes, failures
    );

    // With 10% fault rate, expect roughly 90 successes and 10 failures
    assert!(successes > 80, "Expected mostly successes");
    assert!(failures > 0, "Expected some failures");
    assert_eq!(successes + failures, 100);
}

// =============================================================================
// Edge Cases and Error Handling
// =============================================================================

/// Test registry behavior with empty input
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_registry_empty_input() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let registry = create_registry_with_builtin(None).await;

            // Execute with empty object
            let result = registry.execute("echo", &json!({})).await;

            // Should still succeed, just with default message
            assert!(result.success);
            assert!(result.output.contains("no message"));

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test registry stats after operations
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_registry_stats() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .run_async(|_env| async move {
            let registry = UnifiedToolRegistry::new();

            // Initial stats
            let stats = registry.stats().await;
            assert_eq!(stats.total_tools, 0);
            assert_eq!(stats.builtin_count, 0);
            assert_eq!(stats.mcp_count, 0);

            // Add builtin tool
            let handler: BuiltinToolHandler = Arc::new(|_| Box::pin(async { "ok".to_string() }));
            registry
                .register_builtin("tool1", "Test", json!({}), handler)
                .await;

            // Add MCP tools
            registry
                .register_mcp_tool("mcp1", "MCP 1", json!({}), "server1")
                .await;
            registry
                .register_mcp_tool("mcp2", "MCP 2", json!({}), "server2")
                .await;

            let stats = registry.stats().await;
            assert_eq!(stats.total_tools, 3);
            assert_eq!(stats.builtin_count, 1);
            assert_eq!(stats.mcp_count, 2);

            Ok::<(), CoreError>(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}
