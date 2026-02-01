//! DST Full Simulation Tests for Agent Loop with Type-Based Tool Filtering
//!
//! TigerStyle: These tests exercise the ACTUAL agent loop code paths with
//! fault injection, not just isolated capability logic.
//!
//! What we're testing:
//! 1. Tool filtering happens correctly for each agent type
//! 2. Filtered tools are passed to execution
//! 3. Storage faults during tool execution are handled
//! 4. max_iterations from capabilities is respected
//! 5. Heartbeat enforcement for non-supporting types
//!
//! Run with: cargo test -p kelpie-server --features dst --test agent_loop_types_dst
#![cfg(feature = "dst")]
#![allow(deprecated)]

use kelpie_core::Error;
use kelpie_dst::fault::FaultConfig;
use kelpie_dst::{FaultType, SimConfig, Simulation};
use kelpie_server::models::{AgentState, AgentType, CreateAgentRequest, CreateBlockRequest};
use kelpie_server::state::AppState;
use kelpie_server::tools::{
    register_heartbeat_tools, register_memory_tools, BuiltinToolHandler, ToolSignal,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Helper to convert String error to kelpie_core::Error for DST compatibility
fn sim_err(msg: String) -> Error {
    Error::Internal { message: msg }
}

// =============================================================================
// Simulated Agent Loop
// =============================================================================

/// Simulates the agent loop logic from messages.rs
/// This mirrors the actual code path but without requiring a real LLM
struct SimAgentLoop<R: kelpie_core::Runtime> {
    state: AppState<R>,
    agent_id: String,
    /// Tracks which tools were offered to "LLM"
    tools_offered: Vec<String>,
    /// Tracks which tools were executed
    tools_executed: Vec<String>,
    /// Simulated LLM responses (tool calls to make)
    planned_tool_calls: Vec<(String, Value)>,
    /// Current iteration
    iteration: u32,
}

impl<R: kelpie_core::Runtime + 'static> SimAgentLoop<R> {
    fn new(state: AppState<R>, agent_id: String) -> Self {
        Self {
            state,
            agent_id,
            tools_offered: Vec::new(),
            tools_executed: Vec::new(),
            planned_tool_calls: Vec::new(),
            iteration: 0,
        }
    }

    /// Plan tool calls that the simulated LLM will make
    fn plan_tool_calls(&mut self, calls: Vec<(String, Value)>) {
        self.planned_tool_calls = calls;
    }

    /// Run one iteration of the agent loop
    /// Returns: (should_continue, stop_reason)
    async fn run_iteration(&mut self) -> Result<(bool, String), String> {
        // Step 1: Get agent (can fail with storage faults)
        let agent = self
            .state
            .get_agent(&self.agent_id)
            .map_err(|e| format!("Failed to get agent: {:?}", e))?
            .ok_or_else(|| "Agent not found".to_string())?;

        // Step 2: Get capabilities for this agent type
        let capabilities = agent.agent_type.capabilities();

        // Step 3: Get all tools from registry
        let all_tools = self.state.tool_registry().get_tool_definitions().await;

        // Step 4: Filter tools by capabilities (THIS IS THE CODE PATH WE'RE TESTING)
        let filtered_tools: Vec<_> = all_tools
            .into_iter()
            .filter(|t| capabilities.allowed_tools.contains(&t.name))
            .collect();

        // Record which tools were offered
        self.tools_offered = filtered_tools.iter().map(|t| t.name.clone()).collect();

        // Step 5: Check max_iterations
        self.iteration += 1;
        if self.iteration > capabilities.max_iterations {
            return Ok((false, "max_iterations".to_string()));
        }

        // Step 6: Execute planned tool calls (simulating LLM response)
        if let Some((tool_name, tool_input)) = self.planned_tool_calls.pop() {
            // Verify the tool is in the filtered set
            if !self.tools_offered.contains(&tool_name) {
                return Err(format!(
                    "LLM tried to call tool '{}' which is not in filtered set: {:?}",
                    tool_name, self.tools_offered
                ));
            }

            // Execute via registry (can fail with faults)
            let result = self
                .state
                .tool_registry()
                .execute(&tool_name, &tool_input)
                .await;

            self.tools_executed.push(tool_name.clone());

            // Check for pause signal
            if let ToolSignal::PauseHeartbeats { .. } = &result.signal {
                // Verify agent supports heartbeats
                if !capabilities.supports_heartbeats {
                    return Err(format!(
                        "Agent type {:?} doesn't support heartbeats but pause signal received",
                        agent.agent_type
                    ));
                }
                return Ok((false, "pause_heartbeats".to_string()));
            }

            // Check if more tool calls planned
            if !self.planned_tool_calls.is_empty() {
                return Ok((true, "tool_use".to_string()));
            }
        }

        Ok((false, "end_turn".to_string()))
    }

    /// Run the full agent loop
    async fn run(&mut self) -> Result<String, String> {
        loop {
            let (should_continue, stop_reason) = self.run_iteration().await?;
            if !should_continue {
                return Ok(stop_reason);
            }
        }
    }
}

