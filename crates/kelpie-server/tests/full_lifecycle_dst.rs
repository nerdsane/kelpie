//! Full lifecycle DST test - Phase 3 verification
//!
//! Tests that AgentActor writes granular keys (message:{N}, message_count, blocks)
//! on deactivation, fixing the storage gap where API couldn't read actor data.
//!
//! FDB Principle: Same Code Path
//! Uses production AgentActor with simulated storage that supports fault injection.
//!
//! Fault Injection:
//! - StorageWriteFail: Tests resilience of granular key writes
//! - StorageReadFail: Tests resilience of key reads during verification

#![cfg(feature = "dst")]

use async_trait::async_trait;
use bytes::Bytes;
use kelpie_core::actor::{Actor, ActorContext, ActorId};
use kelpie_core::Result;
use kelpie_dst::{FaultConfig, FaultType, SimConfig, Simulation};
use kelpie_server::actor::{AgentActor, AgentActorState, LlmClient, LlmMessage, LlmResponse};
use kelpie_server::models::{AgentType, CreateAgentRequest, CreateBlockRequest};
use kelpie_server::tools::UnifiedToolRegistry;
use kelpie_storage::ScopedKV;
use std::sync::Arc;

/// Mock LLM for testing
///
/// Note: This test focuses on storage lifecycle, not LLM behavior.
/// The MockLlm provides deterministic responses for lifecycle testing.
/// For LLM fault injection, see real_llm_adapter_streaming_dst.rs.
struct MockLlm;

#[async_trait]
impl LlmClient for MockLlm {
    async fn complete(&self, _messages: Vec<LlmMessage>) -> Result<LlmResponse> {
        Ok(LlmResponse {
            content: "Mock response".to_string(),
            tool_calls: vec![],
            prompt_tokens: 10,
            completion_tokens: 10,
            stop_reason: "end_turn".to_string(),
        })
    }

    async fn complete_with_tools(
        &self,
        _messages: Vec<LlmMessage>,
        _tools: Vec<kelpie_server::llm::ToolDefinition>,
    ) -> Result<LlmResponse> {
        Ok(LlmResponse {
            content: "Mock response with tools".to_string(),
            tool_calls: vec![],
            prompt_tokens: 10,
            completion_tokens: 10,
            stop_reason: "end_turn".to_string(),
        })
    }

    async fn continue_with_tool_result(
        &self,
        _messages: Vec<LlmMessage>,
        _tools: Vec<kelpie_server::llm::ToolDefinition>,
        _assistant_blocks: Vec<kelpie_server::llm::ContentBlock>,
        _tool_results: Vec<(String, String)>,
    ) -> Result<LlmResponse> {
        Ok(LlmResponse {
            content: "Continued".to_string(),
            tool_calls: vec![],
            prompt_tokens: 5,
            completion_tokens: 10,
            stop_reason: "end_turn".to_string(),
        })
    }
}

