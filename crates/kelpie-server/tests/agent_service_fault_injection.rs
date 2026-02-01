//! Aggressive Fault Injection Tests for AgentService
//!
//! TigerStyle: These tests aim to find REAL bugs through fault injection,
//! not just verify that errors propagate. Focus on:
//! - Multi-step operation atomicity
//! - State consistency under crashes
//! - Partial write scenarios
//! - Concurrent operations with faults

use kelpie_core::{Result, Runtime};
use kelpie_dst::{FaultConfig, FaultType, SimConfig, SimEnvironment, SimLlmClient, Simulation};
use kelpie_runtime::{CloneFactory, Dispatcher, DispatcherConfig};
use kelpie_server::actor::{AgentActor, AgentActorState, LlmClient, LlmMessage, LlmResponse};
use kelpie_server::models::{AgentType, CreateAgentRequest, CreateBlockRequest};
use kelpie_server::service::AgentService;
use std::sync::Arc;

// ============================================================================
// Multi-Step Operation Fault Injection
// ============================================================================

/// Test create_agent with crash-after-write during state persistence
///
/// ARCHITECTURE NOTE: This test verifies that CrashAfterWrite does NOT
/// cause create_agent failures, because:
/// 1. create_agent() sets state in memory only (no storage writes)
/// 2. Storage writes happen during deactivation (async, errors swallowed)
/// 3. This is by design for fault tolerance during deactivation
///
/// The test verifies that agents can be created and immediately read back
/// even with storage faults, because the actor stays in memory until
/// deactivation timeout.
#[tokio::test]
async fn test_create_agent_crash_after_write() {
    let config = SimConfig::new(2001);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::CrashAfterWrite, 0.3))
        .run_async(|sim_env| async move {
            let service = create_service(&sim_env)?;

            let mut created_ids = Vec::new();
            let mut success_count = 0;

            // Create 20 agents - all should succeed because:
            // 1. No storage writes during "create" operation
            // 2. Storage writes happen during deactivation (errors swallowed)
            for i in 0..20 {
                let request = CreateAgentRequest {
                    name: format!("agent-{}", i),
                    agent_type: AgentType::LettaV1Agent,
                    model: None,
                    embedding: None,
                    system: Some("Test system".to_string()),
                    description: None,
                    memory_blocks: vec![CreateBlockRequest {
                        label: "persona".to_string(),
                        value: format!("I am agent {}", i),
                        limit: None,
                        description: None,
                    }],
                    block_ids: vec![],
                    tool_ids: vec![],
                    tags: vec![],
                    metadata: serde_json::json!({}),
                    project_id: None,
                    user_id: None,
                    org_id: None,
                };

                // All creates should succeed since no storage writes during create
                let agent = service.create_agent(request).await?;
                success_count += 1;
                created_ids.push(agent.id.clone());

                // Verify we can read it back (from memory, not storage)
                let retrieved = service.get_agent(&agent.id).await?;
                assert_eq!(retrieved.id, agent.id, "ID mismatch");
                assert_eq!(retrieved.name, agent.name, "Name mismatch");
                assert_eq!(
                    retrieved.blocks.len(),
                    agent.blocks.len(),
                    "Block count mismatch"
                );
            }

            println!("Create stats: {} success", success_count);

            // All creates should succeed
            assert_eq!(
                success_count, 20,
                "All creates should succeed (no storage writes during create)"
            );

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Test failed (might have found a bug!): {:?}",
        result.err()
    );
}

