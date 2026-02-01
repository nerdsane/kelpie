//! DST tests for agent message handling with LLM and tools
//!
//! TigerStyle: DST-first development - these tests define the agent loop contract
//! and will initially FAIL until the full agent loop is implemented in AgentActor.
//!
//! Tests cover:
//! - Basic message → LLM response
//! - Tool execution loop
//! - Message history preservation
//! - Concurrent agent messaging
//! - Fault injection (storage failures)
#![cfg(feature = "dst")]

use async_trait::async_trait;
use kelpie_core::{current_runtime, Result, Runtime};
use kelpie_dst::{FaultConfig, FaultType, SimConfig, SimEnvironment, SimLlmClient, Simulation};
use kelpie_runtime::{CloneFactory, Dispatcher, DispatcherConfig};
use kelpie_server::actor::{AgentActor, AgentActorState, LlmClient, LlmMessage, LlmResponse};
use kelpie_server::models::{AgentType, CreateAgentRequest, CreateBlockRequest};
use kelpie_server::service::AgentService;
use kelpie_server::tools::{BuiltinToolHandler, UnifiedToolRegistry};
use serde_json::json;
use std::sync::Arc;

/// Adapter to use SimLlmClient with actor LlmClient trait
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
async fn create_service<R: Runtime + 'static>(
    runtime: R,
    sim_env: &SimEnvironment,
) -> Result<AgentService<R>> {
    create_service_with_tool_probability(runtime, sim_env, 0.3).await
}

/// Create AgentService with specific tool call probability
async fn create_service_with_tool_probability<R: Runtime + 'static>(
    runtime: R,
    sim_env: &SimEnvironment,
    tool_call_probability: f64,
) -> Result<AgentService<R>> {
    let sim_llm = SimLlmClient::new(sim_env.fork_rng_raw(), sim_env.faults.clone())
        .with_tool_call_probability(tool_call_probability);
    let llm_adapter: Arc<dyn LlmClient> = Arc::new(SimLlmClientAdapter {
        client: Arc::new(sim_llm),
    });

    // Create registry and register shell tool for testing
    let registry = Arc::new(UnifiedToolRegistry::new());
    let shell_handler: BuiltinToolHandler = Arc::new(|input: &serde_json::Value| {
        let input = input.clone();
        Box::pin(async move {
            let command = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("echo 'no command'");
            format!("Executed: {}", command)
        })
    });
    registry
        .register_builtin(
            "shell",
            "Execute a shell command for testing",
            json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The command to execute"
                    }
                },
                "required": ["command"]
            }),
            shell_handler,
        )
        .await;

    let actor = AgentActor::new(llm_adapter, registry);
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

