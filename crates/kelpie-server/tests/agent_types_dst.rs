//! DST Tests for Agent Types Abstraction
//!
//! TigerStyle: Tests written BEFORE implementation (DST-first approach).
//! These tests define the expected behavior for agent type capabilities.
//!
//! Run with: cargo test -p kelpie-server --features dst --test agent_types_dst
#![cfg(feature = "dst")]
#![allow(deprecated)]

use kelpie_dst::fault::FaultConfig;
use kelpie_dst::{FaultType, SimConfig, Simulation};
use kelpie_server::models::{AgentState, AgentType, CreateAgentRequest, CreateBlockRequest};
use kelpie_server::state::AppState;
use kelpie_server::tools::{register_heartbeat_tools, register_memory_tools, BuiltinToolHandler};
use serde_json::{json, Value};
use std::sync::Arc;

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

    // Register mock shell tool (real shell uses sandbox which isn't available in tests)
    let shell_handler: BuiltinToolHandler = Arc::new(|input: &Value| {
        let input = input.clone();
        Box::pin(async move {
            let cmd = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("(empty)");
            format!("Mock shell executed: {}", cmd)
        })
    });

    registry
        .register_builtin(
            "shell",
            "Execute a shell command (mock for testing)",
            json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                },
                "required": ["command"]
            }),
            shell_handler,
        )
        .await;

    // Register memory tools
    register_memory_tools(registry, state.clone()).await;

    // Register heartbeat tools
    register_heartbeat_tools(registry).await;
}

// =============================================================================
// Phase 1: Capability Correctness Tests
// =============================================================================

/// Test: MemgptAgent has all expected tools
#[cfg(feature = "dst")]
#[test]
fn test_memgpt_agent_capabilities() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|_env| async move {
        let capabilities = AgentType::MemgptAgent.capabilities();

        // MemGPT should have all memory tools + heartbeat + shell
        let expected_tools = vec![
            "shell",
            "core_memory_append",
            "core_memory_replace",
            "archival_memory_insert",
            "archival_memory_search",
            "conversation_search",
            "pause_heartbeats",
        ];

        assert_eq!(
            capabilities.allowed_tools.len(),
            expected_tools.len(),
            "MemgptAgent should have {} tools, got {}",
            expected_tools.len(),
            capabilities.allowed_tools.len()
        );

        for tool in &expected_tools {
            assert!(
                capabilities.allowed_tools.contains(&tool.to_string()),
                "MemgptAgent should have tool: {}",
                tool
            );
        }

        assert!(
            capabilities.supports_heartbeats,
            "MemgptAgent should support heartbeats"
        );
        assert_eq!(capabilities.max_iterations, 5);

        Ok(())
    });

    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());
}

/// Test: ReactAgent has only shell tool
#[cfg(feature = "dst")]
#[test]
fn test_react_agent_capabilities() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|_env| async move {
        let capabilities = AgentType::ReactAgent.capabilities();

        // ReactAgent should only have shell
        assert_eq!(
            capabilities.allowed_tools.len(),
            1,
            "ReactAgent should have only 1 tool (shell)"
        );
        assert!(
            capabilities.allowed_tools.contains(&"shell".to_string()),
            "ReactAgent should have shell tool"
        );

        // ReactAgent should NOT have memory tools
        assert!(
            !capabilities
                .allowed_tools
                .contains(&"core_memory_append".to_string()),
            "ReactAgent should NOT have core_memory_append"
        );

        assert!(
            !capabilities.supports_heartbeats,
            "ReactAgent should NOT support heartbeats"
        );

        // ReactAgent may need more iterations for ReAct pattern
        assert!(capabilities.max_iterations >= 5);

        Ok(())
    });

    assert!(result.is_ok());
}

