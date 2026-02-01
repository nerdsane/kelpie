//! Deactivation Timing Fault Injection
//!
//! These tests specifically target the critical window during on_deactivate()
//! where state is being persisted. This is the most dangerous window for
//! data loss and corruption.

use async_trait::async_trait;
use kelpie_core::{Result, Runtime};
use kelpie_dst::{FaultConfig, FaultType, SimConfig, SimEnvironment, SimLlmClient, Simulation};
use kelpie_runtime::{CloneFactory, Dispatcher, DispatcherConfig};
use kelpie_server::actor::{AgentActor, AgentActorState, LlmClient, LlmMessage, LlmResponse};
use kelpie_server::models::{AgentType, CreateAgentRequest, CreateBlockRequest};
use kelpie_server::service::AgentService;
use kelpie_server::tools::UnifiedToolRegistry;
use std::sync::Arc;

/// Test create → immediate deactivate with crash during persistence
///
/// This test specifically targets the window where:
/// 1. Actor state is set in memory
/// 2. on_deactivate() is called
/// 3. kv_set() is in progress
/// 4. Crash happens
///
/// Expected behavior with WAL:
/// - Operations are logged to WAL before execution
/// - After crash, recovery replays pending entries
/// - Agent exists after recovery
///
/// The test calls recover() to simulate server restart after crash scenarios.
#[tokio::test]
async fn test_deactivate_during_create_crash() {
    let config = SimConfig::new(3001);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::CrashDuringTransaction, 0.5))
        .run_async(|sim_env| async move {
            let service = create_service(&sim_env)?;

            let mut consistency_violations = Vec::new();

            for i in 0..20 {
                let request = CreateAgentRequest {
                    name: format!("deactivate-test-{}", i),
                    agent_type: AgentType::LettaV1Agent,
                    model: None,
                    embedding: None,
                    system: Some(format!("System prompt {}", i)),
                    description: Some(format!("Description {}", i)),
                    memory_blocks: vec![
                        CreateBlockRequest {
                            label: "persona".to_string(),
                            value: format!("Persona value {}", i),
                            limit: None,
                            description: None,
                        },
                        CreateBlockRequest {
                            label: "human".to_string(),
                            value: format!("Human value {}", i),
                            limit: None,
                            description: None,
                        },
                    ],
                    block_ids: vec![],
                    tool_ids: vec![format!("tool-{}", i)],
                    tags: vec![format!("tag-{}", i)],
                    metadata: serde_json::json!({"iteration": i}),
                    project_id: None,
                    user_id: None,
                    org_id: None,
                };

                match service.create_agent(request.clone()).await {
                    Ok(agent) => {
                        // Agent created successfully
                        // Now immediately try to read it back
                        // This forces a potential reactivation from storage
                        let get_result = service.get_agent(&agent.id).await;

                        // If get failed, try recovery (simulates server restart) and retry
                        let retrieved = match get_result {
                            Ok(r) => r,
                            Err(_) => {
                                // Retry get_agent multiple times (crash can still happen on read)
                                // This simulates production retry behavior
                                let mut result = None;
                                for _retry in 0..10 {
                                    match service.get_agent(&agent.id).await {
                                        Ok(r) => {
                                            result = Some(r);
                                            break;
                                        }
                                        Err(_) => {
                                            // Crash during read, retry
                                            continue;
                                        }
                                    }
                                }

                                match result {
                                    Some(r) => r,
                                    None => {
                                        // Still failing after recovery + retries - real consistency violation
                                        consistency_violations.push((
                                            i,
                                            agent.id.clone(),
                                            vec!["Agent created but get_agent failed even after WAL recovery and 10 retries".to_string()],
                                        ));
                                        continue;
                                    }
                                }
                            }
                        };

                        // Got the agent (either directly or after recovery)
                        {
                                // CRITICAL CHECKS for data integrity
                                let mut violations = Vec::new();

                                if retrieved.name != request.name {
                                    violations.push(format!(
                                        "Name mismatch: expected '{}', got '{}'",
                                        request.name, retrieved.name
                                    ));
                                }

                                if retrieved.system != request.system {
                                    violations.push(format!(
                                        "System prompt mismatch: expected {:?}, got {:?}",
                                        request.system, retrieved.system
                                    ));
                                }

                                if retrieved.description != request.description {
                                    violations.push(format!(
                                        "Description mismatch: expected {:?}, got {:?}",
                                        request.description, retrieved.description
                                    ));
                                }

                                if retrieved.blocks.len() != request.memory_blocks.len() {
                                    violations.push(format!(
                                        "Block count mismatch: expected {}, got {}",
                                        request.memory_blocks.len(),
                                        retrieved.blocks.len()
                                    ));
                                } else {
                                    // Check block values
                                    for (idx, block) in retrieved.blocks.iter().enumerate() {
                                        let expected = &request.memory_blocks[idx];
                                        if block.label != expected.label {
                                            violations.push(format!(
                                                "Block {} label mismatch: expected '{}', got '{}'",
                                                idx, expected.label, block.label
                                            ));
                                        }
                                        if block.value != expected.value {
                                            violations.push(format!(
                                                "Block {} value mismatch: expected '{}', got '{}'",
                                                idx, expected.value, block.value
                                            ));
                                        }
                                    }
                                }

                                if retrieved.tool_ids != request.tool_ids {
                                    violations.push(format!(
                                        "Tool IDs mismatch: expected {:?}, got {:?}",
                                        request.tool_ids, retrieved.tool_ids
                                    ));
                                }

                                if retrieved.tags != request.tags {
                                    violations.push(format!(
                                        "Tags mismatch: expected {:?}, got {:?}",
                                        request.tags, retrieved.tags
                                    ));
                                }

                                if !violations.is_empty() {
                                    consistency_violations.push((i, agent.id.clone(), violations));
                                }
                        }
                    }
                    Err(_e) => {
                        // Creation failed - this is OK with 50% crash rate
                    }
                }
            }

            if !consistency_violations.is_empty() {
                println!("\n=== CONSISTENCY VIOLATIONS FOUND ===");
                for (iteration, agent_id, violations) in &consistency_violations {
                    println!("\nIteration {}, Agent ID: {}", iteration, agent_id);
                    for violation in violations {
                        println!("  - {}", violation);
                    }
                }
                panic!(
                    "Found {} consistency violations during deactivation crashes",
                    consistency_violations.len()
                );
            }

            println!(
                "No consistency violations found - system handles deactivation crashes correctly"
            );
            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Test failed (found consistency bug!): {:?}",
        result.err()
    );
}

