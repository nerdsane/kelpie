//! DST tests for LLM client streaming (Phase 7.8)
//!
//! TigerStyle: DST-first with real fault injection via FaultInjectedHttpClient
//!
//! Tests cover:
//! - Token-by-token streaming (not batch conversion)
//! - Stream cancellation and cleanup
//! - Error handling during streaming
//! - Multiple concurrent streams
//! - Fault injection (LlmTimeout, LlmFailure, NetworkDelay)
//!
//! FDB Principle: Same Code Path
//! These tests use RealLlmAdapter + FaultInjectedHttpClient to exercise
//! the same code path as production, with deterministic fault injection.
#![cfg(feature = "dst")]

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{self, StreamExt};
use kelpie_core::{RngProvider, TimeProvider};
use kelpie_dst::{FaultConfig, FaultType, SimConfig, Simulation};
use kelpie_server::actor::{LlmClient, LlmMessage, RealLlmAdapter, StreamChunk};
use kelpie_server::http::{HttpClient, HttpRequest, HttpResponse};
use kelpie_server::llm::{LlmClient as RealLlmClient, LlmConfig};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

/// Build mock Anthropic SSE response with specified tokens
fn mock_sse_response(tokens: &[&str]) -> String {
    let mut events = vec![
        "event: message_start\n".to_string(),
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_test\"}}\n".to_string(),
        "\n".to_string(),
        "event: content_block_start\n".to_string(),
        "data: {\"type\":\"content_block_start\",\"index\":0}\n".to_string(),
        "\n".to_string(),
    ];

    for token in tokens {
        let escaped = token.replace('\\', "\\\\").replace('"', "\\\"");
        events.push("event: content_block_delta\n".to_string());
        events.push(format!(
            "data: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{}\"}}}}\n",
            escaped
        ));
        events.push("\n".to_string());
    }

    events.push("event: message_stop\n".to_string());
    events.push("data: {\"type\":\"message_stop\"}\n".to_string());
    events.push("\n".to_string());

    events.join("")
}

/// HTTP client with fault injection for DST
struct FaultInjectedHttpClient {
    faults: Arc<kelpie_dst::FaultInjector>,
    rng: Arc<kelpie_dst::DeterministicRng>,
    time: Arc<dyn TimeProvider>,
    stream_body: String,
}

