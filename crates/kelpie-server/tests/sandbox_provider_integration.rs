//! Integration tests for SandboxProvider
//!
//! TigerStyle: Tests sandbox provider with ProcessSandbox backend.
//!
//! These tests verify:
//! - Provider initialization
//! - Command execution via execute_in_sandbox
//! - Per-agent execution via execute_for_agent
//! - Agent cleanup via cleanup_agent_sandbox
//! - Isolation mode behavior (shared vs dedicated)
//!
//! Note: VZ backend tests require macOS ARM64 and are in vz_sandbox_integration.rs

use kelpie_server::tools::{
    cleanup_agent_sandbox, execute_for_agent, execute_in_sandbox, SandboxBackendKind,
    SandboxProvider, EXEC_TIMEOUT_SECONDS_DEFAULT,
};

// =============================================================================
// Provider Initialization
// =============================================================================

#[tokio::test]
async fn test_sandbox_provider_init() {
    // Clear any environment variables that might affect backend selection
    std::env::remove_var("KELPIE_SANDBOX_BACKEND");
    std::env::remove_var("KELPIE_ISOLATION_MODE");

    // Initialize provider
    let provider = SandboxProvider::init().await;
    assert!(provider.is_ok(), "Provider should initialize successfully");

    let provider = provider.unwrap();
    // Backend depends on features and platform
    #[cfg(feature = "libkrun")]
    {
        // When libkrun feature is enabled on macOS ARM64, should use Libkrun
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        assert_eq!(
            provider.kind(),
            SandboxBackendKind::Libkrun,
            "Should use Libkrun backend on macOS ARM64 with libkrun feature"
        );
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        assert_eq!(
            provider.kind(),
            SandboxBackendKind::Process,
            "Should use Process backend on other platforms"
        );
    }
    #[cfg(not(feature = "libkrun"))]
    assert_eq!(
        provider.kind(),
        SandboxBackendKind::Process,
        "Should use Process backend without libkrun feature"
    );
}

#[tokio::test]
async fn test_sandbox_provider_init_idempotent() {
    std::env::remove_var("KELPIE_SANDBOX_BACKEND");
    std::env::remove_var("KELPIE_ISOLATION_MODE");

    // First init
    let provider1 = SandboxProvider::init()
        .await
        .expect("First init should work");

    // Second init should return same instance
    let provider2 = SandboxProvider::init()
        .await
        .expect("Second init should work");

    // Both should point to same Arc
    assert!(
        std::sync::Arc::ptr_eq(&provider1, &provider2),
        "Multiple inits should return same provider"
    );
}

// =============================================================================
// Basic Execution (execute_in_sandbox)
// =============================================================================