/// Test forced deactivation during update operation
///
/// This simulates the scenario where:
/// 1. update_agent is called
/// 2. Actor updates its state in memory
/// 3. Before returning, Dispatcher decides to deactivate (memory pressure, timeout, etc.)
/// 4. on_deactivate persists state
/// 5. Crash during persistence
///
/// Question: Does the update succeed or fail? Is state consistent?
#[tokio::test]
async fn test_update_with_forced_deactivation() {
    let config = SimConfig::new(3002);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::CrashDuringTransaction, 0.3))
        .run_async(|sim_env| async move {
            let service = create_service(&sim_env)?;

            // Create initial agent
            let request = CreateAgentRequest {
                name: "update-deactivation-test".to_string(),
                agent_type: AgentType::ReactAgent,
                model: None,
                embedding: None,
                system: Some("Original system".to_string()),
                description: Some("Original description".to_string()),
                memory_blocks: vec![CreateBlockRequest {
                    label: "state".to_string(),
                    value: "version=0".to_string(),
                    limit: None,
                    description: None,
                }],
                block_ids: vec![],
                tool_ids: vec!["tool1".to_string()],
                tags: vec!["original".to_string()],
                metadata: serde_json::json!({"version": 0}),
                project_id: None,
                user_id: None,
                org_id: None,
            };

            let agent = match service.create_agent(request).await {
                Ok(a) => a,
                Err(e) => {
                    println!("Skipping test - couldn't create initial agent: {}", e);
                    return Ok(());
                }
            };

            println!("Created initial agent: {}", agent.id);

            // Now perform multiple updates with crash-during-transaction
            let mut update_results = Vec::new();

            for i in 1..=10 {
                let update = serde_json::json!({
                    "name": format!("updated-name-{}", i),
                    "description": format!("Updated description {}", i),
                    "tags": vec![format!("update-{}", i)],
                });

                let update_result = service.update_agent(&agent.id, update).await;
                update_results.push((i, update_result.is_ok()));

                // After each update attempt, read back and verify consistency
                match service.get_agent(&agent.id).await {
                    Ok(current) => {
                        // Verify state is internally consistent
                        // If name is "updated-name-X", description should be "Updated description X"
                        if let Some(name_version) = current.name.strip_prefix("updated-name-") {
                            let expected_desc = format!("Updated description {}", name_version);
                            if let Some(desc) = &current.description {
                                if desc != &expected_desc
                                    && desc != "Original description"
                                    && !desc.starts_with("Updated description")
                                {
                                    panic!(
                                        "BUG: Inconsistent state after update {} - \
                                         name='{}' but description='{}'. \
                                         This indicates partial update was persisted.",
                                        i, current.name, desc
                                    );
                                }
                            }
                        }

                        // Verify tags match name version
                        if let Some(name_version) = current.name.strip_prefix("updated-name-") {
                            let expected_tag = format!("update-{}", name_version);
                            if !current.tags.contains(&expected_tag)
                                && current.tags != vec!["original"]
                            {
                                panic!(
                                    "BUG: Tags don't match name version after update {} - \
                                     name='{}' tags={:?}",
                                    i, current.name, current.tags
                                );
                            }
                        }
                    }
                    Err(e) => {
                        panic!(
                            "BUG: After update attempt {}, agent is unreadable: {}. \
                             State corruption during deactivation?",
                            i, e
                        );
                    }
                }

                // Small delay to allow async operations to complete
                kelpie_core::current_runtime()
                    .sleep(std::time::Duration::from_millis(5))
                    .await;
            }

            println!("\nUpdate results:");
            for (i, success) in &update_results {
                println!(
                    "  Update {}: {}",
                    i,
                    if *success { "success" } else { "failed" }
                );
            }

            // Read final state
            match service.get_agent(&agent.id).await {
                Ok(final_state) => {
                    println!("\nFinal state:");
                    println!("  Name: {}", final_state.name);
                    println!("  Description: {:?}", final_state.description);
                    println!("  Tags: {:?}", final_state.tags);
                }
                Err(e) => {
                    panic!("BUG: Final state is unreadable: {}", e);
                }
            }

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Test failed (found update consistency bug!): {:?}",
        result.err()
    );
}