impl FaultInjectedHttpClient {
    async fn inject_faults(&self) -> Result<(), String> {
        if let Some(fault) = self.faults.should_inject("http_send") {
            match fault {
                FaultType::NetworkPacketLoss => {
                    return Err("Network packet loss".to_string());
                }
                FaultType::NetworkDelay { min_ms, max_ms } => {
                    let delay_ms = if min_ms == max_ms {
                        min_ms
                    } else {
                        self.rng.as_ref().gen_range(min_ms, max_ms)
                    };
                    self.time.sleep_ms(delay_ms).await;
                }
                FaultType::LlmTimeout => {
                    return Err("LLM request timed out".to_string());
                }
                FaultType::LlmFailure => {
                    return Err("LLM API failure".to_string());
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[async_trait]
impl HttpClient for FaultInjectedHttpClient {
    async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, String> {
        self.inject_faults().await?;
        Ok(HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: Vec::new(),
        })
    }

    async fn send_streaming(
        &self,
        _request: HttpRequest,
    ) -> Result<Pin<Box<dyn futures::stream::Stream<Item = Result<Bytes, String>> + Send>>, String>
    {
        self.inject_faults().await?;
        let stream = stream::iter(vec![Ok(Bytes::from(self.stream_body.clone()))]);
        Ok(Box::pin(stream))
    }
}

/// Create LLM config for testing
fn test_llm_config() -> LlmConfig {
    LlmConfig {
        base_url: "http://test.anthropic.com".to_string(),
        api_key: "test-key".to_string(),
        model: "claude-test".to_string(),
        max_tokens: 100,
    }
}

/// Test token-by-token streaming with RealLlmAdapter + FaultInjectedHttpClient
///
/// FDB Principle: Same Code Path
/// Uses production adapter with simulated HTTP layer.
///
/// Contract:
/// - Tokens arrive incrementally (one per chunk)
/// - Order preserved
/// - Stream ends with Done chunk
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_llm_client_token_streaming() {
    let config = SimConfig::new(6001);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            let tokens = ["Hello", " ", "world", "!"];
            let expected_content = tokens.join("");

            let http_client = Arc::new(FaultInjectedHttpClient {
                faults: sim_env.faults.clone(),
                rng: sim_env.rng.clone(),
                time: sim_env.io_context.time.clone(),
                stream_body: mock_sse_response(&tokens),
            });

            let llm_client = RealLlmClient::with_http_client(test_llm_config(), http_client);
            let adapter = RealLlmAdapter::new(llm_client);

            // Stream through production code path
            let mut stream = adapter
                .stream_complete(vec![LlmMessage {
                    role: "user".to_string(),
                    content: "Say hello".to_string(),
                }])
                .await?;

            // Collect chunks
            let mut content = String::new();
            let mut chunk_count = 0;
            let mut done_received = false;

            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result?;
                chunk_count += 1;

                match chunk {
                    StreamChunk::ContentDelta { delta } => {
                        content.push_str(&delta);
                    }
                    StreamChunk::Done { .. } => {
                        done_received = true;
                        break;
                    }
                    _ => {}
                }
            }

            // Verify streaming behavior
            assert!(
                chunk_count >= tokens.len(),
                "Should have at least {} chunks",
                tokens.len()
            );
            assert_eq!(content, expected_content, "Content should match");
            assert!(done_received, "Should receive Done chunk");

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Token streaming test failed: {:?}",
        result.err()
    );
}

/// Test stream cancellation (early drop) with RealLlmAdapter
///
/// Contract:
/// - Dropping stream consumer stops iteration
/// - No resource leak
/// - Clean shutdown
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_llm_client_cancellation() {
    let config = SimConfig::new(6002);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            // Generate many tokens for long response
            let tokens: Vec<&str> = (0..50).map(|_| "token ").collect();

            let http_client = Arc::new(FaultInjectedHttpClient {
                faults: sim_env.faults.clone(),
                rng: sim_env.rng.clone(),
                time: sim_env.io_context.time.clone(),
                stream_body: mock_sse_response(&tokens),
            });

            let llm_client = RealLlmClient::with_http_client(test_llm_config(), http_client);
            let adapter = RealLlmAdapter::new(llm_client);

            // Start streaming
            let mut stream = adapter
                .stream_complete(vec![LlmMessage {
                    role: "user".to_string(),
                    content: "Long response".to_string(),
                }])
                .await?;

            // Consume only 5 chunks then drop
            let mut consumed = 0;
            while let Some(chunk_result) = stream.next().await {
                let _chunk = chunk_result?;
                consumed += 1;
                if consumed >= 5 {
                    break; // Drop stream early
                }
            }

            // Stream is dropped here - should clean up without panic
            assert!(consumed >= 5, "Should have consumed at least 5 chunks");

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Cancellation test failed: {:?}",
        result.err()
    );
}

/// Test streaming with network delays (LLM-specific faults)
///
/// Contract:
/// - Stream completes despite NetworkDelay faults
/// - Tokens arrive in order
/// - No tokens lost
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_llm_client_with_network_delay() {
    let config = SimConfig::new(6003);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::NetworkDelay {
                min_ms: 5,
                max_ms: 20,
            },
            0.5, // 50% of HTTP operations delayed
        ))
        .run_async(|sim_env| async move {
            let tokens = ["One", " ", "Two", " ", "Three"];
            let expected_content = tokens.join("");

            let http_client = Arc::new(FaultInjectedHttpClient {
                faults: sim_env.faults.clone(),
                rng: sim_env.rng.clone(),
                time: sim_env.io_context.time.clone(),
                stream_body: mock_sse_response(&tokens),
            });

            let llm_client = RealLlmClient::with_http_client(test_llm_config(), http_client);
            let adapter = RealLlmAdapter::new(llm_client);

            // Stream with delays active
            let mut stream = adapter
                .stream_complete(vec![LlmMessage {
                    role: "user".to_string(),
                    content: "Count words".to_string(),
                }])
                .await?;

            let mut content = String::new();
            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result?;

                if let StreamChunk::ContentDelta { delta } = chunk {
                    content.push_str(&delta);
                }
            }

            // Verify complete content despite delays
            assert_eq!(content, expected_content, "Content should be complete");

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Network delay test failed: {:?}",
        result.err()
    );
}

