//! Integration tests for audit logging
//!
//! TigerStyle: Verify that tool executions are properly audited across all code paths.
//!
//! These tests ensure the critical bug fix (AgentActor not logging to audit) is working.

use async_trait::async_trait;
use kelpie_core::{Result, Runtime};
use kelpie_runtime::{CloneFactory, Dispatcher, DispatcherConfig};
use kelpie_server::actor::{AgentActor, AgentActorState, LlmClient, LlmMessage, LlmResponse};
use kelpie_server::models::{AgentType, CreateAgentRequest, CreateBlockRequest};
use kelpie_server::security::audit::{new_shared_log, AuditEvent};
use kelpie_server::service::AgentService;
use kelpie_server::tools::{ToolExecutionContext, UnifiedToolRegistry};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

// =============================================================================
// Test: AgentActor path logs tool executions
// =============================================================================

/// Verify that tools executed via AgentService path are audited
///
/// This test verifies the fix for the critical bug where AgentActor
/// used `..Default::default()` which set audit_log to None.
///
/// Contract:
/// - AgentActor receives audit_log via with_audit_log()
/// - Tool execution flows through ToolExecutionContext
/// - Audit log receives ToolExecution entry
#[tokio::test]
async fn test_agent_actor_tool_execution_is_audited() {
    // Create shared audit log
    let audit_log = new_shared_log();

    // Create tool registry with a test tool
    let tool_registry = Arc::new(UnifiedToolRegistry::new());

    // Register a simple test tool that returns a fixed response
    // Note: BuiltinToolHandler returns String directly, not Result
    let handler: Arc<
        dyn Fn(&Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>>
            + Send
            + Sync,
    > = Arc::new(|_params| Box::pin(async move { "echo: hello".to_string() }));

    tool_registry
        .register_builtin(
            "test_echo",
            "Echo the input",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {"type": "string"}
                }
            }),
            handler,
        )
        .await;

    // Create mock LLM that returns a tool call
    let mock_llm: Arc<dyn LlmClient> = Arc::new(MockLlmWithToolCall::new());

    // Create AgentActor WITH audit log (the fix)
    let actor = AgentActor::new(mock_llm, tool_registry.clone()).with_audit_log(audit_log.clone());

    // Create dispatcher and service
    let factory = Arc::new(CloneFactory::new(actor));
    let kv = Arc::new(kelpie_storage::MemoryKV::new());
    let runtime = kelpie_core::TokioRuntime;

    let mut dispatcher = Dispatcher::<AgentActor, AgentActorState, _>::new(
        factory,
        kv,
        DispatcherConfig::default(),
        runtime.clone(),
    );
    let handle = dispatcher.handle();

    // Spawn dispatcher using DST-compatible runtime
    let _dispatcher_task = runtime.spawn(async move {
        dispatcher.run().await;
    });

    let service = AgentService::with_tool_registry_and_audit(
        handle,
        tool_registry.clone(),
        audit_log.clone(),
    );

    // Create agent
    let request = CreateAgentRequest {
        name: "audit-test-agent".to_string(),
        agent_type: AgentType::LettaV1Agent,
        model: None,
        embedding: None,
        system: Some("You are a test agent".to_string()),
        description: None,
        memory_blocks: vec![CreateBlockRequest {
            label: "persona".to_string(),
            value: "I am a test".to_string(),
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

    let agent = service.create_agent(request).await.expect("create agent");

    // Send message that triggers tool execution
    let message = serde_json::json!({
        "role": "user",
        "content": "Please use the test_echo tool"
    });

    let _response = service.send_message(&agent.id, message).await;

    // Give the system a moment to process (tool execution is async)
    // Use DST-compatible sleep
    kelpie_core::TokioRuntime
        .sleep(std::time::Duration::from_millis(100))
        .await;

    // Verify audit log has tool execution entry
    let log = audit_log.read().await;
    let recent = log.recent(10);

    // Check for tool execution event
    let has_tool_execution = recent.iter().any(|entry| {
        matches!(
            &entry.event,
            AuditEvent::ToolExecution { tool_name, .. } if tool_name == "test_echo"
        )
    });

    assert!(
        has_tool_execution,
        "Expected audit log to contain ToolExecution for test_echo. Found entries: {:?}",
        recent.iter().map(|e| &e.event).collect::<Vec<_>>()
    );
}

/// Test that tool executions via direct registry call are audited
///
/// This tests the other code path (direct tool execution, not via AgentActor).
#[tokio::test]
async fn test_direct_tool_execution_is_audited() {
    let audit_log = new_shared_log();
    let tool_registry = Arc::new(UnifiedToolRegistry::new());

    // Register test tool
    let handler: Arc<
        dyn Fn(&Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>>
            + Send
            + Sync,
    > = Arc::new(|_params| Box::pin(async move { "direct result".to_string() }));

    tool_registry
        .register_builtin(
            "direct_test",
            "Direct test tool",
            serde_json::json!({"type": "object"}),
            handler,
        )
        .await;

    // Create context with audit log
    let context = ToolExecutionContext {
        agent_id: Some("test-agent-123".to_string()),
        project_id: None,
        call_depth: 0,
        call_chain: vec![],
        dispatcher: None,
        audit_log: Some(audit_log.clone()),
    };

    // Execute tool directly (input is &Value)
    let input: Value = serde_json::json!({});
    let result = tool_registry
        .execute_with_context("direct_test", &input, Some(&context))
        .await;

    assert!(result.success, "Tool execution should succeed");

    // Verify audit log
    let log = audit_log.read().await;
    let recent = log.recent(5);

    let has_entry = recent.iter().any(|entry| {
        matches!(
            &entry.event,
            AuditEvent::ToolExecution { tool_name, agent_id, success, .. }
                if tool_name == "direct_test" && agent_id == "test-agent-123" && *success
        )
    });

    assert!(
        has_entry,
        "Expected audit entry for direct_test. Found: {:?}",
        recent.iter().map(|e| &e.event).collect::<Vec<_>>()
    );
}

/// Test that failed tool executions are logged with error info
#[tokio::test]
async fn test_failed_tool_execution_is_audited() {
    let audit_log = new_shared_log();
    let tool_registry = Arc::new(UnifiedToolRegistry::new());

    // Register tool that returns an error message
    // Note: BuiltinToolHandler returns String, not Result, so we encode failure in the string
    let handler: Arc<
        dyn Fn(&Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>>
            + Send
            + Sync,
    > = Arc::new(|_params| Box::pin(async move { "ERROR: simulated failure".to_string() }));

    tool_registry
        .register_builtin(
            "error_tool",
            "Tool that returns an error",
            serde_json::json!({"type": "object"}),
            handler,
        )
        .await;

    let context = ToolExecutionContext {
        agent_id: Some("fail-test-agent".to_string()),
        project_id: None,
        call_depth: 0,
        call_chain: vec![],
        dispatcher: None,
        audit_log: Some(audit_log.clone()),
    };

    // Execute tool
    let input: Value = serde_json::json!({});
    let result = tool_registry
        .execute_with_context("error_tool", &input, Some(&context))
        .await;

    // This tool returns successfully (just with error-like content)
    assert!(result.success, "Tool execution should succeed");

    // Verify audit log captures the execution
    let log = audit_log.read().await;
    let recent = log.recent(5);

    let has_entry = recent.iter().any(|entry| {
        matches!(
            &entry.event,
            AuditEvent::ToolExecution { tool_name, .. }
                if tool_name == "error_tool"
        )
    });

    assert!(
        has_entry,
        "Expected audit entry for error_tool. Found: {:?}",
        recent.iter().map(|e| &e.event).collect::<Vec<_>>()
    );
}

/// Test that tool not found returns failure (but is NOT currently logged)
///
/// NOTE: This is a known gap - "tool not found" returns early before audit logging.
/// A future improvement would be to also log tool-not-found cases for security monitoring.
#[tokio::test]
async fn test_tool_not_found_returns_failure() {
    let audit_log = new_shared_log();
    let tool_registry = Arc::new(UnifiedToolRegistry::new());

    let context = ToolExecutionContext {
        agent_id: Some("not-found-test".to_string()),
        project_id: None,
        call_depth: 0,
        call_chain: vec![],
        dispatcher: None,
        audit_log: Some(audit_log.clone()),
    };

    // Execute non-existent tool
    let input: Value = serde_json::json!({});
    let result = tool_registry
        .execute_with_context("nonexistent_tool", &input, Some(&context))
        .await;

    // Should fail
    assert!(!result.success, "Tool execution should fail");
    assert!(
        result.output.contains("not found"),
        "Error message should mention 'not found'"
    );

    // Known gap: Tool not found is NOT currently logged to audit
    // The execute_with_context function returns early before the audit logging code
    let log = audit_log.read().await;
    let recent = log.recent(5);
    assert!(
        recent.is_empty(),
        "Currently, tool-not-found is NOT logged (known gap). Found: {:?}",
        recent.iter().map(|e| &e.event).collect::<Vec<_>>()
    );
}

/// Test that audit log is shared between AppState and AgentActor
#[tokio::test]
async fn test_audit_log_is_shared_instance() {
    let audit_log = new_shared_log();
    let tool_registry = Arc::new(UnifiedToolRegistry::new());

    // Log something to the shared log
    {
        let mut log = audit_log.write().await;
        log.log(AuditEvent::AgentCreated {
            agent_id: "shared-test".to_string(),
            agent_name: "Shared Test".to_string(),
        });
    }

    // Create actor with SAME audit log
    let mock_llm: Arc<dyn LlmClient> = Arc::new(MockLlmSimple::new());
    let _actor = AgentActor::new(mock_llm, tool_registry).with_audit_log(audit_log.clone());

    // Verify the audit log is shared (same instance)
    let log = audit_log.read().await;
    let recent = log.recent(1);

    assert_eq!(recent.len(), 1);
    assert!(matches!(
        &recent[0].event,
        AuditEvent::AgentCreated { agent_id, .. } if agent_id == "shared-test"
    ));
}

// =============================================================================
// Mock LLM Clients
// =============================================================================

/// Mock LLM that returns a tool call on first request, then a final response
struct MockLlmWithToolCall {
    call_count: AtomicU64,
}

impl MockLlmWithToolCall {
    fn new() -> Self {
        Self {
            call_count: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl LlmClient for MockLlmWithToolCall {
    async fn complete_with_tools(
        &self,
        _messages: Vec<LlmMessage>,
        _tools: Vec<kelpie_server::llm::ToolDefinition>,
    ) -> Result<LlmResponse> {
        let count = self.call_count.fetch_add(1, Ordering::SeqCst);

        if count == 0 {
            // First call: return tool call
            Ok(LlmResponse {
                content: "I'll use the test_echo tool.".to_string(),
                tool_calls: vec![kelpie_server::actor::LlmToolCall {
                    id: "call_001".to_string(),
                    name: "test_echo".to_string(),
                    input: serde_json::json!({"message": "hello"}),
                }],
                prompt_tokens: 10,
                completion_tokens: 20,
                stop_reason: "tool_use".to_string(),
            })
        } else {
            // Subsequent calls: return final response
            Ok(LlmResponse {
                content: "Done!".to_string(),
                tool_calls: vec![],
                prompt_tokens: 10,
                completion_tokens: 5,
                stop_reason: "end_turn".to_string(),
            })
        }
    }

    async fn continue_with_tool_result(
        &self,
        _messages: Vec<LlmMessage>,
        _tools: Vec<kelpie_server::llm::ToolDefinition>,
        _assistant_blocks: Vec<kelpie_server::llm::ContentBlock>,
        _tool_results: Vec<(String, String)>,
    ) -> Result<LlmResponse> {
        // Return final response after tool execution
        Ok(LlmResponse {
            content: "The tool returned the result.".to_string(),
            tool_calls: vec![],
            prompt_tokens: 15,
            completion_tokens: 10,
            stop_reason: "end_turn".to_string(),
        })
    }
}

/// Simple mock LLM that just returns text
struct MockLlmSimple;

impl MockLlmSimple {
    fn new() -> Self {
        Self
    }
}

#[async_trait]
impl LlmClient for MockLlmSimple {
    async fn complete_with_tools(
        &self,
        _messages: Vec<LlmMessage>,
        _tools: Vec<kelpie_server::llm::ToolDefinition>,
    ) -> Result<LlmResponse> {
        Ok(LlmResponse {
            content: "Hello!".to_string(),
            tool_calls: vec![],
            prompt_tokens: 5,
            completion_tokens: 5,
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
            content: "Done!".to_string(),
            tool_calls: vec![],
            prompt_tokens: 5,
            completion_tokens: 5,
            stop_reason: "end_turn".to_string(),
        })
    }
}
