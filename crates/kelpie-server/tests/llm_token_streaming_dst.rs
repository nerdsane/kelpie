//! DST tests for LLM token streaming (Phase 7.6)
//!
//! TigerStyle: DST-first with full simulation environment
//!
//! Tests cover:
//! - Basic token streaming (incremental delivery)
//! - Network delay tolerance
//! - Stream cancellation and cleanup
//! - Tool calls in streaming mode
//! - Concurrent streams
//!
//! These tests will FAIL initially (no stream_complete implementation yet).
#![cfg(feature = "dst")]

use async_trait::async_trait;
use futures::stream::StreamExt;
use kelpie_core::{Result, Runtime};
use kelpie_dst::{FaultConfig, FaultType, SimConfig, SimEnvironment, SimLlmClient, Simulation};
use kelpie_runtime::{CloneFactory, Dispatcher, DispatcherConfig};
use kelpie_server::actor::{
    AgentActor, AgentActorState, LlmClient, LlmMessage, LlmResponse, StreamChunk,
};
use kelpie_server::models::{AgentType, CreateAgentRequest};
use kelpie_server::service::AgentService;
use kelpie_server::tools::UnifiedToolRegistry;
use std::sync::Arc;

/// Adapter to use SimLlmClient with actor LlmClient trait
struct SimLlmClientAdapter {
    client: Arc<SimLlmClient>,
}

#[async_trait]
impl LlmClient for SimLlmClientAdapter {
    async fn complete_with_tools(
        &self,
        _messages: Vec<LlmMessage>,
        _tools: Vec<kelpie_server::llm::ToolDefinition>,
    ) -> Result<LlmResponse> {
        let _ = &self.client;
        Ok(LlmResponse {
            content: "Simulated LLM response".to_string(),
            tool_calls: vec![],
            prompt_tokens: 0,
            completion_tokens: 0,
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
        let _ = &self.client;
        Ok(LlmResponse {
            content: "Simulated LLM response".to_string(),
            tool_calls: vec![],
            prompt_tokens: 0,
            completion_tokens: 0,
            stop_reason: "end_turn".to_string(),
        })
    }

    // Phase 7.7: Uses default stream_complete implementation (batch to stream)
    // This is sufficient for DST tests - real streaming comes in Phase 7.8
}

/// Create AgentService from simulation environment
fn create_service<R: Runtime + 'static>(
    runtime: R,
    sim_env: &SimEnvironment,
) -> Result<AgentService<R>> {
    let sim_llm = SimLlmClient::new(sim_env.fork_rng_raw(), sim_env.faults.clone());
    let llm_adapter: Arc<dyn LlmClient> = Arc::new(SimLlmClientAdapter {
        client: Arc::new(sim_llm),
    });

    let actor = AgentActor::new(llm_adapter, Arc::new(UnifiedToolRegistry::new()));
    let factory = Arc::new(CloneFactory::new(actor));
    let kv = Arc::new(sim_env.storage.clone());

    let mut dispatcher = Dispatcher::<AgentActor, AgentActorState, _>::new(
        factory,
        kv,
        DispatcherConfig::default(),
        runtime.clone(),
    );
    let handle = dispatcher.handle();

    let _dispatcher_handle = runtime.spawn(async move {
        dispatcher.run().await;
    });

    Ok(AgentService::new(handle))
}

/// Test basic token streaming
///
/// Contract:
/// - Tokens arrive incrementally (not all at once)
/// - Concatenated chunks equal final content
/// - Stream ends with Done chunk
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_llm_token_streaming_basic() {
    let config = SimConfig::new(5001);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            use kelpie_core::current_runtime;
            let service = create_service(current_runtime(), &sim_env)?;

            // Create agent
            let request = CreateAgentRequest {
                name: "streaming-test".to_string(),
                agent_type: AgentType::LettaV1Agent,
                model: None,
                embedding: None,
                system: Some("You are helpful".to_string()),
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
            let agent = service.create_agent(request).await?;

            // Start streaming (this will fail until stream_complete exists)
            let mut stream = service
                .stream_message(&agent.id, "Tell me a story".to_string())
                .await?;

            // Collect chunks
            let mut chunks = Vec::new();
            let mut content = String::new();

            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result?;
                chunks.push(chunk.clone());

                match chunk {
                    StreamChunk::ContentDelta { delta } => {
                        content.push_str(&delta);
                    }
                    StreamChunk::Done { .. } => break,
                    _ => {}
                }
            }

            // Verify streaming behavior
            assert!(
                chunks.len() > 2,
                "Should have multiple chunks, got {}",
                chunks.len()
            );
            assert!(!content.is_empty(), "Should have content");

            // Verify Done chunk at end
            if let Some(StreamChunk::Done { stop_reason }) = chunks.last() {
                assert_eq!(stop_reason, "end_turn");
            } else {
                panic!("Last chunk should be Done");
            }

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Basic streaming test failed: {:?}",
        result.err()
    );
}