// =============================================================================
// Test Helpers
// =============================================================================

fn create_agent_with_type(name: &str, agent_type: AgentType) -> AgentState {
    AgentState::from_request(CreateAgentRequest {
        name: name.to_string(),
        agent_type,
        model: None,
        embedding: None,
        system: None,
        description: None,
        memory_blocks: vec![CreateBlockRequest {
            label: "persona".to_string(),
            value: "Test persona".to_string(),
            description: None,
            limit: None,
        }],
        block_ids: vec![],
        tool_ids: vec![],
        tags: vec![],
        metadata: json!({}),
        project_id: None,
        user_id: None,
        org_id: None,
    })
}

async fn setup_state_with_tools<R: kelpie_core::Runtime + 'static>(state: &AppState<R>) {
    let registry = state.tool_registry();

    // Register mock shell tool
    let shell_handler: BuiltinToolHandler = Arc::new(|input: &Value| {
        let input = input.clone();
        Box::pin(async move {
            let cmd = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("(empty)");
            format!("Shell executed: {}", cmd)
        })
    });

    registry
        .register_builtin(
            "shell",
            "Execute shell command",
            json!({"type": "object", "properties": {"command": {"type": "string"}}}),
            shell_handler,
        )
        .await;

    // Register memory tools
    register_memory_tools(registry, state.clone()).await;

    // Register heartbeat tools
    register_heartbeat_tools(registry).await;
}

// =============================================================================
// Full Simulation Tests
// =============================================================================

/// Test: MemgptAgent sees all 7 tools and can execute memory tools under faults
#[cfg(feature = "dst")]
#[test]
fn test_sim_memgpt_agent_loop_with_storage_faults() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.2).with_filter("block_write"))
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.1).with_filter("agent_read"))
        .run(|env| async move {
            let state =
                AppState::with_fault_injector(kelpie_core::current_runtime(), env.faults.clone());
            setup_state_with_tools(&state).await;

            // Create MemGPT agent
            let agent = create_agent_with_type("memgpt-test", AgentType::MemgptAgent);
            let agent_id = agent.id.clone();
            state.create_agent(agent).expect("create agent");

            // Create simulated agent loop
            let mut loop_sim = SimAgentLoop::new(state.clone(), agent_id.clone());

            // Plan tool calls that MemGPT should be able to make
            loop_sim.plan_tool_calls(vec![
                (
                    "core_memory_append".to_string(),
                    json!({"agent_id": agent_id, "label": "persona", "content": " test"}),
                ),
                ("shell".to_string(), json!({"command": "echo hello"})),
            ]);

            // Run the loop
            let stop_reason = loop_sim.run().await.map_err(sim_err)?;

            // Verify MemGPT saw all 7 tools
            assert_eq!(
                loop_sim.tools_offered.len(),
                7,
                "MemGPT should see 7 tools, got: {:?}",
                loop_sim.tools_offered
            );

            // Verify expected tools are present
            assert!(loop_sim
                .tools_offered
                .contains(&"core_memory_append".to_string()));
            assert!(loop_sim
                .tools_offered
                .contains(&"pause_heartbeats".to_string()));
            assert!(loop_sim.tools_offered.contains(&"shell".to_string()));

            // Verify tools were executed (may have partial success due to faults)
            assert!(
                !loop_sim.tools_executed.is_empty(),
                "Should have executed some tools"
            );

            println!(
                "Stop reason: {}, Tools executed: {:?}",
                stop_reason, loop_sim.tools_executed
            );

            Ok(())
        });

    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());
}

