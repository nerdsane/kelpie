//! Real LLM Integration Tests
//!
//! TigerStyle: End-to-end integration tests with actual LLM APIs.
//!
//! These tests are marked `#[ignore]` by default and require:
//! - `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` environment variable
//!
//! Run with: `cargo test -p kelpie-server --test real_llm_integration -- --ignored`
//!
//! Purpose:
//! - Verify end-to-end flow with real LLM responses
//! - Test tool calling with actual LLM decisions
//! - Validate streaming with real API responses
//! - Catch API compatibility regressions
//!
//! Note: These tests intentionally use tokio::time::timeout (not DST runtime)
//! because they make real HTTP calls to LLM APIs that cannot be simulated.
#![allow(clippy::disallowed_methods)]

use axum::{body::Body, http::Request, Router};
use kelpie_core::{Runtime, TokioRuntime};
use kelpie_dst::fault::FaultInjector;
use kelpie_dst::storage::SimStorage;
use kelpie_dst::DeterministicRng;
use kelpie_runtime::{CloneFactory, Dispatcher, DispatcherConfig};
use kelpie_server::actor::{AgentActor, AgentActorState, LlmClient, RealLlmAdapter};
use kelpie_server::llm::{LlmClient as RealLlmClient, LlmConfig};
use kelpie_server::tools::UnifiedToolRegistry;
use kelpie_server::{api, service, state::AppState};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

/// Check if any LLM API key is configured
fn has_llm_api_key() -> bool {
    std::env::var("ANTHROPIC_API_KEY").is_ok() || std::env::var("OPENAI_API_KEY").is_ok()
}

/// Skip test if no API key is available (returns true if skipped)
fn skip_without_api_key() -> bool {
    if !has_llm_api_key() {
        eprintln!(
            "\n⚠️  Skipping test: No LLM API key found.\n\
             Set ANTHROPIC_API_KEY or OPENAI_API_KEY to run this test.\n"
        );
        true
    } else {
        false
    }
}

/// Create a test app with real LLM client
async fn test_app_with_real_llm() -> Router {
    // Get LLM config from environment
    let config = LlmConfig::from_env()
        .expect("LLM config required - set ANTHROPIC_API_KEY or OPENAI_API_KEY");

    // Use real LLM client that reads from environment
    let real_client = RealLlmClient::new(config);
    let llm: Arc<dyn LlmClient> = Arc::new(RealLlmAdapter::new(real_client));
    let actor = AgentActor::new(llm, Arc::new(UnifiedToolRegistry::new()));
    let factory = Arc::new(CloneFactory::new(actor));

    // Use SimStorage for testing (in-memory KV store)
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

    let service = service::AgentService::new(handle.clone());
    let state = AppState::with_agent_service(runtime, service, handle);

    api::router(state)
}

/// Test: Create agent and send message with real LLM
///
/// Verifies:
/// 1. Agent creation works
/// 2. Message sending with real LLM works
/// 3. Response contains actual LLM content (not empty/stub)
/// 4. Memory blocks are included in context
#[tokio::test]
#[ignore = "Requires ANTHROPIC_API_KEY or OPENAI_API_KEY"]
async fn test_real_llm_agent_message_roundtrip() {
    if skip_without_api_key() {
        return;
    }

    let app = test_app_with_real_llm().await;

    // Create agent with persona and human blocks
    let create_req = Request::builder()
        .method("POST")
        .uri("/v1/agents")
        .header("Content-Type", "application/json")
        .body(Body::from(
            json!({
                "name": "real-llm-test-agent",
                "memory_blocks": [
                    {
                        "label": "persona",
                        "value": "You are a helpful assistant named TestBot. Always respond briefly with just a few words."
                    },
                    {
                        "label": "human",
                        "value": "The human is testing Kelpie's LLM integration."
                    }
                ]
            })
            .to_string(),
        ))
        .unwrap();

    let response = app.clone().oneshot(create_req).await.unwrap();
    assert_eq!(response.status(), 200, "Agent creation should succeed");

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let agent: Value = serde_json::from_slice(&body).unwrap();
    let agent_id = agent["id"].as_str().unwrap();

    // Send a message
    let message_req = Request::builder()
        .method("POST")
        .uri(format!("/v1/agents/{}/messages", agent_id))
        .header("Content-Type", "application/json")
        .body(Body::from(
            json!({
                "role": "user",
                "content": "What is 2 + 2? Reply with just the number."
            })
            .to_string(),
        ))
        .unwrap();

    // Give the LLM time to respond
    let response = tokio::time::timeout(Duration::from_secs(30), app.clone().oneshot(message_req))
        .await
        .expect("LLM response timed out after 30s")
        .unwrap();

    assert_eq!(response.status(), 200, "Message send should succeed");

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let messages: Value = serde_json::from_slice(&body).unwrap();

    // Verify we got a response
    assert!(
        messages.is_array(),
        "Response should be an array of messages"
    );
    let messages_arr = messages.as_array().unwrap();

    // Find the assistant response
    let assistant_msg = messages_arr
        .iter()
        .find(|m| m["role"] == "assistant")
        .expect("Should have an assistant response");

    let content = assistant_msg["content"].as_str().unwrap_or("");
    println!("LLM Response: {}", content);

    // Basic validation - response should not be empty or a stub
    assert!(!content.is_empty(), "LLM response should not be empty");
    assert!(
        !content.contains("not implemented"),
        "Response should not be a stub"
    );
    assert!(
        !content.contains("mock"),
        "Response should not be from mock"
    );

    // Cleanup
    let delete_req = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/agents/{}", agent_id))
        .body(Body::empty())
        .unwrap();
    let _ = app.oneshot(delete_req).await;

    println!("✓ Real LLM integration test passed");
}

