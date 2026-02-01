//! DST coverage for Letta compatibility features (phases 3-9)
//!
//! TigerStyle: Deterministic simulations with fault injection.

#![cfg(feature = "dst")]

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use chrono::TimeZone;
use kelpie_core::Error;
use kelpie_core::TimeProvider;
use kelpie_dst::{FaultConfig, FaultType, SimConfig, Simulation};
use kelpie_server::api;
use kelpie_server::http::{HttpClient, HttpRequest, HttpResponse};
use kelpie_server::llm::{LlmClient, LlmConfig};
use kelpie_server::models::{CreateAgentRequest, MessageRole};
use kelpie_server::state::AppState;
use kelpie_server::storage::KvAdapter;
use kelpie_server::tools::{
    register_memory_tools, register_run_code_tool, register_web_search_tool,
};
use serde_json::json;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use tower::ServiceExt;

fn mock_anthropic_response() -> String {
    serde_json::json!({
        "content": [
            {"type": "text", "text": "Summary: OK"}
        ],
        "usage": {"input_tokens": 10, "output_tokens": 5},
        "stop_reason": "end_turn"
    })
    .to_string()
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock poisoned")
}

/// FaultInjectedHttpClient replaces StubHttpClient with full DST fault injection support.
///
/// TigerStyle: Supports NetworkDelay, HttpConnectionFail, HttpTimeout, HttpServerError,
/// LlmTimeout, and LlmRateLimited faults for deterministic testing.
struct FaultInjectedHttpClient {
    faults: Arc<kelpie_dst::FaultInjector>,
    time: Arc<dyn TimeProvider>,
    rng: Arc<kelpie_dst::DeterministicRng>,
}