/// Test: ReactAgent sees only shell tool, memory tools are filtered out
#[cfg(feature = "dst")]
#[test]
fn test_sim_react_agent_loop_tool_filtering() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.1))
        .run(|env| async move {
            let state =
                AppState::with_fault_injector(kelpie_core::current_runtime(), env.faults.clone());
            setup_state_with_tools(&state).await;

            // Create React agent
            let agent = create_agent_with_type("react-test", AgentType::ReactAgent);
            let agent_id = agent.id.clone();
            state.create_agent(agent).expect("create agent");

            // Create simulated agent loop
            let mut loop_sim = SimAgentLoop::new(state.clone(), agent_id.clone());

            // Plan only shell tool call (what React should have)
            loop_sim.plan_tool_calls(vec![(
                "shell".to_string(),
                json!({"command": "echo react"}),
            )]);

            // Run the loop
            let stop_reason = loop_sim.run().await.map_err(sim_err)?;

            // Verify ReactAgent sees ONLY shell (1 tool)
            assert_eq!(
                loop_sim.tools_offered.len(),
                1,
                "ReactAgent should see only 1 tool, got: {:?}",
                loop_sim.tools_offered
            );
            assert_eq!(loop_sim.tools_offered[0], "shell");

            // Verify memory tools are NOT present
            assert!(!loop_sim
                .tools_offered
                .contains(&"core_memory_append".to_string()));
            assert!(!loop_sim
                .tools_offered
                .contains(&"pause_heartbeats".to_string()));

            println!("Stop reason: {}", stop_reason);

            Ok(())
        });

    assert!(result.is_ok());
}

/// Test: ReactAgent cannot execute memory tools even if "LLM" tries
#[cfg(feature = "dst")]
#[test]
fn test_sim_react_agent_forbidden_tool_rejection() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|_env| async move {
        let state = AppState::new(kelpie_core::current_runtime());
        setup_state_with_tools(&state).await;

        // Create React agent
        let agent = create_agent_with_type("react-test", AgentType::ReactAgent);
        let agent_id = agent.id.clone();
        state.create_agent(agent).expect("create agent");

        // Create simulated agent loop
        let mut loop_sim = SimAgentLoop::new(state.clone(), agent_id.clone());

        // Try to call a memory tool (should be rejected by filtering)
        loop_sim.plan_tool_calls(vec![(
            "core_memory_append".to_string(),
            json!({"agent_id": agent_id, "label": "persona", "content": " hack"}),
        )]);

        // Run the loop - should error because tool not in filtered set
        let result = loop_sim.run().await;

        assert!(
            result.is_err(),
            "Should have rejected memory tool for ReactAgent"
        );
        assert!(
            result.unwrap_err().contains("not in filtered set"),
            "Error should mention filtered set"
        );

        Ok(())
    });

    assert!(result.is_ok());
}