/// Test: LettaV1Agent has simplified tool set
#[cfg(feature = "dst")]
#[test]
fn test_letta_v1_agent_capabilities() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|_env| async move {
        let capabilities = AgentType::LettaV1Agent.capabilities();

        // LettaV1 has shell + core memory tools (no archival, no heartbeat)
        let expected_tools = vec!["shell", "core_memory_append", "core_memory_replace"];

        assert_eq!(
            capabilities.allowed_tools.len(),
            expected_tools.len(),
            "LettaV1Agent should have {} tools",
            expected_tools.len()
        );

        for tool in &expected_tools {
            assert!(
                capabilities.allowed_tools.contains(&tool.to_string()),
                "LettaV1Agent should have tool: {}",
                tool
            );
        }

        // LettaV1 should NOT have archival or heartbeat
        assert!(
            !capabilities
                .allowed_tools
                .contains(&"archival_memory_insert".to_string()),
            "LettaV1Agent should NOT have archival_memory_insert"
        );
        assert!(
            !capabilities
                .allowed_tools
                .contains(&"pause_heartbeats".to_string()),
            "LettaV1Agent should NOT have pause_heartbeats"
        );

        assert!(
            !capabilities.supports_heartbeats,
            "LettaV1Agent should NOT support heartbeats"
        );

        Ok(())
    });

    assert!(result.is_ok());
}

// =============================================================================
// Phase 2: Tool Filtering Tests
// =============================================================================

/// Test: Tool definitions are filtered by agent type
#[cfg(feature = "dst")]
#[test]
fn test_tool_filtering_memgpt() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|_env| async move {
        let state = AppState::new(kelpie_core::current_runtime());
        setup_state_with_tools(&state).await;

        // Create MemGPT agent
        let agent = create_agent_with_type("test-memgpt", AgentType::MemgptAgent);
        state.create_agent(agent.clone()).expect("create agent");

        // Get filtered tools for this agent type
        let capabilities = agent.agent_type.capabilities();
        let all_tools = state.tool_registry().get_tool_definitions().await;
        let filtered: Vec<_> = all_tools
            .into_iter()
            .filter(|t| capabilities.allowed_tools.contains(&t.name))
            .collect();

        // MemGPT should see all 7 tools
        assert_eq!(filtered.len(), 7, "MemGPT should see 7 tools");

        // Verify specific tools are present
        let tool_names: Vec<_> = filtered.iter().map(|t| t.name.as_str()).collect();
        assert!(tool_names.contains(&"pause_heartbeats"));
        assert!(tool_names.contains(&"core_memory_append"));
        assert!(tool_names.contains(&"archival_memory_search"));

        Ok(())
    });

    assert!(result.is_ok());
}

/// Test: ReactAgent only sees shell tool
#[cfg(feature = "dst")]
#[test]
fn test_tool_filtering_react() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|_env| async move {
        let state = AppState::new(kelpie_core::current_runtime());
        setup_state_with_tools(&state).await;

        // Create React agent
        let agent = create_agent_with_type("test-react", AgentType::ReactAgent);
        state.create_agent(agent.clone()).expect("create agent");

        // Get filtered tools for this agent type
        let capabilities = agent.agent_type.capabilities();
        let all_tools = state.tool_registry().get_tool_definitions().await;
        let filtered: Vec<_> = all_tools
            .into_iter()
            .filter(|t| capabilities.allowed_tools.contains(&t.name))
            .collect();

        // ReactAgent should see only shell
        assert_eq!(
            filtered.len(),
            1,
            "ReactAgent should see only 1 tool, got {}",
            filtered.len()
        );
        assert_eq!(filtered[0].name, "shell");

        Ok(())
    });

    assert!(result.is_ok());
}

// =============================================================================
// Phase 3: Forbidden Tool Rejection Tests
// =============================================================================