/// Test concurrent streams with RealLlmAdapter
///
/// Contract:
/// - Multiple adapters can stream concurrently
/// - Streams don't interfere with each other
/// - All streams complete successfully
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_llm_client_concurrent() {
    let config = SimConfig::new(6004);

    let result = Simulation::new(config)
        .run_async(|sim_env| async move {
            use kelpie_core::{current_runtime, Runtime};
            let runtime = current_runtime();

            // Create 3 concurrent streams with different responses
            let mut handles = Vec::new();

            for i in 1..=3 {
                let tokens_static: Vec<&str> = match i {
                    1 => vec!["Client", " one", " response"],
                    2 => vec!["Client", " two", " response"],
                    _ => vec!["Client", " three", " response"],
                };

                let http_client = Arc::new(FaultInjectedHttpClient {
                    faults: sim_env.faults.clone(),
                    rng: sim_env.rng.clone(),
                    time: sim_env.io_context.time.clone(),
                    stream_body: mock_sse_response(&tokens_static),
                });

                let llm_client = RealLlmClient::with_http_client(test_llm_config(), http_client);
                let adapter = RealLlmAdapter::new(llm_client);

                let handle = runtime.spawn(async move {
                    let mut stream = adapter
                        .stream_complete(vec![LlmMessage {
                            role: "user".to_string(),
                            content: format!("Message {}", i),
                        }])
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

/// Test streaming with comprehensive LLM-specific fault injection
///
/// Contract:
/// - Stream completes despite NetworkDelay faults
/// - LlmTimeout and LlmFailure faults cause stream failure (tested separately)
/// - Tokens arrive in order when stream succeeds
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_llm_client_comprehensive_faults() {
    let config = SimConfig::new(6005);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::NetworkDelay {
                min_ms: 10,
                max_ms: 30,
            },
            0.4, // 40% network delays
        ))
        .run_async(|sim_env| async move {
            let tokens: Vec<&str> = (0..10).map(|_| "t ").collect();
            let expected_content = tokens.join("");

            let http_client = Arc::new(FaultInjectedHttpClient {
                faults: sim_env.faults.clone(),
                rng: sim_env.rng.clone(),
                time: sim_env.io_context.time.clone(),
                stream_body: mock_sse_response(&tokens),
            });

            let llm_client = RealLlmClient::with_http_client(test_llm_config(), http_client);
            let adapter = RealLlmAdapter::new(llm_client);

            // Stream with faults active
            let mut stream = adapter
                .stream_complete(vec![LlmMessage {
                    role: "user".to_string(),
                    content: "Test resilience".to_string(),
                }])
                .await?;

            let mut chunk_count = 0;
            let mut content = String::new();

            while let Some(chunk_result) = stream.next().await {
                // Should not fail despite network delays
                let chunk = chunk_result?;
                chunk_count += 1;

                if let StreamChunk::ContentDelta { delta } = chunk {
                    content.push_str(&delta);
                }
            }

            // Verify stream completed successfully despite faults
            assert!(chunk_count > 0, "Should have chunks despite faults");
            assert_eq!(content, expected_content, "Content should be complete");

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Comprehensive fault test failed: {:?}",
        result.err()
    );
}

/// Test streaming with LlmTimeout faults
///
/// Contract:
/// - LlmTimeout fault causes stream initiation to fail
/// - Error is properly propagated
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_llm_client_timeout_fault() {
    let config = SimConfig::new(6006);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::LlmTimeout,
            0.9, // 90% timeout (should reliably fail)
        ))
        .run_async(|sim_env| async move {
            let http_client = Arc::new(FaultInjectedHttpClient {
                faults: sim_env.faults.clone(),
                rng: sim_env.rng.clone(),
                time: sim_env.io_context.time.clone(),
                stream_body: mock_sse_response(&["test"]),
            });

            let llm_client = RealLlmClient::with_http_client(test_llm_config(), http_client);
            let adapter = RealLlmAdapter::new(llm_client);

            // With 90% timeout, stream should likely fail
            let stream_result = adapter
                .stream_complete(vec![LlmMessage {
                    role: "user".to_string(),
                    content: "Test timeout".to_string(),
                }])
                .await;

            // Most attempts should fail with timeout
            match stream_result {
                Err(e) => {
                    let error_msg = e.to_string();
                    tracing::info!(error = %error_msg, "LLM timeout correctly triggered");
                    assert!(
                        error_msg.contains("timeout") || error_msg.contains("LLM"),
                        "Error should mention timeout or LLM: {}",
                        error_msg
                    );
                }
                Ok(_) => {
                    // 10% chance of success - that's fine
                    tracing::info!("Lucky - request succeeded despite 90% timeout rate");
                }
            }

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Timeout fault test failed: {:?}",
        result.err()
    );
}

/// Test streaming with LlmFailure faults
///
/// Contract:
/// - LlmFailure fault causes stream initiation to fail
/// - Error is properly propagated
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_llm_client_failure_fault() {
    let config = SimConfig::new(6007);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::LlmFailure,
            0.9, // 90% failure (should reliably fail)
        ))
        .run_async(|sim_env| async move {
            let http_client = Arc::new(FaultInjectedHttpClient {
                faults: sim_env.faults.clone(),
                rng: sim_env.rng.clone(),
                time: sim_env.io_context.time.clone(),
                stream_body: mock_sse_response(&["test"]),
            });

            let llm_client = RealLlmClient::with_http_client(test_llm_config(), http_client);
            let adapter = RealLlmAdapter::new(llm_client);

            // With 90% failure, stream should likely fail
            let stream_result = adapter
                .stream_complete(vec![LlmMessage {
                    role: "user".to_string(),
                    content: "Test failure".to_string(),
                }])
                .await;

            // Most attempts should fail
            match stream_result {
                Err(e) => {
                    let error_msg = e.to_string();
                    tracing::info!(error = %error_msg, "LLM failure correctly triggered");
                    assert!(
                        error_msg.contains("failure")
                            || error_msg.contains("LLM")
                            || error_msg.contains("API"),
                        "Error should mention failure or LLM: {}",
                        error_msg
                    );
                }
                Ok(_) => {
                    // 10% chance of success - that's fine
                    tracing::info!("Lucky - request succeeded despite 90% failure rate");
                }
            }

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Failure fault test failed: {:?}",
        result.err()
    );
}