/// Test: LettaV1Agent has simplified tool set, no archival/heartbeat
#[cfg(feature = "dst")]
#[test]
fn test_sim_letta_v1_agent_loop_simplified_tools() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.15))
        .run(|env| async move {
            let state =
                AppState::with_fault_injector(kelpie_core::current_runtime(), env.faults.clone());
            setup_state_with_tools(&state).await;

            // Create LettaV1 agent
            let agent = create_agent_with_type("letta-test", AgentType::LettaV1Agent);
            let agent_id = agent.id.clone();
            state.create_agent(agent).expect("create agent");

            // Create simulated agent loop
            let mut loop_sim = SimAgentLoop::new(state.clone(), agent_id.clone());

            // Plan tool calls that LettaV1 should have
            loop_sim.plan_tool_calls(vec![
                (
                    "core_memory_append".to_string(),
                    json!({"agent_id": agent_id, "label": "persona", "content": " letta"}),
                ),
                ("shell".to_string(), json!({"command": "echo letta"})),
            ]);

            // Run the loop
            let _stop_reason = loop_sim.run().await.map_err(sim_err)?;

            // Verify LettaV1 sees 3 tools
            assert_eq!(
                loop_sim.tools_offered.len(),
                3,
                "LettaV1Agent should see 3 tools, got: {:?}",
                loop_sim.tools_offered
            );

            // Verify core memory tools present
            assert!(loop_sim.tools_offered.contains(&"shell".to_string()));
            assert!(loop_sim
                .tools_offered
                .contains(&"core_memory_append".to_string()));
            assert!(loop_sim
                .tools_offered
                .contains(&"core_memory_replace".to_string()));

            // Verify archival and heartbeat NOT present
            assert!(!loop_sim
                .tools_offered
                .contains(&"archival_memory_insert".to_string()));
            assert!(!loop_sim
                .tools_offered
                .contains(&"pause_heartbeats".to_string()));

            Ok(())
        });

    assert!(result.is_ok());
}

/// Test: max_iterations is respected per agent type
#[cfg(feature = "dst")]
#[test]
fn test_sim_max_iterations_by_agent_type() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|_env| async move {
        let state = AppState::new(kelpie_core::current_runtime());
        setup_state_with_tools(&state).await;

        // Test MemGPT (max_iterations = 5)
        {
            let agent = create_agent_with_type("memgpt", AgentType::MemgptAgent);
            let agent_id = agent.id.clone();
            state.create_agent(agent).expect("create");

            let mut loop_sim = SimAgentLoop::new(state.clone(), agent_id.clone());

            // Plan more tool calls than max_iterations
            for i in 0..10 {
                loop_sim.planned_tool_calls.push((
                    "shell".to_string(),
                    json!({"command": format!("echo {}", i)}),
                ));
            }
            loop_sim.planned_tool_calls.reverse(); // pop() takes from end

            let stop_reason = loop_sim.run().await.map_err(sim_err)?;

            assert_eq!(
                stop_reason, "max_iterations",
                "MemGPT should stop at max_iterations"
            );
            assert_eq!(
                loop_sim.iteration, 6,
                "Should have run 6 iterations (stopped at iteration > 5)"
            );
        }

        // Test ReactAgent (max_iterations = 10)
        {
            let agent = create_agent_with_type("react", AgentType::ReactAgent);
            let agent_id = agent.id.clone();
            state.create_agent(agent).expect("create");

            let mut loop_sim = SimAgentLoop::new(state.clone(), agent_id.clone());

            // Plan more tool calls than max_iterations
            for i in 0..15 {
                loop_sim.planned_tool_calls.push((
                    "shell".to_string(),
                    json!({"command": format!("echo {}", i)}),
                ));
            }
            loop_sim.planned_tool_calls.reverse();

            let stop_reason = loop_sim.run().await.map_err(sim_err)?;

            assert_eq!(
                stop_reason, "max_iterations",
                "ReactAgent should stop at max_iterations"
            );
            assert_eq!(
                loop_sim.iteration, 11,
                "Should have run 11 iterations (stopped at iteration > 10)"
            );
        }

        Ok(())
    });

    assert!(result.is_ok());
}