#[async_trait]
impl HttpClient for FaultInjectedHttpClient {
    async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, String> {
        // Check for fault injection
        if let Some(fault) = self.faults.should_inject("http_send") {
            match fault {
                FaultType::NetworkDelay { min_ms, max_ms } => {
                    let delay = min_ms + (self.rng.next_u64() % (max_ms - min_ms + 1));
                    self.time.sleep_ms(delay).await;
                }
                FaultType::HttpConnectionFail => {
                    return Err("Connection failed (fault injected)".to_string());
                }
                FaultType::HttpTimeout { timeout_ms } => {
                    self.time.sleep_ms(timeout_ms).await;
                    return Err(format!("Timeout after {}ms (fault injected)", timeout_ms));
                }
                FaultType::HttpServerError { status } => {
                    return Ok(HttpResponse {
                        status,
                        headers: HashMap::new(),
                        body: format!("Server error {} (fault injected)", status).into_bytes(),
                    });
                }
                FaultType::LlmTimeout => {
                    return Err("LLM request timed out".to_string());
                }
                FaultType::LlmRateLimited => {
                    return Err("LLM rate limited".to_string());
                }
                _ => {}
            }
        }

        Ok(HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: mock_anthropic_response().into_bytes(),
        })
    }

    async fn send_streaming(
        &self,
        _request: HttpRequest,
    ) -> Result<Pin<Box<dyn futures::stream::Stream<Item = Result<Bytes, String>> + Send>>, String>
    {
        // Check for fault injection on streaming requests
        if let Some(fault) = self.faults.should_inject("http_send_streaming") {
            match fault {
                FaultType::HttpConnectionFail => {
                    return Err("Connection failed (fault injected)".to_string());
                }
                FaultType::HttpTimeout { timeout_ms } => {
                    self.time.sleep_ms(timeout_ms).await;
                    return Err(format!(
                        "Streaming timeout after {}ms (fault injected)",
                        timeout_ms
                    ));
                }
                FaultType::LlmTimeout => {
                    return Err("LLM streaming request timed out".to_string());
                }
                _ => {}
            }
        }
        Err("streaming not supported in FaultInjectedHttpClient".to_string())
    }
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_summarization_with_llm_faults() {
    let config = SimConfig::new(8801);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::LlmTimeout, 0.4).with_filter("http_send"))
        .with_fault(FaultConfig::new(FaultType::LlmRateLimited, 0.2).with_filter("http_send"))
        .run_async(|sim_env| async move {
            let sim_http = Arc::new(FaultInjectedHttpClient {
                faults: sim_env.faults.clone(),
                time: sim_env.io_context.time.clone(),
                rng: Arc::new(sim_env.rng.fork()),
            });
            let llm_config = LlmConfig {
                base_url: "http://example.com".to_string(),
                api_key: "test-key".to_string(),
                model: "claude-test".to_string(),
                max_tokens: 128,
            };
            let llm = LlmClient::with_http_client(llm_config, sim_http);
            let state = AppState::with_llm(kelpie_core::current_runtime(), llm);

            let agent = state
                .create_agent_async(CreateAgentRequest {
                    name: "summary-agent".to_string(),
                    agent_type: kelpie_server::models::AgentType::LettaV1Agent,
                    model: None,
                    embedding: None,
                    system: Some("You summarize".to_string()),
                    description: None,
                    memory_blocks: vec![],
                    block_ids: vec![],
                    tool_ids: vec![],
                    tags: vec![],
                    metadata: serde_json::json!({}),
                    project_id: None,
                    user_id: None,
                    org_id: None,
                })
                .await
                .map_err(|e| Error::Internal {
                    message: format!("create_agent_async failed: {}", e),
                })?;

            // Use simulated time for determinism
            // TigerStyle: No fallback to chrono::Utc::now() - that would break determinism
            let sim_time_ms = sim_env.io_context.time.now_ms() as i64;
            let created_at = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(sim_time_ms)
                .expect("timestamp conversion failed - sim_time_ms out of range for chrono");

            state
                .add_message(
                    &agent.id,
                    kelpie_server::models::Message {
                        id: "msg-1".to_string(),
                        agent_id: agent.id.clone(),
                        message_type: "user_message".to_string(),
                        role: MessageRole::User,
                        content: "Summarize this".to_string(),
                        tool_call_id: None,
                        tool_calls: vec![],
                        tool_call: None,
                        tool_return: None,
                        status: None,
                        created_at,
                    },
                )
                .map_err(|e| Error::Internal {
                    message: format!("add_message failed: {}", e),
                })?;

            let app = api::router(state);
            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/v1/agents/{}/messages/summarize", agent.id))
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::json!({
                                "message_count": 1
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();

            match response.status() {
                StatusCode::OK => {}
                StatusCode::INTERNAL_SERVER_ERROR => {}
                _ => {
                    panic!("unexpected response status: {}", response.status());
                }
            }

            Ok(())
        })
        .await;

    assert!(result.is_ok());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_scheduling_job_write_fault() {
    let config = SimConfig::new(8802);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 1.0).with_filter("job_write"))
        .run_async(|_sim_env| async move {
            let state = AppState::with_fault_injector(
                kelpie_core::current_runtime(),
                _sim_env.faults.clone(),
            );
            let app = api::router(state);

            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/jobs")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::json!({
                                "agent_id": "missing-agent",
                                "schedule_type": "interval",
                                "schedule": "60",
                                "action": "summarize_conversation"
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();

            assert!(
                response.status() == StatusCode::BAD_REQUEST
                    || response.status() == StatusCode::INTERNAL_SERVER_ERROR
                    || response.status() == StatusCode::NOT_FOUND
            );

            Ok(())
        })
        .await;

    assert!(result.is_ok());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_projects_write_fault() {
    let config = SimConfig::new(8803);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 1.0).with_filter("project_write"))
        .run_async(|sim_env| async move {
            let state = AppState::with_fault_injector(
                kelpie_core::current_runtime(),
                sim_env.faults.clone(),
            );
            let app = api::router(state);

            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/projects")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::json!({
                                "name": "Faulty Project"
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();

            assert!(
                response.status() == StatusCode::INTERNAL_SERVER_ERROR
                    || response.status() == StatusCode::BAD_REQUEST
            );

            Ok(())
        })
        .await;

    assert!(result.is_ok());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_batch_status_write_fault() {
    let config = SimConfig::new(8804);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 1.0).with_filter("batch_write"))
        .run_async(|sim_env| async move {
            let state = AppState::with_fault_injector(
                kelpie_core::current_runtime(),
                sim_env.faults.clone(),
            );
            let app = api::router(state);

            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/agents/test-agent/messages/batch")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::json!({
                                "messages": [
                                    {"content": "hello"}
                                ]
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();

            assert!(
                response.status() == StatusCode::NOT_FOUND
                    || response.status() == StatusCode::INTERNAL_SERVER_ERROR
                    || response.status() == StatusCode::BAD_REQUEST
            );

            Ok(())
        })
        .await;

    assert!(result.is_ok());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_agent_group_write_fault() {
    let config = SimConfig::new(8805);

    let result = Simulation::new(config)
        .with_fault(
            FaultConfig::new(FaultType::StorageWriteFail, 1.0).with_filter("agent_group_write"),
        )
        .run_async(|sim_env| async move {
            let state = AppState::with_fault_injector(
                kelpie_core::current_runtime(),
                sim_env.faults.clone(),
            );
            let app = api::router(state);

            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/agent-groups")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::json!({
                                "name": "Group A",
                                "agent_ids": []
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();

            assert!(
                response.status() == StatusCode::INTERNAL_SERVER_ERROR
                    || response.status() == StatusCode::BAD_REQUEST
            );

            Ok(())
        })
        .await;

    assert!(result.is_ok());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_custom_tool_storage_fault() {
    let config = SimConfig::new(8806);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 1.0).with_filter("storage_write"))
        .run_async(|sim_env| async move {
            let adapter = KvAdapter::with_dst_storage(sim_env.rng.fork(), sim_env.faults.clone());
            let storage = Arc::new(adapter);
            let state = AppState::with_storage_and_faults(
                kelpie_core::current_runtime(),
                storage,
                sim_env.faults.clone(),
            );

            let result = state
                .register_tool(
                    "faulty_tool".to_string(),
                    "desc".to_string(),
                    serde_json::json!({"type": "object"}),
                    "def faulty_tool() -> str:\n    return \"ok\"\n".to_string(),
                    "python".to_string(),
                    vec![],
                )
                .await;

            assert!(result.is_err());
            Ok(())
        })
        .await;

    assert!(result.is_ok());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_conversation_search_date_with_faults() {
    let config = SimConfig::new(8807);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.5).with_filter("storage_read"))
        .run_async(|sim_env| async move {
            let state = AppState::with_fault_injector(
                kelpie_core::current_runtime(),
                sim_env.faults.clone(),
            );
            let registry = state.tool_registry();
            register_memory_tools(registry, state.clone()).await;

            let agent = state
                .create_agent_async(CreateAgentRequest {
                    name: "date-search-agent".to_string(),
                    agent_type: kelpie_server::models::AgentType::LettaV1Agent,
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
                })
                .await
                .map_err(|e| Error::Internal {
                    message: format!("create_agent_async failed: {}", e),
                })?;

            let in_range_time = chrono::Utc.with_ymd_and_hms(2025, 1, 10, 12, 0, 0).unwrap();
            let out_range_time = chrono::Utc.with_ymd_and_hms(2024, 1, 10, 12, 0, 0).unwrap();

            state
                .add_message(
                    &agent.id,
                    kelpie_server::models::Message {
                        id: "msg-in-range".to_string(),
                        agent_id: agent.id.clone(),
                        message_type: "user_message".to_string(),
                        role: MessageRole::User,
                        content: "hello in range".to_string(),
                        tool_call_id: None,
                        tool_calls: vec![],
                        tool_call: None,
                        tool_return: None,
                        status: None,
                        created_at: in_range_time,
                    },
                )
                .map_err(|e| Error::Internal {
                    message: format!("add_message failed: {}", e),
                })?;

            state
                .add_message(
                    &agent.id,
                    kelpie_server::models::Message {
                        id: "msg-out-range".to_string(),
                        agent_id: agent.id.clone(),
                        message_type: "user_message".to_string(),
                        role: MessageRole::User,
                        content: "hello old".to_string(),
                        tool_call_id: None,
                        tool_calls: vec![],
                        tool_call: None,
                        tool_return: None,
                        status: None,
                        created_at: out_range_time,
                    },
                )
                .map_err(|e| Error::Internal {
                    message: format!("add_message failed: {}", e),
                })?;

            let output = registry
                .execute(
                    "conversation_search_date",
                    &json!({
                        "agent_id": agent.id,
                        "query": "hello",
                        "start_date": "2025-01-01T00:00:00Z",
                        "end_date": "2025-12-31T23:59:59Z"
                    }),
                )
                .await;

            if output.output.contains("Error: failed to list messages") {
                assert!(output.output.contains("message_read"));
            } else {
                assert!(output.output.contains("Found 1 results"));
                assert!(output.output.contains("hello in range"));
                assert!(!output.output.contains("hello old"));
            }

            Ok(())
        })
        .await;

    assert!(result.is_ok());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_web_search_missing_api_key() {
    let config = SimConfig::new(8808);

    let result = Simulation::new(config)
        .run_async(|_sim_env| async move {
            let state;
            let registry;
            {
                let _guard = env_lock();
                let prev_key = std::env::var("TAVILY_API_KEY").ok();
                std::env::set_var("TAVILY_API_KEY", "");

                state = AppState::new(kelpie_core::current_runtime());
                registry = state.tool_registry();

                if let Some(prev) = prev_key {
                    std::env::set_var("TAVILY_API_KEY", prev);
                } else {
                    std::env::remove_var("TAVILY_API_KEY");
                }
            }

            register_web_search_tool(registry).await;

            let output = registry
                .execute(
                    "web_search",
                    &json!({
                        "query": "kelpie letta compatibility"
                    }),
                )
                .await;

            assert!(output.output.contains("TAVILY_API_KEY"));
            Ok(())
        })
        .await;

    assert!(result.is_ok());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_run_code_unsupported_language() {
    let config = SimConfig::new(8809);

    let result = Simulation::new(config)
        .run_async(|_sim_env| async move {
            let state = AppState::new(kelpie_core::current_runtime());
            let registry = state.tool_registry();
            register_run_code_tool(registry).await;

            let output = registry
                .execute(
                    "run_code",
                    &json!({
                        "language": "java",
                        "code": "System.out.println(\"Hello\");"
                    }),
                )
                .await;

            assert!(output
                .output
                .contains("Java execution requires compilation"));
            Ok(())
        })
        .await;

    assert!(result.is_ok());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_export_with_message_read_fault() {
    let config = SimConfig::new(8810);

    let result = Simulation::new(config)
        // Messages are stored in memory, not in KvAdapter storage.
        // list_messages checks for "message_read" fault injection.
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 1.0).with_filter("message_read"))
        .run_async(|sim_env| async move {
            // Use fault injector for message read faults (in-memory message storage)
            let state = AppState::with_fault_injector(
                kelpie_core::current_runtime(),
                sim_env.faults.clone(),
            );
            let agent = state
                .create_agent_async(CreateAgentRequest {
                    name: "export-fault-agent".to_string(),
                    agent_type: kelpie_server::models::AgentType::LettaV1Agent,
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
                })
                .await
                .map_err(|e| Error::Internal {
                    message: format!("create_agent_async failed: {}", e),
                })?;

            state
                .add_message(
                    &agent.id,
                    kelpie_server::models::Message {
                        id: "msg-export".to_string(),
                        agent_id: agent.id.clone(),
                        message_type: "user_message".to_string(),
                        role: MessageRole::User,
                        content: "export message".to_string(),
                        tool_call_id: None,
                        tool_calls: vec![],
                        tool_call: None,
                        tool_return: None,
                        status: None,
                        created_at: chrono::Utc.with_ymd_and_hms(2025, 2, 1, 0, 0, 0).unwrap(),
                    },
                )
                .map_err(|e| Error::Internal {
                    message: format!("add_message failed: {}", e),
                })?;

            let app = api::router(state.clone());
            let response = app
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri(format!(
                            "/v1/agents/{}/export?include_messages=true",
                            agent.id
                        ))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            // Parse response body first to debug
            let status = response.status();
            let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let body_str = String::from_utf8_lossy(&body_bytes);

            // Export is resilient to message read failures - it returns 200 with empty messages
            // rather than propagating the error. This tests that behavior.
            assert_eq!(
                status,
                StatusCode::OK,
                "Expected 200 OK, got {} with body: {}",
                status,
                body_str
            );

            // Parse response and verify messages are empty/absent due to read fault
            // Note: messages field is skipped when empty due to #[serde(skip_serializing_if = "Vec::is_empty")]
            let export: serde_json::Value = serde_json::from_slice(&body_bytes)
                .unwrap_or_else(|_| panic!("Failed to parse response as JSON: {}", body_str));
            let messages = export.get("messages").and_then(|m| m.as_array());
            let message_count = messages.map(|m| m.len()).unwrap_or(0);
            assert!(
                message_count == 0,
                "Expected empty/absent messages due to read fault, got {} messages",
                message_count
            );

            Ok(())
        })
        .await;

    assert!(result.is_ok());
}

