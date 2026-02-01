//! Pilot test for DST Phase 2.6.4 - Runtime integration
//!
//! This test demonstrates that AgentService works with both CurrentRuntime
//! and MadsimRuntime, proving the Runtime generic parameter integration.
//!
//! TigerStyle: Explicit test, demonstrates contract, no complex setup.

use async_trait::async_trait;
#[cfg(madsim)]
use kelpie_core::MadsimRuntime;
use kelpie_core::{current_runtime, Result, Runtime};
use kelpie_runtime::{CloneFactory, Dispatcher, DispatcherConfig};
use kelpie_server::actor::{AgentActor, AgentActorState, LlmClient, LlmMessage, LlmResponse};
use kelpie_server::models::{AgentType, CreateAgentRequest, CreateBlockRequest};
use kelpie_server::service::AgentService;
use kelpie_server::tools::UnifiedToolRegistry;
use kelpie_storage::MemoryKV;
use std::sync::Arc;

/// Mock LLM client for testing
#[derive(Clone)]
struct MockLlmClient;

#[async_trait]
impl LlmClient for MockLlmClient {
    async fn complete_with_tools(
        &self,
        _messages: Vec<LlmMessage>,
        _tools: Vec<kelpie_server::llm::ToolDefinition>,
    ) -> Result<LlmResponse> {
        Ok(LlmResponse {
            content: "Hello from mock LLM".to_string(),
            stop_reason: "end_turn".to_string(),
            tool_calls: vec![],
            prompt_tokens: 10,
            completion_tokens: 5,
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
            content: "Continued response from mock LLM".to_string(),
            stop_reason: "end_turn".to_string(),
            tool_calls: vec![],
            prompt_tokens: 10,
            completion_tokens: 5,
        })
    }
}

/// Helper to create AgentService with generic Runtime
async fn create_agent_service<R: Runtime + 'static>(runtime: R) -> Result<AgentService<R>> {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient);
    let actor = AgentActor::new(llm, Arc::new(UnifiedToolRegistry::new()));
    let factory = Arc::new(CloneFactory::new(actor));
    let kv = Arc::new(MemoryKV::new());

    let mut dispatcher = Dispatcher::<AgentActor, AgentActorState, _>::new(
        factory,
        kv,
        DispatcherConfig::default(),
        runtime.clone(),
    );
    let handle = dispatcher.handle();

    // Spawn dispatcher in background
    let _dispatcher_handle = runtime.spawn(async move {
        dispatcher.run().await;
    });

    Ok(AgentService::new(handle))
}

/// Test AgentService with CurrentRuntime (production)
#[tokio::test]
async fn test_agent_service_tokio_runtime() {
    let runtime = current_runtime();
    let service = create_agent_service(runtime).await.unwrap();

    // Create agent
    let request = CreateAgentRequest {
        name: "tokio-agent".to_string(),
        agent_type: AgentType::LettaV1Agent,
        model: None,
        embedding: None,
        system: Some("You are helpful".to_string()),
        description: None,
        memory_blocks: vec![CreateBlockRequest {
            label: "persona".to_string(),
            value: "I am helpful".to_string(),
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

    let agent_state = service.create_agent(request).await.unwrap();

    // Verify
    assert_eq!(agent_state.name, "tokio-agent");
    assert_eq!(agent_state.agent_type, AgentType::LettaV1Agent);
    assert_eq!(agent_state.blocks.len(), 1);
    assert_eq!(agent_state.blocks[0].label, "persona");
}

/// Test AgentService with MadsimRuntime (deterministic testing)
#[cfg(madsim)]
#[madsim::test]
async fn test_agent_service_madsim_runtime() {
    let runtime = MadsimRuntime;
    let service = create_agent_service(runtime).await.unwrap();

    // Create agent
    let request = CreateAgentRequest {
        name: "madsim-agent".to_string(),
        agent_type: AgentType::LettaV1Agent,
        model: None,
        embedding: None,
        system: Some("You are helpful".to_string()),
        description: None,
        memory_blocks: vec![CreateBlockRequest {
            label: "persona".to_string(),
            value: "I am helpful".to_string(),
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

    let agent_state = service.create_agent(request).await.unwrap();

    // Verify - exact same assertions as tokio test
    assert_eq!(agent_state.name, "madsim-agent");
    assert_eq!(agent_state.agent_type, AgentType::LettaV1Agent);
    assert_eq!(agent_state.blocks.len(), 1);
    assert_eq!(agent_state.blocks[0].label, "persona");
}
