//! DST Tests for Heartbeat Integration with Agent Loop
//!
//! TigerStyle: Meaningful fault injection testing the integration points
//! where pause_heartbeats interacts with state operations.
//!
//! These tests inject faults into:
//! - Message storage (message_write)
//! - Block reads for context building (block_read)
//! - Agent state updates (agent_write)
//!
//! Run with: cargo test -p kelpie-server --features dst --test heartbeat_integration_dst
#![cfg(feature = "dst")]
#![allow(deprecated)]

use kelpie_dst::fault::FaultConfig;
use kelpie_dst::{FaultType, SimConfig, Simulation};
use kelpie_server::models::{AgentState, AgentType, CreateAgentRequest, CreateBlockRequest};
use kelpie_server::state::AppState;
use kelpie_server::tools::{parse_pause_signal, register_pause_heartbeats_with_clock, ClockSource};
use serde_json::json;
use std::sync::Arc;

// =============================================================================
// Helper Functions
// =============================================================================

fn create_test_agent(name: &str) -> AgentState {
    AgentState::from_request(CreateAgentRequest {
        name: name.to_string(),
        agent_type: AgentType::MemgptAgent,
        model: None,
        embedding: None,
        system: None,
        description: None,
        memory_blocks: vec![CreateBlockRequest {
            label: "persona".to_string(),
            value: "Test agent persona".to_string(),
            description: None,
            limit: None,
        }],
        block_ids: vec![],
        tool_ids: vec![],
        tags: vec![],
        metadata: serde_json::json!({}),
        project_id: None,
        user_id: None,
        org_id: None,
    })
}

// =============================================================================
// Fault Injection Tests
// =============================================================================

/// Test: Message write fails after pause_heartbeats is called
///
/// Scenario: Agent calls pause_heartbeats, tool succeeds, but storing
/// the tool result message fails. Verifies fault is properly propagated.
#[cfg(feature = "dst")]
#[test]
fn test_message_write_fault_after_pause() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    // Configure fault to fail message_write operations
    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 1.0).with_filter("message_write"))
        .run(|env| async move {
            // Create state with the simulation's fault injector
            let state =
                AppState::with_fault_injector(kelpie_core::current_runtime(), env.faults.clone());
            let registry = state.tool_registry();

            // Register pause_heartbeats with simulated clock
            let clock = env.clock.clone();
            let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
            register_pause_heartbeats_with_clock(registry, ClockSource::Sim(clock_fn)).await;

            // Create agent (this works because we only filter message_write)
            let agent = create_test_agent("test-agent");
            let agent_id = agent.id.clone();
            state
                .create_agent(agent)
                .expect("Agent creation should work");

            // Execute pause_heartbeats - tool itself should succeed (no storage)
            let tool_result = registry
                .execute("pause_heartbeats", &json!({ "minutes": 5 }))
                .await;
            assert!(tool_result.success, "Tool execution should succeed");

            // Parse the pause signal
            let (minutes, _pause_until) =
                parse_pause_signal(&tool_result.output).expect("Should parse pause signal");
            assert_eq!(minutes, 5);

            // Now try to store a message - this should FAIL due to fault
            let message = kelpie_server::models::Message {
                id: "msg-1".to_string(),
                agent_id: agent_id.clone(),
                message_type: "tool_result".to_string(),
                role: kelpie_server::models::MessageRole::Tool,
                content: tool_result.output.clone(),
                tool_call_id: Some("call-1".to_string()),
                tool_calls: vec![],
                tool_call: None,
                tool_return: None,
                status: None,
                created_at: chrono::DateTime::from_timestamp(1700000000, 0).unwrap(),
            };

            let store_result = state.add_message(&agent_id, message);
            assert!(
                store_result.is_err(),
                "Message storage should fail due to fault injection"
            );

            // Verify the error is the expected fault
            match store_result {
                Err(kelpie_server::state::StateError::FaultInjected { operation }) => {
                    assert_eq!(operation, "message_write");
                }
                other => panic!("Expected FaultInjected error, got: {:?}", other),
            }

            Ok(())
        });

    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());
}