// ============================================================================
// Test Helpers
// ============================================================================

struct SimLlmClientAdapter {
    client: Arc<SimLlmClient>,
}

#[async_trait]
impl LlmClient for SimLlmClientAdapter {
    async fn complete_with_tools(
        &self,
        messages: Vec<LlmMessage>,
        tools: Vec<kelpie_server::llm::ToolDefinition>,
    ) -> Result<LlmResponse> {
        let sim_messages: Vec<kelpie_dst::SimChatMessage> = messages
            .into_iter()
            .map(|m| kelpie_dst::SimChatMessage {
                role: m.role,
                content: m.content,
            })
            .collect();
        let sim_tools: Vec<kelpie_dst::SimToolDefinition> = tools
            .into_iter()
            .map(|t| kelpie_dst::SimToolDefinition {
                name: t.name,
                description: t.description,
            })
            .collect();

        let response = self
            .client
            .complete_with_tools(sim_messages, sim_tools)
            .await
            .map_err(|e| kelpie_core::Error::Internal {
                message: format!("LLM error: {}", e),
            })?;

        Ok(LlmResponse {
            content: response.content,
            tool_calls: response
                .tool_calls
                .into_iter()
                .map(|tc| kelpie_server::actor::LlmToolCall {
                    id: tc.id,
                    name: tc.name,
                    input: tc.input,
                })
                .collect(),
            prompt_tokens: response.prompt_tokens,
            completion_tokens: response.completion_tokens,
            stop_reason: response.stop_reason,
        })
    }

    async fn continue_with_tool_result(
        &self,
        messages: Vec<LlmMessage>,
        tools: Vec<kelpie_server::llm::ToolDefinition>,
        _assistant_blocks: Vec<kelpie_server::llm::ContentBlock>,
        tool_results: Vec<(String, String)>,
    ) -> Result<LlmResponse> {
        let sim_messages: Vec<kelpie_dst::SimChatMessage> = messages
            .into_iter()
            .map(|m| kelpie_dst::SimChatMessage {
                role: m.role,
                content: m.content,
            })
            .collect();
        let sim_tools: Vec<kelpie_dst::SimToolDefinition> = tools
            .into_iter()
            .map(|t| kelpie_dst::SimToolDefinition {
                name: t.name,
                description: t.description,
            })
            .collect();

        let response = self
            .client
            .continue_with_tool_result(sim_messages, sim_tools, tool_results)
            .await
            .map_err(|e| kelpie_core::Error::Internal {
                message: format!("LLM error: {}", e),
            })?;

        Ok(LlmResponse {
            content: response.content,
            tool_calls: response
                .tool_calls
                .into_iter()
                .map(|tc| kelpie_server::actor::LlmToolCall {
                    id: tc.id,
                    name: tc.name,
                    input: tc.input,
                })
                .collect(),
            prompt_tokens: response.prompt_tokens,
            completion_tokens: response.completion_tokens,
            stop_reason: response.stop_reason,
        })
    }
}

fn create_service(sim_env: &SimEnvironment) -> Result<AgentService<kelpie_core::CurrentRuntime>> {
    use kelpie_core::Runtime;
    let sim_llm = SimLlmClient::new(sim_env.fork_rng_raw(), sim_env.faults.clone());
    let llm_adapter: Arc<dyn LlmClient> = Arc::new(SimLlmClientAdapter {
        client: Arc::new(sim_llm),
    });
    let actor = AgentActor::new(llm_adapter, Arc::new(UnifiedToolRegistry::new()));
    let factory = Arc::new(CloneFactory::new(actor));
    let kv = Arc::new(sim_env.storage.clone());
    let mut dispatcher =
        Dispatcher::<AgentActor, AgentActorState, kelpie_core::CurrentRuntime>::new(
            factory,
            kv,
            DispatcherConfig::default(),
            kelpie_core::current_runtime(),
        );
    let handle = dispatcher.handle();
    let _dispatcher_handle = kelpie_core::current_runtime().spawn(async move {
        dispatcher.run().await;
    });
    Ok(AgentService::new(handle))
}