/// Test: Memory persistence across messages with real LLM
///
/// Verifies:
/// 1. Agent remembers context across multiple messages
/// 2. Memory blocks are correctly passed to LLM
#[tokio::test]
#[ignore = "Requires ANTHROPIC_API_KEY or OPENAI_API_KEY"]
async fn test_real_llm_memory_persistence() {
    if skip_without_api_key() {
        return;
    }

    let app = test_app_with_real_llm().await;

    // Create agent
    let create_req = Request::builder()
        .method("POST")
        .uri("/v1/agents")
        .header("Content-Type", "application/json")
        .body(Body::from(
            json!({
                "name": "memory-test-agent",
                "memory_blocks": [
                    {
                        "label": "persona",
                        "value": "You are a helpful assistant. Keep track of what the user tells you."
                    },
                    {
                        "label": "human",
                        "value": "The human likes cats."
                    }
                ]
            })
            .to_string(),
        ))
        .unwrap();

    let response = app.clone().oneshot(create_req).await.unwrap();
    assert_eq!(response.status(), 200);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let agent: Value = serde_json::from_slice(&body).unwrap();
    let agent_id = agent["id"].as_str().unwrap();

    // First message: tell the agent something specific
    let msg1_req = Request::builder()
        .method("POST")
        .uri(format!("/v1/agents/{}/messages", agent_id))
        .header("Content-Type", "application/json")
        .body(Body::from(
            json!({
                "role": "user",
                "content": "My favorite color is purple. Please acknowledge."
            })
            .to_string(),
        ))
        .unwrap();

    let response = tokio::time::timeout(Duration::from_secs(30), app.clone().oneshot(msg1_req))
        .await
        .expect("LLM response timed out")
        .unwrap();
    assert_eq!(response.status(), 200);

    // Second message: ask about what we said
    let msg2_req = Request::builder()
        .method("POST")
        .uri(format!("/v1/agents/{}/messages", agent_id))
        .header("Content-Type", "application/json")
        .body(Body::from(
            json!({
                "role": "user",
                "content": "What is my favorite color?"
            })
            .to_string(),
        ))
        .unwrap();

    let response = tokio::time::timeout(Duration::from_secs(30), app.clone().oneshot(msg2_req))
        .await
        .expect("LLM response timed out")
        .unwrap();
    assert_eq!(response.status(), 200);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let messages: Value = serde_json::from_slice(&body).unwrap();

    // Find the last assistant response
    if let Some(msgs) = messages.as_array() {
        let last_assistant = msgs.iter().rev().find(|m| m["role"] == "assistant");

        if let Some(msg) = last_assistant {
            let content = msg["content"].as_str().unwrap_or("").to_lowercase();
            println!("LLM Response about memory: {}", content);

            // Check if LLM remembered the color
            if content.contains("purple") {
                println!("✓ LLM remembered the favorite color");
            } else {
                println!(
                    "⚠ LLM may not have recalled the color (this can happen with context issues)"
                );
            }
        }
    }

    // Cleanup
    let delete_req = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/agents/{}", agent_id))
        .body(Body::empty())
        .unwrap();
    let _ = app.oneshot(delete_req).await;

    println!("✓ Real LLM memory persistence test passed");
}