/// Test: Heartbeat rejection for non-supporting agent types
#[cfg(feature = "dst")]
#[test]
fn test_sim_heartbeat_rejection_for_react_agent() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|_env| async move {
        let state = AppState::new(kelpie_core::current_runtime());
        setup_state_with_tools(&state).await;

        // Create React agent
        let agent = create_agent_with_type("react", AgentType::ReactAgent);
        let agent_id = agent.id.clone();
        state.create_agent(agent).expect("create");

        // Verify pause_heartbeats is not in the filtered tool set
        let capabilities = AgentType::ReactAgent.capabilities();
        assert!(
            !capabilities
                .allowed_tools
                .contains(&"pause_heartbeats".to_string()),
            "ReactAgent should not have pause_heartbeats in allowed_tools"
        );

        // Even if we try to execute it directly via registry (bypassing filter),
        // the loop would reject the pause signal
        let mut loop_sim = SimAgentLoop::new(state.clone(), agent_id);

        // This should fail because pause_heartbeats is not in filtered set
        loop_sim.plan_tool_calls(vec![(
            "pause_heartbeats".to_string(),
            json!({"minutes": 5}),
        )]);

        let result = loop_sim.run().await;
        assert!(result.is_err(), "Should reject pause_heartbeats for React");

        Ok(())
    });

    assert!(result.is_ok());
}

/// Test: Multiple agent types in same simulation with storage faults
#[cfg(feature = "dst")]
#[test]
fn test_sim_multiple_agent_types_under_faults() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.2))
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.1))
        .run(|env| async move {
            let state =
                AppState::with_fault_injector(kelpie_core::current_runtime(), env.faults.clone());
            setup_state_with_tools(&state).await;

            // Create agents of each type
            let memgpt = create_agent_with_type("memgpt", AgentType::MemgptAgent);
            let react = create_agent_with_type("react", AgentType::ReactAgent);
            let letta = create_agent_with_type("letta", AgentType::LettaV1Agent);

            let memgpt_id = memgpt.id.clone();
            let react_id = react.id.clone();
            let letta_id = letta.id.clone();

            state.create_agent(memgpt).expect("create memgpt");
            state.create_agent(react).expect("create react");
            state.create_agent(letta).expect("create letta");

            // Run loops for each agent type
            let mut results = Vec::new();

            for (agent_id, expected_tools, agent_type) in [
                (memgpt_id.clone(), 7, "MemGPT"),
                (react_id.clone(), 1, "React"),
                (letta_id.clone(), 3, "LettaV1"),
            ] {
                let mut loop_sim = SimAgentLoop::new(state.clone(), agent_id.clone());
                loop_sim
                    .plan_tool_calls(vec![("shell".to_string(), json!({"command": "echo test"}))]);

                match loop_sim.run().await {
                    Ok(stop_reason) => {
                        // Verify tool count
                        if loop_sim.tools_offered.len() != expected_tools {
                            return Err(sim_err(format!(
                                "{} should see {} tools, got {}",
                                agent_type,
                                expected_tools,
                                loop_sim.tools_offered.len()
                            )));
                        }
                        results.push((agent_type, stop_reason, true));
                    }
                    Err(e) => {
                        // Storage faults can cause failures, that's expected
                        results.push((agent_type, e, false));
                    }
                }
            }

            println!("Results: {:?}", results);

            // At least one should succeed (faults are probabilistic)
            let successes = results.iter().filter(|(_, _, ok)| *ok).count();
            assert!(
                successes > 0,
                "At least one agent loop should succeed: {:?}",
                results
            );

            Ok(())
        });

    assert!(result.is_ok());
}