#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_import_with_message_write_fault() {
    let config = SimConfig::new(8811);

    let result = Simulation::new(config)
        // Messages are stored in memory, not in KvAdapter storage.
        // import_messages checks for "message_write" fault injection via add_message.
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 1.0).with_filter("message_write"))
        .run_async(|sim_env| async move {
            // Use fault injector for message write faults (in-memory message storage)
            let state = AppState::with_fault_injector(
                kelpie_core::current_runtime(),
                sim_env.faults.clone(),
            );
            let app = api::router(state);

            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/agents/import")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::json!({
                                "agent": {
                                    "name": "import-fault-agent",
                                    "agent_type": "letta_v1_agent",
                                    "blocks": [],
                                    "tool_ids": [],
                                    "tags": [],
                                    "metadata": {}
                                },
                                "messages": [
                                    {
                                        "role": "user",
                                        "content": "import message"
                                    }
                                ]
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();

            // Import is resilient to message write failures - it returns 200 with agent created
            // but logs a warning about failed message import. This tests that behavior.
            assert_eq!(response.status(), StatusCode::OK);

            // Parse response and verify agent was created successfully
            let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let agent: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
            assert!(
                agent.get("id").is_some(),
                "Expected agent to be created despite message write fault"
            );
            assert_eq!(
                agent.get("name").and_then(|n| n.as_str()),
                Some("import-fault-agent")
            );

            Ok(())
        })
        .await;

    assert!(result.is_ok());
}