/// Test: ReactAgent cannot execute memory tools
#[cfg(feature = "dst")]
#[test]
fn test_forbidden_tool_rejection_react() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|_env| async move {
        let state = AppState::new(kelpie_core::current_runtime());
        setup_state_with_tools(&state).await;

        // Create React agent
        let agent = create_agent_with_type("test-react", AgentType::ReactAgent);
        state.create_agent(agent.clone()).expect("create agent");

        // Get capabilities
        let capabilities = agent.agent_type.capabilities();

        // ReactAgent should NOT be able to call memory tools
        let forbidden_tools = vec![
            "core_memory_append",
            "core_memory_replace",
            "archival_memory_insert",
            "archival_memory_search",
            "conversation_search",
            "pause_heartbeats",
        ];

        for tool_name in forbidden_tools {
            assert!(
                !capabilities.allowed_tools.contains(&tool_name.to_string()),
                "ReactAgent should NOT have access to {}",
                tool_name
            );

            // If we try to execute via registry, it should work (tool exists)
            // but the filtering should have prevented it from being offered
            let tool_exists = state.tool_registry().has_tool(tool_name).await;
            assert!(tool_exists, "Tool {} should exist in registry", tool_name);

            // The key assertion: capabilities don't include it
            let is_allowed = capabilities.allowed_tools.contains(&tool_name.to_string());
            assert!(
                !is_allowed,
                "Tool {} should NOT be in ReactAgent's allowed tools",
                tool_name
            );
        }

        Ok(())
    });

    assert!(result.is_ok());
}

/// Test: LettaV1Agent cannot execute archival or heartbeat tools
#[cfg(feature = "dst")]
#[test]
fn test_forbidden_tool_rejection_letta_v1() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|_env| async move {
        let state = AppState::new(kelpie_core::current_runtime());
        setup_state_with_tools(&state).await;

        let agent = create_agent_with_type("test-letta", AgentType::LettaV1Agent);
        state.create_agent(agent.clone()).expect("create agent");

        let capabilities = agent.agent_type.capabilities();

        // LettaV1 should NOT have archival or heartbeat
        let forbidden_tools = vec![
            "archival_memory_insert",
            "archival_memory_search",
            "conversation_search",
            "pause_heartbeats",
        ];

        for tool_name in forbidden_tools {
            assert!(
                !capabilities.allowed_tools.contains(&tool_name.to_string()),
                "LettaV1Agent should NOT have access to {}",
                tool_name
            );
        }

        // But should have core memory tools
        assert!(capabilities
            .allowed_tools
            .contains(&"core_memory_append".to_string()));
        assert!(capabilities
            .allowed_tools
            .contains(&"core_memory_replace".to_string()));

        Ok(())
    });

    assert!(result.is_ok());
}

// =============================================================================
// Phase 4: Heartbeat Enforcement Tests
// =============================================================================

/// Test: Only MemgptAgent supports heartbeats
#[cfg(feature = "dst")]
#[test]
fn test_heartbeat_support_by_type() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|_env| async move {
        // MemGPT supports heartbeats
        let memgpt_caps = AgentType::MemgptAgent.capabilities();
        assert!(
            memgpt_caps.supports_heartbeats,
            "MemgptAgent must support heartbeats"
        );
        assert!(
            memgpt_caps
                .allowed_tools
                .contains(&"pause_heartbeats".to_string()),
            "MemgptAgent must have pause_heartbeats tool"
        );

        // ReactAgent does NOT support heartbeats
        let react_caps = AgentType::ReactAgent.capabilities();
        assert!(
            !react_caps.supports_heartbeats,
            "ReactAgent must NOT support heartbeats"
        );
        assert!(
            !react_caps
                .allowed_tools
                .contains(&"pause_heartbeats".to_string()),
            "ReactAgent must NOT have pause_heartbeats tool"
        );

        // LettaV1 does NOT support heartbeats
        let letta_caps = AgentType::LettaV1Agent.capabilities();
        assert!(
            !letta_caps.supports_heartbeats,
            "LettaV1Agent must NOT support heartbeats"
        );
        assert!(
            !letta_caps
                .allowed_tools
                .contains(&"pause_heartbeats".to_string()),
            "LettaV1Agent must NOT have pause_heartbeats tool"
        );

        Ok(())
    });

    assert!(result.is_ok());
}

// =============================================================================
// Phase 5: Memory Tools Under Faults Tests
// =============================================================================