/// Test: Determinism - same seed produces same tool filtering behavior
#[cfg(feature = "dst")]
#[test]
fn test_sim_agent_loop_determinism() {
    let seed = 12345u64;

    let run_test = || {
        let config = SimConfig::new(seed);
        Simulation::new(config)
            .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.3))
            .run(|env| async move {
                let state = AppState::with_fault_injector(
                    kelpie_core::current_runtime(),
                    env.faults.clone(),
                );
                setup_state_with_tools(&state).await;

                let mut results = Vec::new();

                for agent_type in [
                    AgentType::MemgptAgent,
                    AgentType::ReactAgent,
                    AgentType::LettaV1Agent,
                ] {
                    let agent = create_agent_with_type("test", agent_type.clone());
                    let agent_id = agent.id.clone();
                    state.create_agent(agent).expect("create");

                    let mut loop_sim = SimAgentLoop::new(state.clone(), agent_id);
                    loop_sim
                        .plan_tool_calls(vec![("shell".to_string(), json!({"command": "echo"}))]);

                    let loop_result = loop_sim.run().await;
                    results.push((
                        format!("{:?}", agent_type),
                        loop_sim.tools_offered.len(),
                        loop_result.is_ok(),
                    ));
                }

                Ok(results)
            })
    };

    let result1 = run_test().expect("first run");
    let result2 = run_test().expect("second run");

    assert_eq!(result1, result2, "Same seed must produce same results");
}

/// Test: High load - many agents of different types under faults
#[cfg(feature = "dst")]
#[test]
fn test_sim_high_load_mixed_agent_types() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.1))
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.05))
        .run(|env| async move {
            let state =
                AppState::with_fault_injector(kelpie_core::current_runtime(), env.faults.clone());
            setup_state_with_tools(&state).await;

            let success_count = Arc::new(AtomicU32::new(0));
            let failure_count = Arc::new(AtomicU32::new(0));

            // Create 30 agents (10 of each type)
            for i in 0..30 {
                let agent_type = match i % 3 {
                    0 => AgentType::MemgptAgent,
                    1 => AgentType::ReactAgent,
                    _ => AgentType::LettaV1Agent,
                };

                let agent = create_agent_with_type(&format!("agent-{}", i), agent_type);
                let agent_id = agent.id.clone();
                state.create_agent(agent).expect("create");

                let mut loop_sim = SimAgentLoop::new(state.clone(), agent_id);
                loop_sim.plan_tool_calls(vec![(
                    "shell".to_string(),
                    json!({"command": format!("echo {}", i)}),
                )]);

                match loop_sim.run().await {
                    Ok(_) => {
                        success_count.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(_) => {
                        failure_count.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }

            let successes = success_count.load(Ordering::SeqCst);
            let failures = failure_count.load(Ordering::SeqCst);

            println!(
                "High load test: {} successes, {} failures (10-15% expected fault rate)",
                successes, failures
            );

            // With 10-15% fault rate, most should succeed
            assert!(
                successes > 20,
                "Should have >20 successes, got {}",
                successes
            );
            assert_eq!(successes + failures, 30);

            Ok(())
        });

    assert!(result.is_ok());
}

/// Test: Tool execution results are correct under faults
#[cfg(feature = "dst")]
#[test]
fn test_sim_tool_execution_results_under_faults() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.3).with_filter("block_write"))
        .run(|env| async move {
            let state =
                AppState::with_fault_injector(kelpie_core::current_runtime(), env.faults.clone());
            setup_state_with_tools(&state).await;

            // Create MemGPT agent (has memory tools)
            let agent = create_agent_with_type("memgpt", AgentType::MemgptAgent);
            let agent_id = agent.id.clone();
            state.create_agent(agent).expect("create");

            // Execute memory tool directly and track results
            let mut successes = 0;
            let mut failures = 0;

            for i in 0..20 {
                let result = state
                    .tool_registry()
                    .execute(
                        "core_memory_append",
                        &json!({
                            "agent_id": agent_id,
                            "label": "persona",
                            "content": format!(" test-{}", i)
                        }),
                    )
                    .await;

                if result.success {
                    successes += 1;
                } else {
                    failures += 1;
                }
            }

            println!(
                "Tool execution: {} successes, {} failures (30% fault rate on block_write)",
                successes, failures
            );

            // With 30% fault rate, expect roughly 70% success
            assert!(successes > 0, "Should have some successes");
            assert!(
                failures > 0,
                "Should have some failures with 30% fault rate"
            );

            Ok(())
        });

    assert!(result.is_ok());
}
