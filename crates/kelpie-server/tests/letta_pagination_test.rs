// Unit test to verify pagination fix for Letta SDK compatibility
//
// Tests the `?after=<id>` parameter for cursor-based pagination

use async_trait::async_trait;
use kelpie_core::{Runtime, TokioRuntime};
use kelpie_dst::{DeterministicRng, FaultInjector, SimStorage};
use kelpie_runtime::{CloneFactory, Dispatcher, DispatcherConfig};
use kelpie_server::actor::{AgentActor, AgentActorState, LlmClient, LlmMessage, LlmResponse};
use kelpie_server::models::{AgentType, CreateAgentRequest};
use kelpie_server::service::AgentService;
use kelpie_server::state::AppState;
use kelpie_server::tools::UnifiedToolRegistry;
use std::sync::Arc;

/// Mock LLM client for testing
struct MockLlmClient;

#[async_trait]
impl LlmClient for MockLlmClient {
    async fn complete_with_tools(
        &self,
        _messages: Vec<LlmMessage>,
        _tools: Vec<kelpie_server::llm::ToolDefinition>,
    ) -> kelpie_core::Result<LlmResponse> {
        Ok(LlmResponse {
            content: "Test response".to_string(),
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
    ) -> kelpie_core::Result<LlmResponse> {
        Ok(LlmResponse {
            content: "Test response".to_string(),
            tool_calls: vec![],
            prompt_tokens: 0,
            completion_tokens: 0,
            stop_reason: "end_turn".to_string(),
        })
    }
}

/// Create test AppState with AgentService
async fn create_test_state() -> AppState<TokioRuntime> {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient);
    let actor = AgentActor::new(llm, Arc::new(UnifiedToolRegistry::new()));
    let factory = Arc::new(CloneFactory::new(actor));

    let rng = DeterministicRng::new(42);
    let faults = Arc::new(FaultInjector::new(rng.fork()));
    let storage = SimStorage::new(rng.fork(), faults);
    let kv = Arc::new(storage);

    let runtime = TokioRuntime;

    let mut dispatcher = Dispatcher::<AgentActor, AgentActorState, _>::new(
        factory,
        kv,
        DispatcherConfig::default(),
        runtime.clone(),
    );
    let handle = dispatcher.handle();

    drop(runtime.spawn(async move {
        dispatcher.run().await;
    }));

    let service = AgentService::new(handle.clone());
    AppState::with_agent_service(runtime, service, handle)
}

// TODO: These tests require implementing list_agents in AgentService
// Currently list_agents_async uses storage/HashMap directly, not AgentService
#[tokio::test]
#[ignore = "requires list_agents implementation in AgentService"]
async fn test_list_agents_pagination_with_after_cursor() {
    let state = create_test_state().await;

    // Create 5 agents
    for i in 0..5 {
        let request = CreateAgentRequest {
            name: format!("agent_{}", i),
            agent_type: AgentType::MemgptAgent,
            ..Default::default()
        };

        state.create_agent_async(request).await.unwrap();
    }

    // List all agents (should return 5)
    let (all_agents, _cursor) = state.list_agents_async(10, None, None).await.unwrap();
    assert_eq!(all_agents.len(), 5, "Should return all 5 agents");

    // Use the FIRST agent from the sorted list (not first created)
    // This ensures we're testing pagination in the middle of the list
    let first_sorted_id = &all_agents[0].id;

    // List after first sorted agent (should return remaining 4)
    let (after_agents, _cursor) = state
        .list_agents_async(10, Some(first_sorted_id), None)
        .await
        .unwrap();

    assert_eq!(
        after_agents.len(),
        4,
        "Should return 4 agents after first sorted agent"
    );

    // Verify the first agent is NOT in the results (cursor is excluded)
    assert!(
        !after_agents.iter().any(|a| a.id == *first_sorted_id),
        "Cursor agent should not be in results (cursor should be excluded)"
    );

    // List after middle agent (use 3rd agent from sorted list)
    let middle_id = &all_agents[2].id;
    let (middle_agents, _cursor) = state
        .list_agents_async(10, Some(middle_id), None)
        .await
        .unwrap();

    assert_eq!(
        middle_agents.len(),
        2,
        "Should return 2 agents after middle cursor (agents 3 and 4)"
    );

    // List after last agent (should return 0)
    let last_id = &all_agents[all_agents.len() - 1].id;
    let (end_agents, cursor) = state
        .list_agents_async(10, Some(last_id), None)
        .await
        .unwrap();

    assert_eq!(end_agents.len(), 0, "Should return 0 agents after last");
    assert!(cursor.is_none(), "Cursor should be None at end of list");
}

#[tokio::test]
#[ignore = "requires list_agents implementation in AgentService"]
async fn test_list_agents_pagination_with_limit() {
    let state = create_test_state().await;

    // Create 10 agents
    for i in 0..10 {
        let request = CreateAgentRequest {
            name: format!("agent_{}", i),
            agent_type: AgentType::MemgptAgent,
            ..Default::default()
        };

        state.create_agent_async(request).await.unwrap();
    }

    // List with limit=3 (should return 3 and next cursor)
    let (page1, cursor1) = state.list_agents_async(3, None, None).await.unwrap();

    assert_eq!(page1.len(), 3, "First page should have 3 agents");
    assert!(cursor1.is_some(), "Should have next cursor");

    // List next page using cursor
    let (page2, cursor2) = state
        .list_agents_async(3, cursor1.as_deref(), None)
        .await
        .unwrap();

    assert_eq!(page2.len(), 3, "Second page should have 3 agents");
    assert!(cursor2.is_some(), "Should have next cursor");

    // Verify no overlap between pages
    for agent in &page2 {
        assert!(
            !page1.iter().any(|a| a.id == agent.id),
            "Pages should not overlap"
        );
    }
}