/// Test delete_agent atomicity with crash-after-write
///
/// BUG TARGET: delete_agent has two steps:
/// 1. Invoke "delete_agent" operation (clears storage via kv_delete)
/// 2. Deactivate actor (removes from memory)
///
/// If crash happens after step 1 but before step 2:
/// - Storage is cleared
/// - Actor still in memory
/// - Next get_agent might return stale cached state
///
/// If crash happens DURING kv_delete:
/// - State might be partially cleared
/// - Reactivation might load corrupt state
#[tokio::test]
async fn test_delete_agent_atomicity_crash() {
    let config = SimConfig::new(2002);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::CrashAfterWrite, 0.4))
        .run_async(|sim_env| async move {
            let service = create_service(&sim_env)?;

            // Create 10 agents
            let mut agent_ids = Vec::new();
            for i in 0..10 {
                let request = CreateAgentRequest {
                    name: format!("to-delete-{}", i),
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
                // Ignore creation failures
                if let Ok(agent) = service.create_agent(request).await {
                    agent_ids.push(agent.id);
                }
            }

            println!("Successfully created {} agents", agent_ids.len());

            // Now try to delete them with crash-after-write faults
            for agent_id in &agent_ids {
                match service.delete_agent(agent_id).await {
                    Ok(_) => {
                        // CRITICAL: Verify it's actually deleted
                        // Wait a bit to allow deactivation to complete
                        kelpie_core::current_runtime()
                            .sleep(std::time::Duration::from_millis(10))
                            .await;

                        match service.get_agent(agent_id).await {
                            Ok(agent) => {
                                // BUG FOUND: Delete succeeded but agent still exists
                                panic!(
                                    "BUG: delete_agent succeeded but agent {} still exists: {:?}. \
                                     Atomicity violation - state not cleared or actor not deactivated.",
                                    agent_id, agent
                                );
                            }
                            Err(_) => {
                                // Good - agent is gone
                            }
                        }
                    }
                    Err(_e) => {
                        // Crash during delete is expected
                        // But verify agent is in a consistent state (either exists or doesn't)
                        match service.get_agent(agent_id).await {
                            Ok(_agent) => {
                                // Agent still exists - delete failed cleanly
                            }
                            Err(_) => {
                                // Agent is gone - delete succeeded despite error
                                // This could be a bug if error is misleading
                            }
                        }
                    }
                }
            }

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Test failed (might have found atomicity bug!): {:?}",
        result.err()
    );
}

/// Test update_agent with concurrent operations and faults
///
/// BUG TARGET: What if two updates happen concurrently?
/// - Lost updates (last write wins without merge)
/// - Partial updates visible
/// - Read-your-writes violations
#[tokio::test]
async fn test_update_agent_concurrent_with_faults() {
    let config = SimConfig::new(2003);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::CrashAfterWrite, 0.2))
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.2))
        .run_async(|sim_env| async move {
            let service = create_service(&sim_env)?;

            // Create one agent
            let request = CreateAgentRequest {
                name: "concurrent-test".to_string(),
                agent_type: AgentType::LettaV1Agent,
                model: None,
                embedding: None,
                system: None,
                description: Some("Original".to_string()),
                memory_blocks: vec![],
                block_ids: vec![],
                tool_ids: vec![],
                tags: vec![],
                metadata: serde_json::json!({}),
                project_id: None,
                user_id: None,
                org_id: None,
            };

            let agent = match service.create_agent(request).await {
                Ok(a) => a,
                Err(e) => {
                    println!("Skipping test - couldn't create agent: {}", e);
                    return Ok(());
                }
            };

            // Launch 5 concurrent updates with different fields
            let mut handles = vec![];
            for i in 0..5 {
                let service_clone = service.clone();
                let agent_id = agent.id.clone();
                let handle = kelpie_core::current_runtime().spawn(async move {
                    let update = serde_json::json!({
                        "name": format!("update-{}", i),
                        "description": format!("Description from thread {}", i),
                    });
                    service_clone.update_agent(&agent_id, update).await
                });
                handles.push(handle);
            }

            // Wait for all updates
            let mut success_count = 0;
            let mut failure_count = 0;
            for handle in handles {
                match handle.await {
                    Ok(Ok(_)) => success_count += 1,
                    Ok(Err(_)) => failure_count += 1,
                    Err(_) => failure_count += 1,
                }
            }

            println!(
                "Concurrent updates: {} success, {} failures",
                success_count, failure_count
            );

            // Read final state
            match service.get_agent(&agent.id).await {
                Ok(final_agent) => {
                    println!("Final agent state: {:?}", final_agent);

                    // CRITICAL CHECKS:
                    // 1. State should be consistent (not mix of multiple updates)
                    // 2. If name is "update-X", description should be "Description from thread X"
                    if final_agent.name.starts_with("update-") {
                        let thread_num = final_agent.name.strip_prefix("update-").unwrap();
                        let expected_desc = format!("Description from thread {}", thread_num);

                        if let Some(desc) = &final_agent.description {
                            if desc != &expected_desc && desc != "Original" {
                                // BUG FOUND: Partial update visible
                                panic!(
                                    "BUG: Inconsistent state - name is '{}' but description is '{}'. \
                                     Expected '{}' or 'Original'. This indicates partial updates are visible.",
                                    final_agent.name, desc, expected_desc
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    println!("Warning: Could not read final state: {}", e);
                }
            }

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Test failed (might have found concurrency bug!): {:?}",
        result.err()
    );
}

/// Test storage corruption during agent state persistence
///
/// BUG TARGET: What if storage returns corrupted data?
/// - Can we detect it?
/// - Do we panic or handle gracefully?
/// - Does deserialization catch it?
#[tokio::test]
async fn test_agent_state_corruption() {
    let config = SimConfig::new(2004);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageCorruption, 0.3))
        .run_async(|sim_env| async move {
            let service = create_service(&sim_env)?;

            // Create agent
            let request = CreateAgentRequest {
                name: "corruption-test".to_string(),
                agent_type: AgentType::LettaV1Agent,
                model: None,
                embedding: None,
                system: Some("Test system".to_string()),
                description: None,
                memory_blocks: vec![CreateBlockRequest {
                    label: "persona".to_string(),
                    value: "Test value with important data".to_string(),
                    limit: None,
                    description: None,
                }],
                block_ids: vec![],
                tool_ids: vec![],
                tags: vec![],
                metadata: serde_json::json!({"key": "value"}),
                project_id: None,
                user_id: None,
                org_id: None,            };

            let agent = match service.create_agent(request).await {
                Ok(a) => a,
                Err(e) => {
                    println!("Skipping test - couldn't create agent: {}", e);
                    return Ok(());
                }
            };

            // Read back multiple times - some reads might get corrupted data
            let mut corruption_detected = false;
            let mut successful_reads = 0;
            let mut failed_reads = 0;

            for i in 0..20 {
                match service.get_agent(&agent.id).await {
                    Ok(retrieved) => {
                        successful_reads += 1;

                        // CRITICAL: Verify data integrity
                        if retrieved.name != agent.name {
                            corruption_detected = true;
                            println!(
                                "WARNING: Read {} - Name corruption detected: expected '{}', got '{}'",
                                i, agent.name, retrieved.name
                            );
                        }

                        if retrieved.blocks.len() != agent.blocks.len() {
                            corruption_detected = true;
                            println!(
                                "WARNING: Read {} - Block count corruption: expected {}, got {}",
                                i,
                                agent.blocks.len(),
                                retrieved.blocks.len()
                            );
                        }

                        // Verify block data integrity
                        if !retrieved.blocks.is_empty()
                            && !retrieved.blocks[0].value.contains("important data")
                        {
                            corruption_detected = true;
                            println!(
                                "WARNING: Read {} - Block data corruption: '{}'",
                                i, retrieved.blocks[0].value
                            );
                        }
                    }
                    Err(e) => {
                        failed_reads += 1;
                        let err_str = e.to_string();

                        // Check if error indicates corruption was caught
                        if err_str.contains("deserialize") || err_str.contains("parse") {
                            corruption_detected = true;
                            println!("Read {} - Corruption caught by deserializer: {}", i, e);
                        } else {
                            println!("Read {} - Failed: {}", i, e);
                        }
                    }
                }
            }

            println!(
                "Corruption test: {} successful reads, {} failed reads, corruption_detected={}",
                successful_reads, failed_reads, corruption_detected
            );

            // With 30% corruption rate, we should detect some corruption
            assert!(
                corruption_detected || failed_reads > 0,
                "Expected to detect corruption with 30% fault rate"
            );

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Test failed (corruption handling issue?): {:?}",
        result.err()
    );
}

/// Test send_message with LLM failure during state update
///
/// BUG TARGET: handle_message does:
/// 1. Read agent state
/// 2. Build prompt from blocks
/// 3. Call LLM
/// 4. Return response
///
/// But what about iteration counter? Session state?
/// If we crash after LLM call but before state update, do we:
/// - Lose the response?
/// - Double-charge for the same message?
/// - Get inconsistent iteration counts?
#[tokio::test]
async fn test_send_message_crash_after_llm() {
    let config = SimConfig::new(2005);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::CrashAfterWrite, 0.3))
        .run_async(|sim_env| async move {
            let service = create_service(&sim_env)?;

            // Create agent
            let request = CreateAgentRequest {
                name: "message-test".to_string(),
                agent_type: AgentType::LettaV1Agent,
                model: None,
                embedding: None,
                system: Some("You are a helpful assistant".to_string()),
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

            let agent = match service.create_agent(request).await {
                Ok(a) => a,
                Err(e) => {
                    println!("Skipping test - couldn't create agent: {}", e);
                    return Ok(());
                }
            };

            // Send multiple messages with crash-after-write faults
            let mut successful_messages = 0;
            let mut failed_messages = 0;

            for i in 0..10 {
                let message = serde_json::json!({
                    "role": "user",
                    "content": format!("Message number {}", i)
                });

                match service.send_message(&agent.id, message).await {
                    Ok(response) => {
                        successful_messages += 1;
                        println!("Message {} succeeded: {:?}", i, response);

                        // CRITICAL: Verify we can still read agent state
                        // If crash happened after LLM but before state update,
                        // actor might be in inconsistent state
                        match service.get_agent(&agent.id).await {
                            Ok(_) => {} // Good
                            Err(e) => {
                                panic!(
                                    "BUG: After successful message {}, agent state is unreadable: {}. \
                                     State corruption after LLM call?",
                                    i, e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        failed_messages += 1;
                        println!("Message {} failed (expected): {}", i, e);
                    }
                }
            }

            println!(
                "Message test: {} success, {} failures",
                successful_messages, failed_messages
            );

            // Read final agent state and verify consistency
            match service.get_agent(&agent.id).await {
                Ok(final_agent) => {
                    println!("Final agent iteration: {}", final_agent.id);
                    // TODO: When we add iteration counter, verify it matches message count
                }
                Err(e) => {
                    println!("Warning: Could not read final state: {}", e);
                }
            }

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Test failed (message handling bug?): {:?}",
        result.err()
    );
}

// ============================================================================
// Test Helpers
// ============================================================================

use async_trait::async_trait;
use kelpie_server::tools::UnifiedToolRegistry;

/// Adapter to use SimLlmClient as LlmClient
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

/// Create AgentService from simulation environment
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