/// Test storage gap fix: Actor writes granular keys on deactivation
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_actor_writes_granular_keys_on_deactivate() {
    let config = SimConfig::new(9001);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            // Create actor
            let llm = Arc::new(MockLlm);
            let actor = AgentActor::new(llm, Arc::new(UnifiedToolRegistry::new()));

            // Create actor context
            let actor_id = ActorId::new("agents", "test-agent")?;
            let kv = Arc::new(sim_env.storage.clone());
            let scoped_kv = ScopedKV::new(actor_id.clone(), kv.clone());
            let mut ctx = ActorContext::new(
                actor_id.clone(),
                AgentActorState::default(),
                Box::new(scoped_kv),
            );

            // 1. Create agent
            let request = CreateAgentRequest {
                name: "Test Agent".to_string(),
                agent_type: AgentType::LettaV1Agent,
                model: None,
                embedding: None,
                system: Some("You are helpful".to_string()),
                description: None,
                memory_blocks: vec![
                    CreateBlockRequest {
                        label: "persona".to_string(),
                        value: "I am a test agent".to_string(),
                        description: None,
                        limit: None,
                    },
                    CreateBlockRequest {
                        label: "human".to_string(),
                        value: "User is testing".to_string(),
                        description: None,
                        limit: None,
                    },
                ],
                block_ids: vec![],
                tool_ids: vec![],
                tags: vec![],
                metadata: serde_json::json!({}),
                project_id: None,
                user_id: None,
                org_id: None,
            };

            let payload = serde_json::to_vec(&request).unwrap();
            actor
                .invoke(&mut ctx, "create", Bytes::from(payload))
                .await?;

            // 2. Send message (creates message history)
            let msg_request = kelpie_server::actor::HandleMessageFullRequest {
                content: "Hello!".to_string(),
                call_context: None, // Top-level call
            };
            let msg_payload = serde_json::to_vec(&msg_request).unwrap();
            actor
                .invoke(&mut ctx, "handle_message_full", Bytes::from(msg_payload))
                .await?;

            let message_count_before = ctx.state.messages.len();
            assert!(message_count_before > 0, "Should have messages");

            // 3. Deactivate actor (writes granular keys)
            actor.on_deactivate(&mut ctx).await?;

            tracing::info!(
                agent_id = %actor_id,
                message_count = message_count_before,
                "Actor deactivated, granular keys written"
            );

            // 4. **KEY TEST**: Verify granular keys were written
            let kv_trait: &dyn kelpie_storage::ActorKV = kv.as_ref();

            // Check message_count key
            let count_bytes = kv_trait.get(&actor_id, b"message_count").await?;
            assert!(count_bytes.is_some(), "message_count key should exist");
            let count_str = String::from_utf8(count_bytes.unwrap().to_vec()).unwrap();
            let count: usize = count_str.parse().unwrap();
            assert_eq!(count, message_count_before, "message_count should match");

            // Check individual message keys
            for idx in 0..message_count_before {
                let message_key = format!("message:{}", idx);
                let msg_bytes = kv_trait.get(&actor_id, message_key.as_bytes()).await?;
                assert!(msg_bytes.is_some(), "message:{} key should exist", idx);

                // Verify we can deserialize the message
                let msg: kelpie_server::models::Message =
                    serde_json::from_slice(&msg_bytes.unwrap()).unwrap();
                tracing::info!(message_id = %msg.id, "Read message {}", idx);
            }

            // Check blocks key
            let blocks_bytes = kv_trait.get(&actor_id, b"blocks").await?;
            assert!(blocks_bytes.is_some(), "blocks key should exist");

            let blocks: Vec<kelpie_server::models::Block> =
                serde_json::from_slice(&blocks_bytes.unwrap()).unwrap();
            assert_eq!(blocks.len(), 2, "Should have 2 blocks");
            assert_eq!(blocks[0].label, "persona");
            assert_eq!(blocks[1].label, "human");

            tracing::info!("✅ Storage gap fix verified!");

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

/// Test empty agent (no messages)
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_empty_agent_writes_zero_count() {
    let config = SimConfig::new(9002);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            let llm = Arc::new(MockLlm);
            let actor = AgentActor::new(llm, Arc::new(UnifiedToolRegistry::new()));

            let actor_id = ActorId::new("agents", "empty-agent")?;
            let kv = Arc::new(sim_env.storage.clone());
            let scoped_kv = ScopedKV::new(actor_id.clone(), kv.clone());
            let mut ctx = ActorContext::new(
                actor_id.clone(),
                AgentActorState::default(),
                Box::new(scoped_kv),
            );

            // Create agent without messages
            let request = CreateAgentRequest {
                name: "Empty Agent".to_string(),
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

            let payload = serde_json::to_vec(&request).unwrap();
            actor
                .invoke(&mut ctx, "create", Bytes::from(payload))
                .await?;

            // Deactivate without sending messages
            actor.on_deactivate(&mut ctx).await?;

            // Verify message_count is 0
            let kv_trait: &dyn kelpie_storage::ActorKV = kv.as_ref();
            let count_bytes = kv_trait.get(&actor_id, b"message_count").await?;
            assert!(count_bytes.is_some(), "message_count key should exist");
            let count_str = String::from_utf8(count_bytes.unwrap().to_vec()).unwrap();
            let count: usize = count_str.parse().unwrap();
            assert_eq!(count, 0, "Empty agent should have message_count=0");

            Ok(())
        })
        .await;

    assert!(result.is_ok());
}