/// Test: MemgptAgent handles storage faults gracefully
#[cfg(feature = "dst")]
#[test]
fn test_memgpt_memory_tools_under_faults() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.3).with_filter("block_write"))
        .run(|env| async move {
            let state =
                AppState::with_fault_injector(kelpie_core::current_runtime(), env.faults.clone());
            register_memory_tools(state.tool_registry(), state.clone()).await;

            // Create MemGPT agent
            let agent = create_agent_with_type("test-memgpt", AgentType::MemgptAgent);
            let agent_id = agent.id.clone();
            state.create_agent(agent).expect("create agent");

            // MemGPT should be able to use memory tools (with some failures due to faults)
            let capabilities = AgentType::MemgptAgent.capabilities();
            assert!(capabilities
                .allowed_tools
                .contains(&"core_memory_append".to_string()));

            // Try memory operations - some will fail due to fault injection
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
                            "content": format!(" Test content {}", i)
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
                "Memory operations: {} successes, {} failures (30% fault rate)",
                successes, failures
            );

            // With 30% fault rate, expect some failures
            assert!(successes > 0, "Should have some successful operations");
            // Note: failures might be 0 if faults don't hit this operation
            assert_eq!(successes + failures, 20);

            Ok(())
        });

    assert!(result.is_ok());
}

// =============================================================================
// Phase 6: Type Isolation Tests
// =============================================================================

/// Test: Different agent types don't affect each other
#[cfg(feature = "dst")]
#[test]
fn test_agent_type_isolation() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|_env| async move {
        let state = AppState::new(kelpie_core::current_runtime());
        setup_state_with_tools(&state).await;

        // Create agents of each type
        let memgpt = create_agent_with_type("agent-memgpt", AgentType::MemgptAgent);
        let react = create_agent_with_type("agent-react", AgentType::ReactAgent);
        let letta = create_agent_with_type("agent-letta", AgentType::LettaV1Agent);

        state.create_agent(memgpt.clone()).expect("create memgpt");
        state.create_agent(react.clone()).expect("create react");
        state.create_agent(letta.clone()).expect("create letta");

        // Verify each agent has correct type
        let retrieved_memgpt = state
            .get_agent(&memgpt.id)
            .expect("get memgpt")
            .expect("memgpt exists");
        let retrieved_react = state
            .get_agent(&react.id)
            .expect("get react")
            .expect("react exists");
        let retrieved_letta = state
            .get_agent(&letta.id)
            .expect("get letta")
            .expect("letta exists");

        assert_eq!(retrieved_memgpt.agent_type, AgentType::MemgptAgent);
        assert_eq!(retrieved_react.agent_type, AgentType::ReactAgent);
        assert_eq!(retrieved_letta.agent_type, AgentType::LettaV1Agent);

        // Capabilities should be independent
        let memgpt_caps = retrieved_memgpt.agent_type.capabilities();
        let react_caps = retrieved_react.agent_type.capabilities();

        // MemGPT has more tools
        assert!(memgpt_caps.allowed_tools.len() > react_caps.allowed_tools.len());

        // Creating one type doesn't affect another's capabilities
        assert_eq!(memgpt_caps.allowed_tools.len(), 7);
        assert_eq!(react_caps.allowed_tools.len(), 1);

        Ok(())
    });

    assert!(result.is_ok());
}

// =============================================================================
// Phase 7: Determinism Tests
// =============================================================================

