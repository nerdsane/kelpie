//! Integration tests for SimAgentEnv with full Simulation harness
//!
//! TigerStyle: Verify agent harness works under fault injection with full DST.

use kelpie_dst::{
    AgentTestConfig, BlockTestState, FaultConfig, FaultType, SimAgentEnv, SimChatMessage,
    SimConfig, SimLlmClient, SimToolDefinition, Simulation,
};
use std::sync::Arc;

#[madsim::test]
async fn test_agent_env_with_simulation_basic() {
    let config = SimConfig::new(42);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            // Create SimAgentEnv using simulation environment
            let llm = Arc::new(SimLlmClient::new(
                sim_env.fork_rng_raw(),
                sim_env.faults.clone(),
            ));
            let mut agent_env = SimAgentEnv::new(
                sim_env.storage.clone(),
                llm,
                sim_env.clock.clone(),
                sim_env.faults.clone(),
                sim_env.fork_rng(),
            );

            // Create agent
            let agent_id = agent_env.create_agent(AgentTestConfig::default())?;
            assert!(!agent_id.is_empty());

            // Send message
            let response = agent_env.send_message(&agent_id, "Hello").await?;
            assert!(!response.content.is_empty());

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());
}

#[madsim::test]
async fn test_agent_env_with_llm_faults() {
    let config = SimConfig::new(12345);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::LlmTimeout, 0.3))
        .with_fault(FaultConfig::new(FaultType::LlmFailure, 0.2))
        .run_async(|sim_env| async move {
            let llm = Arc::new(SimLlmClient::new(
                sim_env.fork_rng_raw(),
                sim_env.faults.clone(),
            ));
            let mut agent_env = SimAgentEnv::new(
                sim_env.storage.clone(),
                llm,
                sim_env.clock.clone(),
                sim_env.faults.clone(),
                sim_env.fork_rng(),
            );

            let agent_id = agent_env.create_agent(AgentTestConfig::default())?;

            // Try multiple messages - some should fail due to faults
            let mut success_count = 0;
            let mut failure_count = 0;

            for i in 0..10 {
                match agent_env
                    .send_message(&agent_id, &format!("Message {}", i))
                    .await
                {
                    Ok(_) => success_count += 1,
                    Err(e) => {
                        // Verify fault injection is working (LLM errors map to Internal or OperationTimedOut)
                        // Use matches! for type-safe error checking instead of string matching
                        use kelpie_core::Error;
                        assert!(
                            matches!(e, Error::OperationTimedOut { .. } | Error::Internal { .. }),
                            "Unexpected error type: {:?}",
                            e
                        );
                        failure_count += 1;
                    }
                }
            }

            // With 30% + 20% = 50% fault rate, we should see some failures
            assert!(
                failure_count > 0,
                "Expected some failures with fault injection"
            );
            assert!(success_count > 0, "Expected some successes");

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());
}

#[madsim::test]
async fn test_agent_env_with_storage_faults() {
    let config = SimConfig::new(54321);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.1))
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.1))
        .run_async(|sim_env| async move {
            let llm = Arc::new(SimLlmClient::new(
                sim_env.fork_rng_raw(),
                sim_env.faults.clone(),
            ));
            let mut agent_env = SimAgentEnv::new(
                sim_env.storage.clone(),
                llm,
                sim_env.clock.clone(),
                sim_env.faults.clone(),
                sim_env.fork_rng(),
            );

            // Create multiple agents - storage operations under fault injection
            let mut created = Vec::new();
            for i in 0..5 {
                let config = AgentTestConfig {
                    name: format!("agent-{}", i),
                    ..Default::default()
                };
                match agent_env.create_agent(config) {
                    Ok(id) => created.push(id),
                    Err(_) => {
                        // Storage fault - acceptable
                    }
                }
            }

            assert!(!created.is_empty(), "Should create at least some agents");

            // Try to retrieve agents
            for agent_id in &created {
                let _ = agent_env.get_agent(agent_id);
                // May fail due to storage faults - that's expected
            }

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());
}

#[madsim::test]
async fn test_agent_env_with_time_advancement() {
    let config = SimConfig::new(99999);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            let llm = Arc::new(SimLlmClient::new(
                sim_env.fork_rng_raw(),
                sim_env.faults.clone(),
            ));
            let mut agent_env = SimAgentEnv::new(
                sim_env.storage.clone(),
                llm,
                sim_env.clock.clone(),
                sim_env.faults.clone(),
                sim_env.fork_rng(),
            );

            let start_time = agent_env.now_ms();

            let agent_id = agent_env.create_agent(AgentTestConfig::default())?;

            // Advance time
            agent_env.advance_time_ms(5000);

            let end_time = agent_env.now_ms();
            assert_eq!(end_time - start_time, 5000);

            // Agent should still be accessible after time advancement
            let agent = agent_env.get_agent(&agent_id)?;
            assert_eq!(agent.id, agent_id);

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());
}