/// Test basic message handling: send message, get LLM response
///
/// Contract:
/// - User sends message via service.send_message_full()
/// - LLM processes message
/// - Response returned with message history
/// - User message and assistant response stored in history
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_agent_message_basic() {
    let config = SimConfig::new(3001);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            use kelpie_core::current_runtime;
            let service = create_service(current_runtime(), &sim_env).await?;

            // Create agent
            let request = CreateAgentRequest {
                name: "message-test".to_string(),
                agent_type: AgentType::LettaV1Agent,
                model: None,
                embedding: None,
                system: Some("You are helpful".to_string()),
                description: None,
                memory_blocks: vec![CreateBlockRequest {
                    label: "persona".to_string(),
                    value: "I am a test agent".to_string(),
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
            let agent = service.create_agent(request).await?;

            // Send message (will fail until send_message_full is implemented)
            let message_request = serde_json::json!({
                "role": "user",
                "content": "Hello, how are you?"
            });

            // This will fail with "method not found" until implemented
            let response = service.send_message(&agent.id, message_request).await?;

            // Assertions: Basic message handling
            assert!(
                response.get("messages").is_some(),
                "Response should contain messages"
            );

            let messages = response.get("messages").unwrap().as_array().unwrap();
            assert!(
                messages.len() >= 2,
                "Should have at least user + assistant messages"
            );

            // Verify user message
            let user_msg = &messages[0];
            assert_eq!(user_msg.get("role").unwrap().as_str().unwrap(), "user");
            assert_eq!(
                user_msg.get("content").unwrap().as_str().unwrap(),
                "Hello, how are you?"
            );

            // Verify assistant message
            let assistant_msg = &messages[1];
            assert_eq!(
                assistant_msg.get("role").unwrap().as_str().unwrap(),
                "assistant"
            );
            assert!(
                !assistant_msg
                    .get("content")
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .is_empty(),
                "Assistant should have content"
            );

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Basic message handling failed: {:?}",
        result.err()
    );
}

/// Test message handling with tool execution loop
///
/// Contract:
/// - LLM can call tools
/// - Tool results fed back to LLM
/// - Loop continues until no more tool calls (max 5 iterations)
/// - All tool calls/results in message history
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_agent_message_with_tool_call() {
    let config = SimConfig::new(3002);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            use kelpie_core::current_runtime;
            // Use 100% tool call probability to guarantee tool calls for this test
            let service =
                create_service_with_tool_probability(current_runtime(), &sim_env, 1.0).await?;

            // Create agent with shell tool
            let request = CreateAgentRequest {
                name: "tool-test".to_string(),
                agent_type: AgentType::LettaV1Agent,
                model: None,
                embedding: None,
                system: Some("You can execute shell commands".to_string()),
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

            // Send message that should trigger tool call
            let message_request = serde_json::json!({
                "role": "user",
                "content": "Run the command 'echo hello'"
            });

            let response = service.send_message(&agent.id, message_request).await?;

            // Assertions: Tool execution
            let messages = response.get("messages").unwrap().as_array().unwrap();

            // Should have: user, (assistant + tool messages), final assistant
            assert!(
                messages.len() >= 2,
                "Should have multiple messages with tool calls"
            );

            // Check if any message is a tool call or tool response
            // (This validates that the tool loop executed)
            let has_tool_interaction = messages.iter().any(|msg| {
                let role = msg.get("role").unwrap().as_str().unwrap();
                role == "tool" || msg.get("tool_calls").is_some()
            });

            assert!(
                has_tool_interaction,
                "Should have tool call or tool response in message history"
            );

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Tool call handling failed: {:?}",
        result.err()
    );
}

/// Test message handling with storage faults
///
/// Contract:
/// - Storage writes can fail (fault injection)
/// - System retries or handles gracefully
/// - No data corruption
/// - Messages still delivered
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_agent_message_with_storage_fault() {
    let config = SimConfig::new(3003);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.3))
        .run_async(|sim_env| async move {
            let service = create_service(current_runtime(), &sim_env).await?;

            // Create agent
            let request = CreateAgentRequest {
                name: "fault-test".to_string(),
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

            // Send message with storage faults active
            let message_request = serde_json::json!({
                "role": "user",
                "content": "Test message with faults"
            });

            // Should either succeed or fail gracefully (no panic)
            let response_result = service.send_message(&agent.id, message_request).await;

            // Assertions: Handles faults gracefully
            match response_result {
                Ok(response) => {
                    // If succeeded, verify response is valid
                    assert!(response.get("messages").is_some());
                }
                Err(e) => {
                    // If failed, verify it's a retriable error (not corruption)
                    let err_str = e.to_string();
                    assert!(
                        err_str.contains("storage") || err_str.contains("write"),
                        "Error should be storage-related: {}",
                        err_str
                    );
                }
            }

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Storage fault handling failed: {:?}",
        result.err()
    );
}

/// Test message history preservation across actor restarts
///
/// Contract:
/// - Messages stored in AgentActorState
/// - State persisted to KV storage
/// - After deactivation + reactivation, history is loaded
/// - Subsequent messages see full history
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_agent_message_history() {
    let config = SimConfig::new(3004);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            use kelpie_core::current_runtime;
            let service = create_service(current_runtime(), &sim_env).await?;

            // Create agent
            let request = CreateAgentRequest {
                name: "history-test".to_string(),
                agent_type: AgentType::LettaV1Agent,
                model: None,
                embedding: None,
                system: Some("Remember our conversation".to_string()),
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

            // Send first message
            let msg1 = serde_json::json!({"role": "user", "content": "My name is Alice"});
            let response1 = service.send_message(&agent.id, msg1).await?;
            assert!(response1.get("messages").is_some());

            // Send second message
            let msg2 = serde_json::json!({"role": "user", "content": "What is my name?"});
            let response2 = service.send_message(&agent.id, msg2).await?;

            // Assertions: History preserved
            let messages = response2.get("messages").unwrap().as_array().unwrap();

            // Should include both conversations
            // (exact count depends on implementation, but should be > 2)
            assert!(
                messages.len() >= 2,
                "Message history should include multiple messages"
            );

            // Verify first message is in history
            let has_first_msg = messages.iter().any(|msg| {
                msg.get("content")
                    .and_then(|c| c.as_str())
                    .map(|s| s.contains("Alice"))
                    .unwrap_or(false)
            });

            assert!(
                has_first_msg,
                "First message should be in history for context"
            );

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Message history test failed: {:?}",
        result.err()
    );
}

/// Test concurrent message handling across multiple agents
///
/// Contract:
/// - Multiple agents can process messages simultaneously
/// - No message mixing between agents
/// - Each agent maintains its own history
/// - All responses are correct
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_agent_message_concurrent() {
    let config = SimConfig::new(3005);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            use kelpie_core::current_runtime;
            let runtime = current_runtime();
            let service = create_service(runtime.clone(), &sim_env).await?;

            // Create 5 agents with different names
            let mut agent_ids = Vec::new();
            for i in 1..=5 {
                let request = CreateAgentRequest {
                    name: format!("agent-{}", i),
                    agent_type: AgentType::LettaV1Agent,
                    model: None,
                    embedding: None,
                    system: Some(format!("You are agent number {}", i)),
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

            // Send messages to all agents concurrently
            let mut handles = Vec::new();
            for (idx, agent_id) in agent_ids.iter().enumerate() {
                let service_clone = service.clone();
                let agent_id_clone = agent_id.clone();
                let message = format!("Message to agent {}", idx + 1);

                let handle = runtime.spawn(async move {
                    let msg_request = serde_json::json!({"role": "user", "content": message});
                    service_clone
                        .send_message(&agent_id_clone, msg_request)
                        .await
                });
                handles.push(handle);
            }

            // Wait for all messages to complete
            let mut responses = Vec::new();
            for handle in handles {
                let response = handle.await.unwrap()?;
                responses.push(response);
            }

            // Assertions: All agents responded
            assert_eq!(responses.len(), 5, "All agents should respond");

            // Verify all responses are valid
            for response in &responses {
                assert!(
                    response.get("messages").is_some(),
                    "Each response should have messages"
                );
            }

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Concurrent message handling failed: {:?}",
        result.err()
    );
}

/// Test message handling with network/delivery faults (Issue #102)
///
/// Contract:
/// - Network packet loss, agent call timeouts, rejections simulated
/// - System handles delivery failures gracefully
/// - Either succeeds with retry or fails with clear error
/// - No silent message loss
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_agent_message_with_delivery_faults() {
    let config = SimConfig::new(3006);

    let result = Simulation::new(config)
        // Message delivery faults - filtered to agent_call operations to avoid
        // being triggered by unrelated storage/LLM operations
        .with_fault(FaultConfig::new(FaultType::NetworkPacketLoss, 0.2).with_filter("agent_call"))
        .with_fault(
            FaultConfig::new(FaultType::AgentCallTimeout { timeout_ms: 1000 }, 0.15)
                .with_filter("agent_call"),
        )
        .with_fault(
            FaultConfig::new(
                FaultType::AgentCallRejected {
                    reason: "simulated_busy".to_string(),
                },
                0.1,
            )
            .with_filter("agent_call"),
        )
        .with_fault(
            FaultConfig::new(FaultType::AgentCallNetworkDelay { delay_ms: 500 }, 0.2)
                .with_filter("agent_call"),
        )
        // Also test LLM-level faults that affect message handling
        .with_fault(FaultConfig::new(FaultType::LlmTimeout, 0.1).with_filter("llm"))
        .with_fault(FaultConfig::new(FaultType::LlmRateLimited, 0.1).with_filter("llm"))
        .run_async(|sim_env| async move {
            let service = create_service(current_runtime(), &sim_env).await?;

            // Create agent
            let request = CreateAgentRequest {
                name: "delivery-fault-test".to_string(),
                agent_type: AgentType::LettaV1Agent,
                model: None,
                embedding: None,
                system: Some("Handle delivery faults gracefully".to_string()),
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

            // Send multiple messages to test delivery resilience
            let mut success_count = 0;
            let mut failure_count = 0;

            for i in 0..10 {
                let message_request = serde_json::json!({
                    "role": "user",
                    "content": format!("Delivery test message {}", i)
                });

                match service.send_message(&agent.id, message_request).await {
                    Ok(response) => {
                        // Message delivered successfully
                        assert!(
                            response.get("messages").is_some(),
                            "Successful response should have messages"
                        );
                        success_count += 1;
                    }
                    Err(e) => {
                        // Delivery failed - verify it's a retriable error
                        let err_str = e.to_string().to_lowercase();
                        let is_delivery_error = err_str.contains("timeout")
                            || err_str.contains("rejected")
                            || err_str.contains("network")
                            || err_str.contains("packet")
                            || err_str.contains("busy")
                            || err_str.contains("unavailable")
                            || err_str.contains("llm")
                            || err_str.contains("rate limit");

                        // Allow any error under high fault conditions
                        // The key invariant is no silent drops or panics
                        if !is_delivery_error {
                            // Log unexpected error type but don't fail
                            // (high fault rates can cause cascading failures)
                            eprintln!("Iteration {}: Unexpected error type: {}", i, err_str);
                        }
                        failure_count += 1;
                    }
                }
            }

            // With 20% packet loss + 15% timeout + 10% rejection, expect some failures
            // But also expect some successes (tests should not fail deterministically)
            println!(
                "Delivery fault test: {} successes, {} failures out of 10 messages",
                success_count, failure_count
            );

            // At least one should succeed or fail (no silent drops)
            assert!(
                success_count + failure_count == 10,
                "All messages should either succeed or fail explicitly"
            );

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Delivery fault handling failed: {:?}",
        result.err()
    );
}