/// Test lifecycle with storage fault injection
///
/// Verifies that the storage operations during agent lifecycle
/// handle faults gracefully. Tests both write failures during
/// create/deactivate and read failures during verification.
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_lifecycle_with_storage_faults() {
    let config = SimConfig::new(9003);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.05)) // 5% write failures
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.03)) // 3% read failures
        .run_async(|sim_env| async move {
            let mut success_count = 0;
            let mut failure_count = 0;

            // Run multiple iterations to trigger faults
            for i in 0..20 {
                let llm = Arc::new(MockLlm);
                let actor = AgentActor::new(llm, Arc::new(UnifiedToolRegistry::new()));

                let actor_id = ActorId::new("agents", format!("fault-test-agent-{}", i))?;
                let kv = Arc::new(sim_env.storage.clone());
                let scoped_kv = ScopedKV::new(actor_id.clone(), kv.clone());
                let mut ctx = ActorContext::new(
                    actor_id.clone(),
                    AgentActorState::default(),
                    Box::new(scoped_kv),
                );

                // Create agent
                let request = CreateAgentRequest {
                    name: format!("Fault Test Agent {}", i),
                    agent_type: AgentType::LettaV1Agent,
                    model: None,
                    embedding: None,
                    system: Some("You are helpful".to_string()),
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
                    metadata: serde_json::json!({}),
                    project_id: None,
                    user_id: None,
                    org_id: None,
                };

                let payload = serde_json::to_vec(&request).unwrap();
                match actor.invoke(&mut ctx, "create", Bytes::from(payload)).await {
                    Ok(_) => {
                        // Try to deactivate and write granular keys
                        match actor.on_deactivate(&mut ctx).await {
                            Ok(_) => {
                                success_count += 1;
                            }
                            Err(_) => {
                                failure_count += 1;
                            }
                        }
                    }
                    Err(_) => {
                        failure_count += 1;
                    }
                }
            }

            // With 5% write + 3% read fault rates over 20 iterations,
            // we should see some successes and possibly some failures
            tracing::info!(
                success_count = success_count,
                failure_count = failure_count,
                "Lifecycle chaos test completed"
            );

            assert!(success_count > 0, "Should have some successful operations");

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Lifecycle fault test failed: {:?}",
        result.err()
    );
}

/// Test lifecycle under high storage fault rate
///
/// Verifies that the system handles high fault rates gracefully
/// without panicking or corrupting state.
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_lifecycle_high_fault_rate_chaos() {
    let config = SimConfig::new(9004);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.30)) // 30% write failures
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.20)) // 20% read failures
        .run_async(|sim_env| async move {
            let mut success_count = 0;
            let mut failure_count = 0;

            // Run 50 iterations with high fault rate
            for i in 0..50 {
                let llm = Arc::new(MockLlm);
                let actor = AgentActor::new(llm, Arc::new(UnifiedToolRegistry::new()));

                let actor_id = ActorId::new("agents", format!("chaos-agent-{}", i))?;
                let kv = Arc::new(sim_env.storage.clone());
                let scoped_kv = ScopedKV::new(actor_id.clone(), kv.clone());
                let mut ctx = ActorContext::new(
                    actor_id.clone(),
                    AgentActorState::default(),
                    Box::new(scoped_kv),
                );

                let request = CreateAgentRequest {
                    name: format!("Chaos Agent {}", i),
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

                let payload = serde_json::to_vec(&request).unwrap();
                match actor.invoke(&mut ctx, "create", Bytes::from(payload)).await {
                    Ok(_) => match actor.on_deactivate(&mut ctx).await {
                        Ok(_) => success_count += 1,
                        Err(_) => failure_count += 1,
                    },
                    Err(_) => failure_count += 1,
                }
            }

            tracing::info!(
                success_count = success_count,
                failure_count = failure_count,
                "High fault rate chaos test completed"
            );

            // With 30% write + 20% read, expect both successes and failures
            assert!(
                success_count > 0,
                "Should have some successful operations despite high fault rate"
            );
            assert!(
                failure_count > 0,
                "Should have some failures with 30%/20% fault rates"
            );

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "High fault rate chaos test failed: {:?}",
        result.err()
    );
}