/// Test: Same seed produces same capability behavior
#[cfg(feature = "dst")]
#[test]
fn test_agent_types_determinism() {
    let seed = 42u64;

    let run_test = || {
        let config = SimConfig::new(seed);
        Simulation::new(config)
            .with_fault(
                FaultConfig::new(FaultType::StorageWriteFail, 0.5).with_filter("block_write"),
            )
            .run(|env| async move {
                let state = AppState::with_fault_injector(
                    kelpie_core::current_runtime(),
                    env.faults.clone(),
                );
                register_memory_tools(state.tool_registry(), state.clone()).await;

                // Create all agent types
                let memgpt = create_agent_with_type("memgpt", AgentType::MemgptAgent);
                let react = create_agent_with_type("react", AgentType::ReactAgent);
                let letta = create_agent_with_type("letta", AgentType::LettaV1Agent);

                state.create_agent(memgpt.clone()).expect("create");
                state.create_agent(react.clone()).expect("create");
                state.create_agent(letta.clone()).expect("create");

                // Collect capability info
                let mut results = Vec::new();

                for agent_type in [
                    AgentType::MemgptAgent,
                    AgentType::ReactAgent,
                    AgentType::LettaV1Agent,
                ] {
                    let caps = agent_type.capabilities();
                    results.push((
                        caps.allowed_tools.len(),
                        caps.supports_heartbeats,
                        caps.max_iterations,
                    ));
                }

                // Try some operations that involve fault injection
                let mut operation_results = Vec::new();
                for i in 0..10 {
                    let result = state
                        .tool_registry()
                        .execute(
                            "core_memory_append",
                            &json!({
                                "agent_id": memgpt.id,
                                "label": "persona",
                                "content": format!(" {}", i)
                            }),
                        )
                        .await;
                    operation_results.push(result.success);
                }

                Ok((results, operation_results))
            })
    };

    let result1 = run_test().expect("first run");
    let result2 = run_test().expect("second run");

    assert_eq!(result1, result2, "Same seed must produce identical results");
}

/// Test: All agent types are valid (no panics)
#[cfg(feature = "dst")]
#[test]
fn test_all_agent_types_valid() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|_env| async move {
        let all_types = vec![
            AgentType::MemgptAgent,
            AgentType::ReactAgent,
            AgentType::LettaV1Agent,
        ];

        for agent_type in all_types {
            // Capabilities should not panic
            let caps = agent_type.capabilities();

            // Every type should have at least shell
            assert!(
                caps.allowed_tools.contains(&"shell".to_string()),
                "{:?} should have shell tool",
                agent_type
            );

            // max_iterations should be reasonable
            assert!(
                caps.max_iterations >= 1 && caps.max_iterations <= 100,
                "{:?} has invalid max_iterations: {}",
                agent_type,
                caps.max_iterations
            );

            // allowed_tools should not be empty
            assert!(
                !caps.allowed_tools.is_empty(),
                "{:?} should have at least one tool",
                agent_type
            );
        }

        Ok(())
    });

    assert!(result.is_ok());
}

// =============================================================================
// Phase 8: Edge Cases
// =============================================================================

/// Test: Default agent type is MemgptAgent
#[cfg(feature = "dst")]
#[test]
fn test_default_agent_type() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|_env| async move {
        let default_type: AgentType = Default::default();
        assert!(
            matches!(default_type, AgentType::MemgptAgent),
            "Default agent type should be MemgptAgent"
        );

        // Default type should have full capabilities
        let caps = default_type.capabilities();
        assert!(caps.supports_heartbeats);
        assert_eq!(caps.allowed_tools.len(), 7);

        Ok(())
    });

    assert!(result.is_ok());
}

/// Test: Tool count consistency across types
#[cfg(feature = "dst")]
#[test]
fn test_tool_count_hierarchy() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config).run(|_env| async move {
        let memgpt_tools = AgentType::MemgptAgent.capabilities().allowed_tools.len();
        let letta_tools = AgentType::LettaV1Agent.capabilities().allowed_tools.len();
        let react_tools = AgentType::ReactAgent.capabilities().allowed_tools.len();

        // MemGPT has the most tools
        assert!(
            memgpt_tools > letta_tools,
            "MemGPT should have more tools than LettaV1"
        );
        assert!(
            letta_tools > react_tools,
            "LettaV1 should have more tools than React"
        );

        // Specific counts
        assert_eq!(memgpt_tools, 7, "MemGPT should have 7 tools");
        assert_eq!(letta_tools, 3, "LettaV1 should have 3 tools");
        assert_eq!(react_tools, 1, "React should have 1 tool");

        Ok(())
    });

    assert!(result.is_ok());
}