/// Test: Block read fails when building context before pause
///
/// Scenario: Agent loop tries to read memory blocks for system prompt,
/// but block_read fails. Verifies graceful handling.
#[cfg(feature = "dst")]
#[test]
fn test_block_read_fault_during_context_build() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 1.0).with_filter("block_read"))
        .run(|env| async move {
            let state =
                AppState::with_fault_injector(kelpie_core::current_runtime(), env.faults.clone());

            // Create agent (works - block_read fault only affects reads)
            let agent = create_test_agent("test-agent");
            let agent_id = agent.id.clone();
            state
                .create_agent(agent)
                .expect("Agent creation should work");

            // Try to read block by label - should fail
            let block_result = state.get_block_by_label(&agent_id, "persona");
            assert!(
                block_result.is_err(),
                "Block read should fail due to fault injection"
            );

            match block_result {
                Err(kelpie_server::state::StateError::FaultInjected { operation }) => {
                    assert_eq!(operation, "block_read");
                }
                other => panic!("Expected FaultInjected error, got: {:?}", other),
            }

            Ok(())
        });

    assert!(result.is_ok());
}

/// Test: Probabilistic faults during agent loop simulation
///
/// Scenario: 30% fault rate on message_write. Some operations succeed,
/// some fail. Verifies system handles mixed success/failure.
#[cfg(feature = "dst")]
#[test]
fn test_probabilistic_faults_during_pause_flow() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.3).with_filter("message_write"))
        .run(|env| async move {
            let state =
                AppState::with_fault_injector(kelpie_core::current_runtime(), env.faults.clone());
            let registry = state.tool_registry();

            let clock = env.clock.clone();
            let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
            register_pause_heartbeats_with_clock(registry, ClockSource::Sim(clock_fn)).await;

            // Create agent
            let agent = create_test_agent("test-agent");
            let agent_id = agent.id.clone();
            state
                .create_agent(agent)
                .expect("Agent creation should work");

            // Simulate 20 pause/message cycles
            let mut successes = 0;
            let mut failures = 0;

            for i in 0..20 {
                // Execute pause_heartbeats (always succeeds - no storage)
                let tool_result = registry
                    .execute("pause_heartbeats", &json!({ "minutes": (i % 60) + 1 }))
                    .await;
                assert!(tool_result.success);

                // Try to store message (may fail due to 30% fault)
                let message = kelpie_server::models::Message {
                    id: format!("msg-{}", i),
                    agent_id: agent_id.clone(),
                    message_type: "tool_result".to_string(),
                    role: kelpie_server::models::MessageRole::Tool,
                    content: tool_result.output,
                    tool_call_id: Some(format!("call-{}", i)),
                    tool_calls: vec![],
                    tool_call: None,
                    tool_return: None,
                    status: None,
                    created_at: chrono::DateTime::from_timestamp(1700000000, 0).unwrap(),
                };

                match state.add_message(&agent_id, message) {
                    Ok(_) => successes += 1,
                    Err(kelpie_server::state::StateError::FaultInjected { .. }) => failures += 1,
                    Err(e) => panic!("Unexpected error: {:?}", e),
                }
            }

            // With 30% fault rate over 20 attempts, expect roughly 6 failures
            println!(
                "Results: {} successes, {} failures (expected ~70% success)",
                successes, failures
            );
            assert!(successes > 0, "Should have some successes");
            assert!(
                failures > 0,
                "Should have some failures with 30% fault rate"
            );
            assert_eq!(successes + failures, 20);

            Ok(())
        });

    assert!(result.is_ok());
}

/// Test: Agent write fault - configured upfront
#[cfg(feature = "dst")]
#[test]
fn test_agent_write_fault() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    // Configure agent_write fault upfront
    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 1.0).with_filter("agent_write"))
        .run(|env| async move {
            let state =
                AppState::with_fault_injector(kelpie_core::current_runtime(), env.faults.clone());
            let registry = state.tool_registry();

            let clock = env.clock.clone();
            let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
            register_pause_heartbeats_with_clock(registry, ClockSource::Sim(clock_fn)).await;

            // Create agent - this should still work (create_agent doesn't use agent_write)
            let agent = create_test_agent("test-agent");
            let agent_id = agent.id.clone();
            state
                .create_agent(agent)
                .expect("Agent creation should work");

            // Execute pause - still works (no storage)
            let tool_result = registry
                .execute("pause_heartbeats", &json!({ "minutes": 10 }))
                .await;
            assert!(tool_result.success);

            // Try to update agent - should fail due to agent_write fault
            let update_result = state.update_agent(&agent_id, |agent| {
                agent.name = "updated-name".to_string();
            });

            assert!(
                update_result.is_err(),
                "Agent update should fail due to fault"
            );

            Ok(())
        });

    assert!(result.is_ok());
}