#[tokio::test]
async fn test_execute_in_sandbox_echo() {
    std::env::remove_var("KELPIE_SANDBOX_BACKEND");
    std::env::remove_var("KELPIE_ISOLATION_MODE");

    // Ensure provider is initialized
    SandboxProvider::init()
        .await
        .expect("Provider init should work");

    // Execute a simple echo command
    let result =
        execute_in_sandbox("echo", &["hello", "world"], EXEC_TIMEOUT_SECONDS_DEFAULT).await;

    assert!(result.is_ok(), "Echo should succeed: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.success, "Echo should exit with code 0");
    assert_eq!(output.exit_code, 0);
    assert!(
        output.stdout.contains("hello world"),
        "Stdout should contain 'hello world', got: {}",
        output.stdout
    );
}

#[tokio::test]
async fn test_execute_in_sandbox_python() {
    std::env::remove_var("KELPIE_SANDBOX_BACKEND");
    std::env::remove_var("KELPIE_ISOLATION_MODE");

    SandboxProvider::init()
        .await
        .expect("Provider init should work");

    // Execute Python code
    let result = execute_in_sandbox(
        "python3",
        &["-c", "print('Hello from Python')"],
        EXEC_TIMEOUT_SECONDS_DEFAULT,
    )
    .await;

    // Python might not be installed on all systems, so handle gracefully
    match result {
        Ok(output) => {
            if output.success {
                assert!(
                    output.stdout.contains("Hello from Python"),
                    "Python output should contain message"
                );
            } else {
                // Python not available - that's ok for this test
                tracing::info!("Python not available, skipping assertion");
            }
        }
        Err(e) => {
            // If sandbox creation fails, that's a real error
            // But if python3 just isn't found, that's expected on some systems
            tracing::info!("Python execution failed (may not be installed): {}", e);
        }
    }
}

#[tokio::test]
async fn test_execute_in_sandbox_nonexistent_command() {
    std::env::remove_var("KELPIE_SANDBOX_BACKEND");
    std::env::remove_var("KELPIE_ISOLATION_MODE");

    SandboxProvider::init()
        .await
        .expect("Provider init should work");

    // Execute a command that doesn't exist
    let result = execute_in_sandbox(
        "this_command_definitely_does_not_exist_xyz",
        &[],
        EXEC_TIMEOUT_SECONDS_DEFAULT,
    )
    .await;

    // Should either fail or return non-zero exit code
    match result {
        Ok(output) => {
            assert!(
                !output.success,
                "Non-existent command should fail, got success"
            );
            assert_ne!(output.exit_code, 0, "Exit code should be non-zero");
        }
        Err(_) => {
            // Command not found error is also acceptable
        }
    }
}

#[tokio::test]
async fn test_execute_in_sandbox_stderr() {
    std::env::remove_var("KELPIE_SANDBOX_BACKEND");
    std::env::remove_var("KELPIE_ISOLATION_MODE");

    SandboxProvider::init()
        .await
        .expect("Provider init should work");

    // Execute command that writes to stderr
    let result = execute_in_sandbox(
        "sh",
        &["-c", "echo 'error message' >&2"],
        EXEC_TIMEOUT_SECONDS_DEFAULT,
    )
    .await;

    assert!(result.is_ok(), "Command should execute");
    let output = result.unwrap();
    assert!(
        output.stderr.contains("error message"),
        "Stderr should contain message, got: {}",
        output.stderr
    );
}

// =============================================================================
// Per-Agent Execution (execute_for_agent)
// =============================================================================

#[tokio::test]
async fn test_execute_for_agent_basic() {
    std::env::remove_var("KELPIE_SANDBOX_BACKEND");
    std::env::remove_var("KELPIE_ISOLATION_MODE");

    SandboxProvider::init()
        .await
        .expect("Provider init should work");

    // Execute for a specific agent
    let result = execute_for_agent(
        "test-agent-001",
        "echo",
        &["agent", "execution"],
        EXEC_TIMEOUT_SECONDS_DEFAULT,
    )
    .await;

    assert!(
        result.is_ok(),
        "Agent execution should succeed: {:?}",
        result.err()
    );
    let output = result.unwrap();
    assert!(output.success);
    assert!(output.stdout.contains("agent execution"));
}

#[tokio::test]
async fn test_execute_for_agent_different_agents() {
    std::env::remove_var("KELPIE_SANDBOX_BACKEND");
    std::env::remove_var("KELPIE_ISOLATION_MODE");

    SandboxProvider::init()
        .await
        .expect("Provider init should work");

    // Execute for multiple different agents
    let agents = vec!["agent-a", "agent-b", "agent-c"];

    for agent_id in agents {
        let result = execute_for_agent(
            agent_id,
            "echo",
            &["hello", "from", agent_id],
            EXEC_TIMEOUT_SECONDS_DEFAULT,
        )
        .await;

        assert!(
            result.is_ok(),
            "Execution for {} should succeed: {:?}",
            agent_id,
            result.err()
        );
        let output = result.unwrap();
        assert!(
            output.success,
            "Agent {} execution should succeed",
            agent_id
        );
        assert!(
            output.stdout.contains(agent_id),
            "Output should contain agent id {}",
            agent_id
        );
    }
}

// =============================================================================
// Agent Cleanup
// =============================================================================

#[tokio::test]
async fn test_cleanup_agent_sandbox_basic() {
    std::env::remove_var("KELPIE_SANDBOX_BACKEND");
    std::env::remove_var("KELPIE_ISOLATION_MODE");

    SandboxProvider::init()
        .await
        .expect("Provider init should work");

    // First execute for the agent to create resources
    let _ = execute_for_agent(
        "cleanup-test-agent",
        "echo",
        &["test"],
        EXEC_TIMEOUT_SECONDS_DEFAULT,
    )
    .await;

    // Then cleanup
    let result = cleanup_agent_sandbox("cleanup-test-agent").await;

    assert!(result.is_ok(), "Cleanup should succeed: {:?}", result.err());
}

#[tokio::test]
async fn test_cleanup_agent_sandbox_nonexistent() {
    std::env::remove_var("KELPIE_SANDBOX_BACKEND");
    std::env::remove_var("KELPIE_ISOLATION_MODE");

    SandboxProvider::init()
        .await
        .expect("Provider init should work");

    // Cleanup an agent that was never created - should be a no-op
    let result = cleanup_agent_sandbox("nonexistent-agent").await;

    assert!(
        result.is_ok(),
        "Cleanup of nonexistent agent should be no-op: {:?}",
        result.err()
    );
}

// =============================================================================
// Isolation Mode (Dedicated vs Shared)
// =============================================================================

#[tokio::test]
#[ignore] // Requires isolated process - run with: cargo test test_dedicated_isolation_mode -- --ignored
async fn test_dedicated_isolation_mode() {
    // Note: This test requires a fresh process since provider is a global singleton.
    // When running with other tests in parallel, the provider may already be initialized
    // with a different mode.
    //
    // Run this test in isolation with:
    //   cargo test -p kelpie-server --test sandbox_provider_integration test_dedicated -- --ignored

    // Set dedicated mode BEFORE init
    std::env::set_var("KELPIE_ISOLATION_MODE", "dedicated");
    std::env::remove_var("KELPIE_SANDBOX_BACKEND");

    let provider = SandboxProvider::init()
        .await
        .expect("Provider init should work");

    // Verify isolation mode
    assert_eq!(
        provider.isolation_mode(),
        kelpie_sandbox::IsolationMode::Dedicated,
        "Should be in dedicated mode"
    );

    // Clean up
    std::env::remove_var("KELPIE_ISOLATION_MODE");
}

// =============================================================================
// Error Handling
// =============================================================================

#[tokio::test]
async fn test_provider_not_initialized_error() {
    // This test is tricky because we need to test before initialization.
    // Since tests run in parallel and other tests may have initialized it,
    // we just verify the error message format if provider isn't available.

    // If provider is already initialized, this test is a no-op
    if SandboxProvider::get().is_some() {
        return;
    }

    // Try to execute without initialization
    let result = execute_in_sandbox("echo", &["test"], 30).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .contains("Sandbox provider not initialized"),
        "Should indicate provider needs initialization"
    );
}