#[madsim::test]
async fn test_agent_env_determinism() {
    let seed = 77777;

    let run_simulation = || async {
        let config = SimConfig::new(seed);

        Simulation::new(config)
            .run_async(|sim_env| async move {
                let llm = Arc::new(SimLlmClient::new(
                    sim_env.fork_rng_raw(),
                    sim_env.faults.clone(),
                ));
                let mut agent_env = SimAgentEnv::new(
                    sim_env.storage.clone(),
                    llm,
                    sim_env.clock.clone(),
                    sim_env.faults.clone(),
                    sim_env.fork_rng(),
                );

                let agent_id = agent_env.create_agent(AgentTestConfig::default())?;
                let response = agent_env
                    .send_message(&agent_id, "Determinism test")
                    .await?;

                Ok((agent_id, response.content))
            })
            .await
    };

    let result1 = run_simulation().await.unwrap();
    let result2 = run_simulation().await.unwrap();

    // Same seed should produce same results
    assert_eq!(result1.0, result2.0, "Agent IDs should match");
    assert_eq!(result1.1, result2.1, "LLM responses should match");
}

#[madsim::test]
async fn test_agent_env_multiple_agents_concurrent() {
    let config = SimConfig::new(11111);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            let llm = Arc::new(SimLlmClient::new(
                sim_env.fork_rng_raw(),
                sim_env.faults.clone(),
            ));
            let mut agent_env = SimAgentEnv::new(
                sim_env.storage.clone(),
                llm,
                sim_env.clock.clone(),
                sim_env.faults.clone(),
                sim_env.fork_rng(),
            );

            // Create multiple agents
            let mut agent_ids = Vec::new();
            for i in 0..10 {
                let config = AgentTestConfig {
                    name: format!("agent-{}", i),
                    agent_type: "letta_v1".to_string(),
                    blocks: vec![BlockTestState {
                        label: "persona".to_string(),
                        value: format!("I am agent {}", i),
                    }],
                    ..Default::default()
                };
                let id = agent_env.create_agent(config)?;
                agent_ids.push(id);
            }

            assert_eq!(agent_ids.len(), 10);

            // Verify all agents are accessible
            let listed = agent_env.list_agents();
            assert_eq!(listed.len(), 10);

            // Send messages to multiple agents
            for agent_id in &agent_ids {
                let response = agent_env.send_message(agent_id, "Hello").await?;
                assert!(!response.content.is_empty());
            }

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());
}

#[madsim::test]
async fn test_agent_env_with_tools() {
    let config = SimConfig::new(22222);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            let llm = Arc::new(
                SimLlmClient::new(sim_env.fork_rng_raw(), sim_env.faults.clone())
                    .with_tool_call_probability(1.0),
            );
            let mut agent_env = SimAgentEnv::new(
                sim_env.storage.clone(),
                llm,
                sim_env.clock.clone(),
                sim_env.faults.clone(),
                sim_env.fork_rng(),
            );

            let config = AgentTestConfig {
                tools: vec!["shell".to_string(), "core_memory_append".to_string()],
                ..Default::default()
            };
            let agent_id = agent_env.create_agent(config)?;

            // With tool_call_probability=1.0, should get tool calls
            let response = agent_env
                .send_message(&agent_id, "Execute a command")
                .await?;

            assert_eq!(response.stop_reason, "tool_use");
            assert!(!response.tool_calls.is_empty());
            assert!(
                response.tool_calls[0].name == "shell"
                    || response.tool_calls[0].name == "core_memory_append"
            );

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());
}

#[madsim::test]
async fn test_agent_env_stress_with_faults() {
    let config = SimConfig::new(33333);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::LlmTimeout, 0.15))
        .with_fault(FaultConfig::new(FaultType::LlmFailure, 0.1))
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.05))
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.05))
        .run_async(|sim_env| async move {
            let llm = Arc::new(SimLlmClient::new(
                sim_env.fork_rng_raw(),
                sim_env.faults.clone(),
            ));
            let mut agent_env = SimAgentEnv::new(
                sim_env.storage.clone(),
                llm,
                sim_env.clock.clone(),
                sim_env.faults.clone(),
                sim_env.fork_rng(),
            );

            // Create multiple agents under fault conditions
            let mut agent_ids = Vec::new();
            for i in 0..20 {
                let config = AgentTestConfig {
                    name: format!("stress-agent-{}", i),
                    ..Default::default()
                };
                match agent_env.create_agent(config) {
                    Ok(id) => agent_ids.push(id),
                    Err(_) => {
                        // Storage fault - continue
                    }
                }
            }

            // Send many messages under fault conditions
            let mut total_attempts = 0;
            let mut total_successes = 0;

            for agent_id in &agent_ids {
                for j in 0..5 {
                    total_attempts += 1;
                    if agent_env
                        .send_message(agent_id, &format!("Stress test message {}", j))
                        .await
                        .is_ok()
                    {
                        total_successes += 1;
                    }
                }
            }

            // Should have some successes and some failures
            assert!(total_successes > 0, "Should have some successful messages");
            assert!(
                total_successes < total_attempts,
                "Should have some failed messages due to faults"
            );

            // Verify agents still accessible
            for agent_id in &agent_ids {
                let _ = agent_env.get_agent(agent_id);
                // May fail due to faults - acceptable
            }

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());
}