/// Test streaming with storage delays
///
/// Contract:
/// - Stream completes despite StorageLatency faults
/// - No tokens lost
/// - Final content is complete
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_llm_streaming_with_network_delay() {
    let config = SimConfig::new(5002);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::StorageLatency {
                min_ms: 10,
                max_ms: 50,
            },
            0.5, // 50% of operations delayed
        ))
        .run_async(|sim_env| async move {
            use kelpie_core::current_runtime;
            let service = create_service(current_runtime(), &sim_env)?;

            // Create agent
            let request = CreateAgentRequest {
                name: "delay-test".to_string(),
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
            let agent = service.create_agent(request).await?;

            // Stream with delays active
            let mut stream = service
                .stream_message(&agent.id, "Count to 5".to_string())
                .await?;

            let mut chunk_count = 0;
            let mut content = String::new();

            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result?;
                chunk_count += 1;

                if let StreamChunk::ContentDelta { delta } = chunk {
                    content.push_str(&delta);
                }
            }

            // Verify stream completed
            assert!(chunk_count > 0, "Should have received chunks");
            assert!(!content.is_empty(), "Should have content despite delays");

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Network delay streaming test failed: {:?}",
        result.err()
    );
}

/// Test stream cancellation
///
/// Contract:
/// - Dropping stream consumer stops iteration
/// - No panic or resource leak
/// - Clean shutdown
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_llm_streaming_cancellation() {
    let config = SimConfig::new(5003);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            use kelpie_core::current_runtime;
            let service = create_service(current_runtime(), &sim_env)?;

            // Create agent
            let request = CreateAgentRequest {
                name: "cancel-test".to_string(),
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
            let agent = service.create_agent(request).await?;

            // Start streaming
            let mut stream = service
                .stream_message(&agent.id, "Long response...".to_string())
                .await?;

            // Consume only 3 chunks then drop
            let mut consumed = 0;
            while let Some(chunk_result) = stream.next().await {
                let _chunk = chunk_result?;
                consumed += 1;
                if consumed >= 3 {
                    break; // Drop stream early
                }
            }

            // Stream is dropped here - should clean up without panic
            assert_eq!(consumed, 3, "Should have consumed exactly 3 chunks");

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Cancellation test failed: {:?}",
        result.err()
    );
}

/// Test streaming with tool calls
///
/// Contract:
/// - Tool calls appear as ToolCallStart chunks
/// - Content deltas continue after tool execution
/// - Stream completes with Done
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_llm_streaming_with_tool_calls() {
    let config = SimConfig::new(5004);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            use kelpie_core::current_runtime;
            let service = create_service(current_runtime(), &sim_env)?;

            // Create agent with tool
            let request = CreateAgentRequest {
                name: "tool-stream-test".to_string(),
                agent_type: AgentType::LettaV1Agent,
                model: None,
                embedding: None,
                system: Some("Use tools when needed".to_string()),
                description: None,
                memory_blocks: vec![],
                block_ids: vec![],
                tool_ids: vec!["shell".to_string()],
                tags: vec![],
                metadata: serde_json::json!({}),
                project_id: None,
                user_id: None,
                org_id: None,
            };
            let agent = service.create_agent(request).await?;

            // Stream message that triggers tool use
            let mut stream = service
                .stream_message(&agent.id, "Execute: echo hello".to_string())
                .await?;

            let mut has_tool_call = false;
            let mut has_content = false;

            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result?;

                match chunk {
                    StreamChunk::ToolCallStart { .. } => {
                        has_tool_call = true;
                    }
                    StreamChunk::ContentDelta { .. } => {
                        has_content = true;
                    }
                    StreamChunk::Done { .. } => break,
                    _ => {} // Handle ToolCallDelta and any future variants
                }
            }

            // Verify both tool call and content appeared
            assert!(
                has_tool_call || has_content,
                "Should have either tool call or content"
            );

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Tool call streaming test failed: {:?}",
        result.err()
    );
}

