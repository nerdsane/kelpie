//! Integration tests for custom tool execution
//!
//! TigerStyle: Tests verify end-to-end flow from registration to execution.
//!
//! NOTE: Tests marked with `#[ignore]` require:
//! - A writable filesystem for sandbox working directories
//! - Python, Node.js, and bash installed
//!
//! Run with: `cargo test --test custom_tool_integration -- --ignored`

// Allow tokio::spawn in integration tests - these don't need DST compatibility
#![allow(clippy::disallowed_methods)]

use kelpie_server::tools::UnifiedToolRegistry;
use serde_json::json;

/// Test that a custom Python tool can be registered and executed
///
/// Requires: writable filesystem, Python installed
#[tokio::test]
#[ignore = "requires writable filesystem and Python"]
async fn test_custom_python_tool_execution() {
    let registry = UnifiedToolRegistry::new();

    // Register a simple Python tool
    registry
        .register_custom_tool(
            "add_numbers",
            "Adds two numbers",
            json!({
                "type": "object",
                "properties": {
                    "a": {"type": "number"},
                    "b": {"type": "number"}
                },
                "required": ["a", "b"]
            }),
            r#"
import json
import sys

input_data = json.loads(sys.stdin.read())
a = input_data.get("a", 0)
b = input_data.get("b", 0)
result = a + b
print(json.dumps({"result": result}))
"#
            .to_string(),
            "python",
            vec![],
        )
        .await;

    // Verify tool is registered
    let tools = registry.list_tools().await;
    assert!(
        tools.iter().any(|t| t == "add_numbers"),
        "Tool should be in registry"
    );

    // Execute the tool
    let input = json!({"a": 5, "b": 3});

    let exec_result = registry.execute("add_numbers", &input).await;

    assert!(exec_result.success, "Execution should succeed");
    assert!(
        exec_result.output.contains("8"),
        "Output should contain sum: {}",
        exec_result.output
    );
}

/// Test that a custom JavaScript tool can be registered and executed
///
/// Requires: writable filesystem, Node.js installed
#[tokio::test]
#[ignore = "requires writable filesystem and Node.js"]
async fn test_custom_javascript_tool_execution() {
    let registry = UnifiedToolRegistry::new();

    // Register a simple JavaScript tool
    registry
        .register_custom_tool(
            "greet",
            "Greets a person",
            json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string"}
                },
                "required": ["name"]
            }),
            r#"
const input = JSON.parse(require('fs').readFileSync('/dev/stdin', 'utf8'));
const name = input.name || 'World';
console.log(JSON.stringify({ greeting: `Hello, ${name}!` }));
"#
            .to_string(),
            "javascript",
            vec![],
        )
        .await;

    // Execute the tool
    let input = json!({"name": "Alice"});

    let exec_result = registry.execute("greet", &input).await;

    assert!(exec_result.success, "Execution should succeed");
    assert!(
        exec_result.output.contains("Hello, Alice!"),
        "Output should contain greeting: {}",
        exec_result.output
    );
}

/// Test that a custom shell tool can be registered and executed
///
/// Requires: writable filesystem, bash installed
#[tokio::test]
#[ignore = "requires writable filesystem and bash"]
async fn test_custom_shell_tool_execution() {
    let registry = UnifiedToolRegistry::new();

    // Register a simple shell tool
    registry
        .register_custom_tool(
            "echo_input",
            "Echoes input back",
            json!({
                "type": "object",
                "properties": {
                    "message": {"type": "string"}
                },
                "required": ["message"]
            }),
            r#"echo "Received: $INPUT_MESSAGE""#.to_string(),
            "shell",
            vec![],
        )
        .await;

    // Execute the tool
    let input = json!({"message": "test message"});

    let exec_result = registry.execute("echo_input", &input).await;

    assert!(exec_result.success, "Execution should succeed");
    assert!(
        exec_result.output.contains("Received:"),
        "Output should contain echo: {}",
        exec_result.output
    );
}

/// Test that tool execution with sandbox pool works correctly
///
/// Requires: writable filesystem, Python installed
#[tokio::test]
#[ignore = "requires writable filesystem and Python"]
async fn test_tool_execution_with_sandbox_pool() {
    use kelpie_sandbox::{PoolConfig, ProcessSandboxFactory, SandboxConfig, SandboxPool};
    use std::sync::Arc;

    // Create a sandbox pool
    let pool_config = PoolConfig::new(SandboxConfig::default())
        .with_min_size(1)
        .with_max_size(2);

    let pool = SandboxPool::new(ProcessSandboxFactory::new(), pool_config)
        .expect("Pool creation should succeed");

    let registry = UnifiedToolRegistry::new();
    registry.set_sandbox_pool(Arc::new(pool)).await;

    // Register a tool
    registry
        .register_custom_tool(
            "pooled_tool",
            "Tests pooled execution",
            json!({"type": "object"}),
            "print('pooled execution works')".to_string(),
            "python",
            vec![],
        )
        .await;

    // Execute multiple times to test pool reuse
    for i in 0..3 {
        let result = registry.execute("pooled_tool", &json!({})).await;
        assert!(
            result.success,
            "Execution {} should succeed: {:?}",
            i, result.error
        );
    }
}

/// Test that unsupported runtime returns error
#[tokio::test]
async fn test_unsupported_runtime_error() {
    let registry = UnifiedToolRegistry::new();

    // Register tool with unsupported runtime
    registry
        .register_custom_tool(
            "invalid_tool",
            "Uses invalid runtime",
            json!({"type": "object"}),
            "some code".to_string(),
            "rust", // Not a supported runtime
            vec![],
        )
        .await;

    // Execution should fail
    let exec_result = registry.execute("invalid_tool", &json!({})).await;

    assert!(
        !exec_result.success,
        "Execution with unsupported runtime should fail"
    );
    assert!(
        exec_result.error.is_some(),
        "Error message should be present"
    );
}

/// Test concurrent tool execution
///
/// Requires: writable filesystem, Python installed
#[tokio::test]
#[ignore = "requires writable filesystem and Python"]
async fn test_concurrent_tool_execution() {
    use std::sync::Arc;

    let registry = Arc::new(UnifiedToolRegistry::new());

    // Register a tool
    registry
        .register_custom_tool(
            "concurrent_tool",
            "For concurrent testing",
            json!({
                "type": "object",
                "properties": {
                    "id": {"type": "number"}
                }
            }),
            r#"
import json
import sys
import time

input_data = json.loads(sys.stdin.read())
id = input_data.get("id", 0)
time.sleep(0.1)  # Small delay to test concurrency
print(json.dumps({"id": id, "done": True}))
"#
            .to_string(),
            "python",
            vec![],
        )
        .await;

    // Execute concurrently
    let handles: Vec<_> = (0..3)
        .map(|i| {
            let registry = Arc::clone(&registry);
            tokio::spawn(async move {
                let input = json!({"id": i});
                registry.execute("concurrent_tool", &input).await
            })
        })
        .collect();

    // All should succeed
    for (i, handle) in handles.into_iter().enumerate() {
        let result = handle.await.expect("Task should complete");
        assert!(
            result.success,
            "Concurrent execution {} should succeed: {:?}",
            i, result.error
        );
    }
}