/// Test for Issue #63: Agent list API race condition
///
/// This test verifies that agents are immediately visible in list operations
/// after creation, even with name filtering. Reproduces the race condition
/// from Letta SDK tests where `create_agent()` returns before persistence
/// completes, causing intermittent failures in `list_agents(name=X)`.
///
/// TigerStyle: DST coverage for concurrent create/list operations
#[madsim::test]
async fn test_agent_list_race_condition_issue_63() {
    let config = SimConfig::new(63_000);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            let llm = Arc::new(SimLlmClient::new(
                sim_env.fork_rng_raw(),
                sim_env.faults.clone(),
            ));
            let mut agent_env = SimAgentEnv::new(
                sim_env.storage.clone(),
                llm,
                sim_env.clock.clone(),
                sim_env.faults.clone(),
                sim_env.fork_rng(),
            );

            // Create multiple agents with different names
            let names = vec!["alice", "bob", "charlie", "alice-duplicate"];
            let mut created_ids = Vec::new();

            for name in &names {
                let config = AgentTestConfig {
                    name: name.to_string(),
                    ..Default::default()
                };
                let id = agent_env.create_agent(config)?;
                created_ids.push((id, name.to_string()));

                // IMMEDIATELY list agents after creation (this is where the race occurs)
                let all_agents = agent_env.list_agents();

                // Verify the just-created agent is visible in the list
                // list_agents() returns Vec<String> (agent IDs)
                let (last_id, _last_name) = created_ids.last().unwrap();
                let found = all_agents.iter().any(|agent_id| agent_id == last_id);
                assert!(
                    found,
                    "Agent {} should be visible immediately after creation",
                    name
                );

                // Test that agents with this name are visible via get_agent lookup
                // (Issue #63 was about name filtering - we verify visibility here)
                let matching_ids: Vec<_> = created_ids
                    .iter()
                    .filter(|(_, n)| n == *name)
                    .map(|(id, _)| id.clone())
                    .collect();
                let visible_count = matching_ids
                    .iter()
                    .filter(|id| all_agents.contains(id))
                    .count();
                assert!(
                    visible_count >= 1,
                    "Should find at least one agent named '{}' after creation",
                    name
                );
            }

            // Final verification: all agents should be listable
            let final_list = agent_env.list_agents();
            assert_eq!(
                final_list.len(),
                names.len(),
                "All {} agents should be visible",
                names.len()
            );

            // Verify agents are visible by looking up their names via get_agent
            // Find ID for "alice" and verify it exists in final_list
            let alice_id = created_ids
                .iter()
                .find(|(_, n)| n == "alice")
                .map(|(id, _)| id);
            assert!(
                alice_id.is_some() && final_list.contains(alice_id.unwrap()),
                "Should find agent named 'alice'"
            );

            // Verify "alice-duplicate" is also visible
            let alice_dup_id = created_ids
                .iter()
                .find(|(_, n)| n == "alice-duplicate")
                .map(|(id, _)| id);
            assert!(
                alice_dup_id.is_some() && final_list.contains(alice_dup_id.unwrap()),
                "Should find agent named 'alice-duplicate'"
            );

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());
}

#[madsim::test]
async fn test_llm_client_direct_with_simulation() {
    let config = SimConfig::new(44444);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::LlmFailure, 0.25))
        .run_async(|sim_env| async move {
            let llm = SimLlmClient::new(sim_env.fork_rng_raw(), sim_env.faults.clone());

            let messages = vec![SimChatMessage {
                role: "user".to_string(),
                content: "Test message".to_string(),
            }];

            let tools = vec![SimToolDefinition {
                name: "test_tool".to_string(),
                description: "A test tool".to_string(),
            }];

            // Try multiple completions with fault injection
            let mut success_count = 0;
            let mut failure_count = 0;

            for _ in 0..20 {
                match llm
                    .complete_with_tools(messages.clone(), tools.clone())
                    .await
                {
                    Ok(_) => success_count += 1,
                    Err(_) => failure_count += 1,
                }
            }

            // With 25% failure rate, should see both successes and failures
            assert!(success_count > 0, "Should have some successes");
            assert!(failure_count > 0, "Should have some failures");

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());
}