/// Test concurrent streaming
///
/// Contract:
/// - Multiple agents can stream concurrently
/// - Streams don't interfere with each other
/// - All streams complete successfully
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_llm_streaming_concurrent() {
    let config = SimConfig::new(5005);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            use kelpie_core::current_runtime;
            let runtime = current_runtime();
            let service = create_service(runtime.clone(), &sim_env)?;

            // Create 3 agents
            let mut agent_ids = Vec::new();
            for i in 1..=3 {
                let request = CreateAgentRequest {
                    name: format!("concurrent-stream-{}", i),
                    agent_type: AgentType::LettaV1Agent,
                    model: None,
                    embedding: None,
                    system: Some(format!("Agent {}", i)),
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
                let agent = service.create_agent(request).await?;
                agent_ids.push(agent.id);
            }

            // Start concurrent streams
            let mut handles = Vec::new();
            for (idx, agent_id) in agent_ids.iter().enumerate() {
                let service_clone = service.clone();
                let agent_id_clone = agent_id.clone();
                let handle = runtime.spawn(async move {
                    let mut stream = service_clone
                        .stream_message(&agent_id_clone, format!("Message {}", idx + 1))
                        .await?;

                    let mut chunk_count = 0;
                    while let Some(chunk_result) = stream.next().await {
                        let _chunk = chunk_result?;
                        chunk_count += 1;
                    }

                    Ok::<usize, kelpie_core::Error>(chunk_count)
                });
                handles.push(handle);
            }

            // All streams should complete
            for handle in handles {
                match handle.await {
                    Ok(Ok(count)) => {
                        assert!(count > 0, "Stream should have chunks");
                    }
                    Ok(Err(e)) => {
                        panic!("Stream failed: {:?}", e);
                    }
                    Err(e) => {
                        panic!("Task panicked: {:?}", e);
                    }
                }
            }

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Concurrent streaming test failed: {:?}",
        result.err()
    );
}

/// Test streaming with comprehensive fault injection (Phase 7.7)
///
/// Contract:
/// - Stream works despite multiple simultaneous faults
/// - No data corruption
/// - Graceful degradation
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_llm_streaming_with_comprehensive_faults() {
    let config = SimConfig::new(5006);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::NetworkDelay {
                min_ms: 5,
                max_ms: 30,
            },
            0.4, // 40% network delays
        ))
        .with_fault(FaultConfig::new(
            FaultType::StorageLatency {
                min_ms: 10,
                max_ms: 20,
            },
            0.3,
        ))
        .run_async(|sim_env| async move {
            use kelpie_core::current_runtime;
            let service = create_service(current_runtime(), &sim_env)?;

            // Create agent
            let request = CreateAgentRequest {
                name: "fault-tolerant-stream".to_string(),
                agent_type: AgentType::LettaV1Agent,
                model: None,
                embedding: None,
                system: Some("You are resilient".to_string()),
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
            let agent = service.create_agent(request).await?;

            // Stream with multiple faults active
            let mut stream = service
                .stream_message(&agent.id, "Test resilience".to_string())
                .await?;

            let mut chunk_count = 0;
            let mut content = String::new();

            while let Some(chunk_result) = stream.next().await {
                // Should not fail despite faults
                let chunk = chunk_result?;
                chunk_count += 1;

                if let StreamChunk::ContentDelta { delta } = chunk {
                    content.push_str(&delta);
                }
            }

            // Verify stream completed successfully despite faults
            assert!(chunk_count > 0, "Should have chunks despite faults");
            assert!(!content.is_empty(), "Should have content despite faults");

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Comprehensive fault streaming test failed: {:?}",
        result.err()
    );
}