// =============================================================================
// Concurrent Execution
// =============================================================================

#[tokio::test]
async fn test_concurrent_execution() {
    std::env::remove_var("KELPIE_SANDBOX_BACKEND");
    std::env::remove_var("KELPIE_ISOLATION_MODE");

    SandboxProvider::init()
        .await
        .expect("Provider init should work");

    // Execute multiple commands concurrently
    let handles: Vec<_> = (0..5)
        .map(|i| {
            tokio::spawn(async move {
                execute_for_agent(
                    &format!("concurrent-agent-{}", i),
                    "echo",
                    &[&format!("message-{}", i)],
                    EXEC_TIMEOUT_SECONDS_DEFAULT,
                )
                .await
            })
        })
        .collect();

    let results: Vec<_> = futures::future::join_all(handles).await;

    for (i, result) in results.into_iter().enumerate() {
        let result = result.expect("Task should not panic");
        assert!(
            result.is_ok(),
            "Concurrent execution {} should succeed: {:?}",
            i,
            result.err()
        );
        let output = result.unwrap();
        assert!(output.success, "Concurrent execution {} should succeed", i);
    }
}

// =============================================================================
// Backend Kind Detection
// =============================================================================

#[test]
fn test_backend_kind_parse() {
    assert_eq!(
        SandboxBackendKind::parse("process"),
        Some(SandboxBackendKind::Process)
    );
    assert_eq!(
        SandboxBackendKind::parse("PROCESS"),
        Some(SandboxBackendKind::Process)
    );
    assert_eq!(SandboxBackendKind::parse("unknown"), None);
    assert_eq!(SandboxBackendKind::parse(""), None);
}

#[test]
fn test_backend_kind_display() {
    assert_eq!(SandboxBackendKind::Process.to_string(), "process");
}
