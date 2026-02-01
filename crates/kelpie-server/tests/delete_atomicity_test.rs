//! Delete Operation Atomicity Test
//!
//! Test for potential BUG-002: delete_agent two-step atomicity

use async_trait::async_trait;
use kelpie_core::Result;
use kelpie_dst::{FaultConfig, FaultType, SimConfig, SimEnvironment, SimLlmClient, Simulation};
use kelpie_runtime::{CloneFactory, Dispatcher, DispatcherConfig};
use kelpie_server::actor::{AgentActor, AgentActorState, LlmClient, LlmMessage, LlmResponse};
use kelpie_server::models::{AgentType, CreateAgentRequest};
use kelpie_server::service::AgentService;
use kelpie_server::tools::UnifiedToolRegistry;
use std::sync::Arc;

/// Test delete_agent with crash between state clear and deactivate
///
/// POTENTIAL BUG-002: delete_agent has two steps:
/// 1. Invoke "delete_agent" operation (clears storage)
/// 2. dispatcher.deactivate() (removes from memory)
///
/// If crash happens after step 1 but before step 2:
/// - Storage is cleared (correct)
/// - Actor stays in memory with cleared state (memory leak)
/// - Next get_agent would reactivate and find no state (correct behavior)
///
/// The question: Is this a bug or acceptable behavior?
/// - Memory leak: Actor stays in memory until manual eviction
/// - But functionally correct: Agent doesn't exist from user perspective
#[tokio::test]
async fn test_delete_crash_between_clear_and_deactivate() {
    let config = SimConfig::new(4001);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::CrashAfterWrite, 0.5))
        .run_async(|sim_env| async move {
            let service = create_service(&sim_env)?;

            let mut deleted_count = 0;
            let mut delete_failed_count = 0;
            let mut leaked_actors = Vec::new();

            // Create and delete 20 agents
            for i in 0..20 {
                let request = CreateAgentRequest {
                    name: format!("delete-test-{}", i),
                    agent_type: AgentType::LettaV1Agent,
                    model: None,
                    embedding: None,
                    system: None,
                    description: None,
                    memory_blocks: vec![],
                    block_ids: vec![],
                    tool_ids: vec![],
                    tags: vec![],
                    metadata: serde_json::json!({}),
                    project_id: None,
                    user_id: None,
                    org_id: None,
                };

                // Create agent
                let agent = match service.create_agent(request).await {
                    Ok(a) => a,
                    Err(_) => continue, // Skip if creation fails
                };

                // Try to delete with crash-after-write faults
                match service.delete_agent(&agent.id).await {
                    Ok(_) => {
                        deleted_count += 1;

                        // Verify agent is actually gone
                        match service.get_agent(&agent.id).await {
                            Ok(still_exists) => {
                                // BUG: Delete succeeded but agent still exists
                                leaked_actors.push((agent.id.clone(), still_exists));
                            }
                            Err(_) => {
                                // Good - agent is gone
                            }
                        }
                    }
                    Err(_e) => {
                        delete_failed_count += 1;

                        // Verify agent is in consistent state
                        match service.get_agent(&agent.id).await {
                            Ok(_) => {
                                // Agent still exists - delete failed cleanly
                            }
                            Err(_) => {
                                // Agent is gone despite error - might be misleading
                                // but functionally correct
                            }
                        }
                    }
                }
            }

            println!(
                "Delete stats: {} deleted, {} failed",
                deleted_count, delete_failed_count
            );

            if !leaked_actors.is_empty() {
                println!("\n=== POTENTIAL MEMORY LEAKS ===");
                for (id, agent) in &leaked_actors {
                    println!(
                        "  - Agent {} still exists after delete: {:?}",
                        id, agent.name
                    );
                }
                panic!(
                    "Found {} agents still accessible after delete",
                    leaked_actors.len()
                );
            }

            println!("No memory leaks detected - all deleted agents are inaccessible");

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Test failed (found delete bug?): {:?}",
        result.err()
    );
}

/// Test delete_agent immediately followed by create_agent with same name
///
/// This tests if delete properly clears all state so a new agent can be created
#[tokio::test]
async fn test_delete_then_recreate() {
    let config = SimConfig::new(4002);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::CrashAfterWrite, 0.3))
        .run_async(|sim_env| async move {
            let service = create_service(&sim_env)?;

            for i in 0..10 {
                let request = CreateAgentRequest {
                    name: format!("recreate-test-{}", i),
                    agent_type: AgentType::LettaV1Agent,
                    model: None,
                    embedding: None,
                    system: Some("Version 1".to_string()),
                    description: Some("First version".to_string()),
                    memory_blocks: vec![],
                    block_ids: vec![],
                    tool_ids: vec![],
                    tags: vec!["v1".to_string()],
                    metadata: serde_json::json!({"version": 1}),
                    project_id: None,
                    user_id: None,
                    org_id: None,
                };

                // Create agent v1
                let agent_v1 = match service.create_agent(request).await {
                    Ok(a) => a,
                    Err(_) => continue,
                };

                println!("Created agent v1: {}", agent_v1.id);

                // Delete it
                match service.delete_agent(&agent_v1.id).await {
                    Ok(_) => {
                        println!("Deleted agent: {}", agent_v1.id);
                    }
                    Err(e) => {
                        println!("Delete failed: {}", e);
                        continue;
                    }
                }

                // Verify it's gone
                match service.get_agent(&agent_v1.id).await {
                    Ok(agent) => {
                        panic!(
                            "BUG: Agent {} still exists after delete: {:?}",
                            agent_v1.id, agent
                        );
                    }
                    Err(_) => {
                        println!("Verified agent {} is gone", agent_v1.id);
                    }
                }

                // Create new agent with same name but different data (different ID)
                let request_v2 = CreateAgentRequest {
                    name: format!("recreate-test-{}", i),
                    agent_type: AgentType::ReactAgent, // Different type
                    model: None,
                    embedding: None,
                    system: Some("Version 2".to_string()), // Different system
                    description: Some("Second version".to_string()),
                    memory_blocks: vec![],
                    block_ids: vec![],
                    tool_ids: vec![],
                    tags: vec!["v2".to_string()],
                    metadata: serde_json::json!({"version": 2}),
                    project_id: None,
                    user_id: None,
                    org_id: None,
                };

                let agent_v2 = match service.create_agent(request_v2).await {
                    Ok(a) => a,
                    Err(e) => {
                        panic!(
                            "BUG: Could not recreate agent after delete for test {}: {}",
                            i, e
                        );
                    }
                };

                println!("Created agent v2: {}", agent_v2.id);

                // Verify v2 has correct data (not v1 data)
                assert_ne!(
                    agent_v1.id, agent_v2.id,
                    "New agent should have different ID"
                );
                assert_eq!(agent_v2.agent_type, AgentType::ReactAgent);
                assert_eq!(agent_v2.system, Some("Version 2".to_string()));
                assert_eq!(agent_v2.tags, vec!["v2".to_string()]);

                println!("Verified v2 has correct data, not v1 data");
            }

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Test failed (delete-recreate bug?): {:?}",
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