/// Test: Multiple faults active simultaneously - configured upfront
#[cfg(feature = "dst")]
#[test]
fn test_multiple_simultaneous_faults() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 1.0).with_filter("block_read"))
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 1.0).with_filter("message_write"))
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 1.0).with_filter("agent_write"))
        .run(|env| async move {
            let state =
                AppState::with_fault_injector(kelpie_core::current_runtime(), env.faults.clone());
            let registry = state.tool_registry();

            let clock = env.clock.clone();
            let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
            register_pause_heartbeats_with_clock(registry, ClockSource::Sim(clock_fn)).await;

            // Create agent (works - create_agent doesn't hit these faults)
            let agent = create_test_agent("test-agent");
            let agent_id = agent.id.clone();
            state
                .create_agent(agent)
                .expect("Agent creation should work");

            // pause_heartbeats still works (no storage)
            let tool_result = registry
                .execute("pause_heartbeats", &json!({ "minutes": 5 }))
                .await;
            assert!(tool_result.success, "Tool should work despite other faults");

            // But all state operations fail
            assert!(
                state.get_block_by_label(&agent_id, "persona").is_err(),
                "Block read should fail"
            );

            let message = kelpie_server::models::Message {
                id: "msg-1".to_string(),
                agent_id: agent_id.clone(),
                message_type: "tool_result".to_string(),
                role: kelpie_server::models::MessageRole::Tool,
                content: "test".to_string(),
                tool_call_id: None,
                tool_calls: vec![],
                tool_call: None,
                tool_return: None,
                status: None,
                created_at: chrono::DateTime::from_timestamp(1700000000, 0).unwrap(),
            };
            assert!(
                state.add_message(&agent_id, message).is_err(),
                "Message write should fail"
            );

            assert!(
                state
                    .update_agent(&agent_id, |a| a.name = "x".to_string())
                    .is_err(),
                "Agent update should fail"
            );

            Ok(())
        });

    assert!(result.is_ok());
}

/// Test: Determinism - same seed produces same fault pattern
#[cfg(feature = "dst")]
#[test]
fn test_fault_injection_determinism() {
    let seed = 42u64;

    let run_simulation = || {
        let config = SimConfig::new(seed);
        Simulation::new(config)
            .with_fault(
                FaultConfig::new(FaultType::StorageWriteFail, 0.5).with_filter("message_write"),
            )
            .run(|env| async move {
                let state = AppState::with_fault_injector(
                    kelpie_core::current_runtime(),
                    env.faults.clone(),
                );

                let agent = create_test_agent("test-agent");
                let agent_id = agent.id.clone();
                state
                    .create_agent(agent)
                    .expect("Agent creation should work");

                let mut results = Vec::new();
                for i in 0..10 {
                    let message = kelpie_server::models::Message {
                        id: format!("msg-{}", i),
                        agent_id: agent_id.clone(),
                        message_type: "test".to_string(),
                        role: kelpie_server::models::MessageRole::Tool,
                        content: format!("content-{}", i),
                        tool_call_id: None,
                        tool_calls: vec![],
                        tool_call: None,
                        tool_return: None,
                        status: None,
                        created_at: chrono::DateTime::from_timestamp(1700000000, 0).unwrap(),
                    };
                    results.push(state.add_message(&agent_id, message).is_ok());
                }
                Ok(results)
            })
    };

    let result1 = run_simulation().expect("First run failed");
    let result2 = run_simulation().expect("Second run failed");

    assert_eq!(
        result1, result2,
        "Same seed must produce same fault pattern"
    );
}

/// Test: Pause tool isolation - works even when everything else fails
#[cfg(feature = "dst")]
#[test]
fn test_pause_tool_isolation_from_storage_faults() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    // Configure ALL storage operations to fail
    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 1.0))
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 1.0))
        .run(|env| async move {
            let state =
                AppState::with_fault_injector(kelpie_core::current_runtime(), env.faults.clone());
            let registry = state.tool_registry();

            let clock = env.clock.clone();
            let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
            register_pause_heartbeats_with_clock(registry, ClockSource::Sim(clock_fn)).await;

            // pause_heartbeats should STILL work - it's stateless
            for minutes in 1..=60 {
                let tool_result = registry
                    .execute("pause_heartbeats", &json!({ "minutes": minutes }))
                    .await;
                assert!(
                    tool_result.success,
                    "pause_heartbeats should work at {} minutes even with storage faults",
                    minutes
                );

                let (actual_minutes, _) =
                    parse_pause_signal(&tool_result.output).expect("Should parse pause signal");
                assert_eq!(actual_minutes, minutes);
            }

            Ok(())
        });

    assert!(result.is_ok());
}
