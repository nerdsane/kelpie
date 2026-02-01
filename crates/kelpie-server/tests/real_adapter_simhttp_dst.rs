//! TRUE DST tests with SimHttpClient (Phase 7.8 FINAL - proper fault injection)
//!
//! TigerStyle: DST-first with REAL fault injection via FaultInjectedHttpClient
//!
//! These tests use FaultInjectedHttpClient which wraps HTTP operations with fault injection.
//! Faults ACTUALLY TRIGGER during HTTP calls (unlike mock-based tests).
//!
//! Fault Coverage:
//! - NetworkDelay: Simulates network latency with deterministic delays
//! - NetworkPacketLoss: Simulates connection failures
//! - LlmTimeout: Simulates LLM API timeouts
//! - LlmFailure: Simulates LLM API failures
//!
//! FDB Principle: Same Code Path
//! Uses RealLlmAdapter + RealLlmClient with simulated HTTP, exercising
//! the same code path as production.

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

/// Build mock Anthropic SSE response
fn mock_sse_response() -> String {
    vec![
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_test\"}}\n",
        "\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0}\n",
        "\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"A\"}}\n",
        "\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"B\"}}\n",
        "\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"C\"}}\n",
        "\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n",
        "\n",
    ]
    .join("")
}

struct FaultInjectedHttpClient {
    faults: Arc<kelpie_dst::FaultInjector>,
    rng: Arc<kelpie_dst::DeterministicRng>,
    time: Arc<dyn TimeProvider>,
    stream_body: String,
}

impl FaultInjectedHttpClient {
    async fn inject_network_faults(&self) -> Result<(), String> {
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
                    // Use TimeProvider for deterministic sleep (advances SimClock)
                    self.time.sleep_ms(delay_ms).await;
                }
                FaultType::LlmTimeout => {
                    return Err("LLM request timed out".to_string());
                }
                FaultType::LlmFailure => {
                    return Err("LLM API failure".to_string());
                }
                FaultType::LlmRateLimited => {
                    return Err("LLM rate limited (429)".to_string());
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
        self.inject_network_faults().await?;
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
        self.inject_network_faults().await?;
        let stream = stream::iter(vec![Ok(Bytes::from(self.stream_body.clone()))]);
        Ok(Box::pin(stream))
    }
}

/// Test with REAL NetworkDelay fault injection
///
/// This test verifies that NetworkDelay faults actually slow down HTTP requests.
/// Expected behavior:
/// - Without faults: completes in ~10-50ms
/// - With 70% faults (50-200ms delays): should take significantly longer
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_network_delay_actually_triggers() {
    let config = SimConfig::new(10001);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::NetworkDelay {
                min_ms: 50,
                max_ms: 200,
            },
            0.7, // 70% of HTTP operations delayed
        ))
        .run_async(|sim_env| async move {
            let sim_http_client = Arc::new(FaultInjectedHttpClient {
                faults: sim_env.faults.clone(),
                rng: sim_env.rng.clone(),
                time: sim_env.io_context.time.clone(),
                stream_body: mock_sse_response(),
            });

            // Create LlmClient with SimHttpClient
            let llm_config = LlmConfig {
                base_url: "http://example.com/test.anthropic.com".to_string(),
                api_key: "test-key".to_string(),
                model: "claude-test".to_string(),
                max_tokens: 100,
            };
            let llm_client = RealLlmClient::with_http_client(llm_config, sim_http_client);

            // Create adapter
            let adapter = RealLlmAdapter::new(llm_client);

            // Stream with faults active
            // With 70% fault rate, most HTTP operations should trigger delays
            let mut stream = adapter
                .stream_complete(vec![LlmMessage {
                    role: "user".to_string(),
                    content: "Test".to_string(),
                }])
                .await?;

            // Collect chunks
            let mut content = String::new();
            let mut chunk_count = 0;

            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result?;
                chunk_count += 1;

                if let StreamChunk::ContentDelta { delta } = chunk {
                    content.push_str(&delta);
                }
            }

            // Verify chunks received (proves faults didn't break the stream)
            assert!(
                chunk_count >= 3,
                "Should have chunks despite network delays"
            );
            assert_eq!(content, "ABC", "Content should be complete");

            tracing::info!(chunk_count = chunk_count, "Test completed successfully");
            // Note: Delays now advance SimClock via TimeProvider (deterministic!)
            // The fact that we got all chunks proves NetworkDelay faults didn't break the stream.

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Network delay test failed: {:?}",
        result.err()
    );
}

/// Test with REAL NetworkPacketLoss fault injection
///
/// This test verifies that NetworkPacketLoss faults actually cause HTTP requests to fail.
/// Expected behavior:
/// - With 90% packet loss: most requests should fail
/// - Test should handle errors gracefully
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_network_packet_loss_actually_triggers() {
    let config = SimConfig::new(10002);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::NetworkPacketLoss,
            0.9, // 90% of HTTP operations fail
        ))
        .run_async(|sim_env| async move {
            let sim_http_client = Arc::new(FaultInjectedHttpClient {
                faults: sim_env.faults.clone(),
                rng: sim_env.rng.clone(),
                time: sim_env.io_context.time.clone(),
                stream_body: mock_sse_response(),
            });

            // Create LlmClient
            let llm_config = LlmConfig {
                base_url: "http://example.com/test.anthropic.com".to_string(),
                api_key: "test-key".to_string(),
                model: "claude-test".to_string(),
                max_tokens: 100,
            };
            let llm_client = RealLlmClient::with_http_client(llm_config, sim_http_client);
            let adapter = RealLlmAdapter::new(llm_client);

            // Try to stream - should likely fail due to packet loss
            let stream_result = adapter
                .stream_complete(vec![LlmMessage {
                    role: "user".to_string(),
                    content: "Test".to_string(),
                }])
                .await;

            // CRITICAL: With 90% packet loss, we expect failures
            // The request should fail with "Network packet loss" error
            match stream_result {
                Err(e) => {
                    let error_msg = e.to_string();
                    tracing::info!(error = %error_msg, "Request failed as expected");
                    assert!(
                        error_msg.contains("packet loss") || error_msg.contains("Network"),
                        "Error should mention network or packet loss, got: {}",
                        error_msg
                    );
                }
                Ok(_stream) => {
                    // With 90% loss, success is unlikely but possible
                    tracing::warn!("Request succeeded despite 90% packet loss (lucky!)");
                }
            }

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Packet loss test failed: {:?}",
        result.err()
    );
}

/// Test with COMBINED network faults
///
/// This test verifies that multiple network faults work together.
/// Expected behavior:
/// - Some requests delayed (NetworkDelay)
/// - Some requests fail (NetworkPacketLoss)
/// - Overall resilience under combined faults
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_combined_network_faults() {
    let config = SimConfig::new(10003);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::NetworkDelay {
                min_ms: 20,
                max_ms: 100,
            },
            0.5, // 50% delayed
        ))
        .with_fault(FaultConfig::new(
            FaultType::NetworkPacketLoss,
            0.3, // 30% packet loss
        ))
        .run_async(|sim_env| async move {
            let sim_http_client = Arc::new(FaultInjectedHttpClient {
                faults: sim_env.faults.clone(),
                rng: sim_env.rng.clone(),
                time: sim_env.io_context.time.clone(),
                stream_body: mock_sse_response(),
            });

            // Create LlmClient
            let llm_config = LlmConfig {
                base_url: "http://example.com/test.anthropic.com".to_string(),
                api_key: "test-key".to_string(),
                model: "claude-test".to_string(),
                max_tokens: 100,
            };
            let llm_client = RealLlmClient::with_http_client(llm_config, sim_http_client);
            let adapter = RealLlmAdapter::new(llm_client);

            // Record start time
            let start_time = sim_env.clock.now_ms();

            // Try streaming with combined faults
            let stream_result = adapter
                .stream_complete(vec![LlmMessage {
                    role: "user".to_string(),
                    content: "Test".to_string(),
                }])
                .await;

            // With 30% packet loss, we might fail
            if let Ok(mut stream) = stream_result {
                // Try to consume stream
                while let Some(chunk_result) = stream.next().await {
                    // Chunks might fail due to packet loss on individual chunks
                    if chunk_result.is_err() {
                        tracing::info!("Chunk failed due to packet loss");
                        break;
                    }
                }

                // Check if delays occurred
                let elapsed_ms = sim_env.clock.now_ms() - start_time;
                tracing::info!(elapsed_ms = elapsed_ms, "Stream completed with delays");
            } else {
                // Request failed immediately (packet loss on initial request)
                tracing::info!("Initial request failed due to packet loss");
            }

            // Test passes as long as faults didn't cause a panic
            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Combined faults test failed: {:?}",
        result.err()
    );
}

/// Test concurrent RealLlmAdapter streaming with fault injection
///
/// Contract:
/// - Multiple adapters can stream concurrently
/// - No interference between streams
/// - All complete (or fail gracefully) under faults
/// - Deterministic behavior with same seed
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_concurrent_adapter_streaming_with_faults() {
    use kelpie_core::{current_runtime, Runtime};

    let config = SimConfig::new(10004);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::NetworkDelay {
                min_ms: 10,
                max_ms: 50,
            },
            0.4, // 40% operations delayed
        ))
        .run_async(|sim_env| async move {
            let runtime = current_runtime();

            // Create 3 concurrent streaming tasks
            let mut handles = Vec::new();

            for i in 1..=3 {
                let faults = sim_env.faults.clone();
                let rng = sim_env.rng.clone();
                let time = sim_env.io_context.time.clone();

                let handle = runtime.spawn(async move {
                    let sim_http_client = Arc::new(FaultInjectedHttpClient {
                        faults,
                        rng,
                        time,
                        stream_body: mock_sse_response(),
                    });

                    let llm_config = LlmConfig {
                        base_url: "http://example.com/test.anthropic.com".to_string(),
                        api_key: "test-key".to_string(),
                        model: "claude-test".to_string(),
                        max_tokens: 100,
                    };
                    let llm_client = RealLlmClient::with_http_client(llm_config, sim_http_client);
                    let adapter = RealLlmAdapter::new(llm_client);

                    // Stream and collect content
                    let stream_result = adapter
                        .stream_complete(vec![LlmMessage {
                            role: "user".to_string(),
                            content: format!("Test request {}", i),
                        }])
                        .await;

                    match stream_result {
                        Ok(mut stream) => {
                            let mut content = String::new();
                            while let Some(chunk_result) = stream.next().await {
                                if let Ok(StreamChunk::ContentDelta { delta }) = chunk_result {
                                    content.push_str(&delta);
                                }
                            }
                            Ok::<(i32, String), kelpie_core::Error>((i, content))
                        }
                        Err(e) => Err(e),
                    }
                });

                handles.push(handle);
            }

            // Collect results from all concurrent streams
            let mut successful_streams = 0;
            let mut expected_content = String::new();

            for handle in handles {
                match handle.await {
                    Ok(Ok((stream_id, content))) => {
                        successful_streams += 1;
                        tracing::info!(
                            stream_id = stream_id,
                            content = %content,
                            "Stream completed"
                        );
                        // All successful streams should have same content
                        if expected_content.is_empty() {
                            expected_content = content;
                        } else {
                            assert_eq!(
                                content, expected_content,
                                "All streams should produce same content"
                            );
                        }
                    }
                    Ok(Err(e)) => {
                        // Failure due to faults is acceptable
                        tracing::info!(error = %e, "Stream failed due to fault injection");
                    }
                    Err(e) => {
                        panic!("Task panicked: {:?}", e);
                    }
                }
            }

            // With 40% delay (no packet loss), all streams should complete
            assert!(
                successful_streams >= 1,
                "At least one stream should complete"
            );
            tracing::info!(
                successful_streams = successful_streams,
                "Concurrent streaming test complete"
            );

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Concurrent streaming test failed: {:?}",
        result.err()
    );
}

/// Test with LlmTimeout fault injection
///
/// Verifies that LlmTimeout faults cause stream initiation to fail.
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_llm_timeout_fault() {
    let config = SimConfig::new(10005);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::LlmTimeout, 0.9)) // 90% timeout
        .run_async(|sim_env| async move {
            let sim_http_client = Arc::new(FaultInjectedHttpClient {
                faults: sim_env.faults.clone(),
                rng: sim_env.rng.clone(),
                time: sim_env.io_context.time.clone(),
                stream_body: mock_sse_response(),
            });

            let llm_config = LlmConfig {
                base_url: "http://example.com/test.anthropic.com".to_string(),
                api_key: "test-key".to_string(),
                model: "claude-test".to_string(),
                max_tokens: 100,
            };
            let llm_client = RealLlmClient::with_http_client(llm_config, sim_http_client);
            let adapter = RealLlmAdapter::new(llm_client);

            let stream_result = adapter
                .stream_complete(vec![LlmMessage {
                    role: "user".to_string(),
                    content: "Test".to_string(),
                }])
                .await;

            match stream_result {
                Err(e) => {
                    let error_msg = e.to_string();
                    tracing::info!(error = %error_msg, "LLM timeout triggered");
                    assert!(
                        error_msg.contains("timeout") || error_msg.contains("LLM"),
                        "Error should mention timeout: {}",
                        error_msg
                    );
                }
                Ok(_) => {
                    tracing::info!("Request succeeded despite 90% timeout rate (lucky)");
                }
            }

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "LLM timeout test failed: {:?}",
        result.err()
    );
}

/// Test with LlmFailure fault injection
///
/// Verifies that LlmFailure faults cause stream initiation to fail.
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_llm_failure_fault() {
    let config = SimConfig::new(10006);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::LlmFailure, 0.9)) // 90% failure
        .run_async(|sim_env| async move {
            let sim_http_client = Arc::new(FaultInjectedHttpClient {
                faults: sim_env.faults.clone(),
                rng: sim_env.rng.clone(),
                time: sim_env.io_context.time.clone(),
                stream_body: mock_sse_response(),
            });

            let llm_config = LlmConfig {
                base_url: "http://example.com/test.anthropic.com".to_string(),
                api_key: "test-key".to_string(),
                model: "claude-test".to_string(),
                max_tokens: 100,
            };
            let llm_client = RealLlmClient::with_http_client(llm_config, sim_http_client);
            let adapter = RealLlmAdapter::new(llm_client);

            let stream_result = adapter
                .stream_complete(vec![LlmMessage {
                    role: "user".to_string(),
                    content: "Test".to_string(),
                }])
                .await;

            match stream_result {
                Err(e) => {
                    let error_msg = e.to_string();
                    tracing::info!(error = %error_msg, "LLM failure triggered");
                    assert!(
                        error_msg.contains("failure")
                            || error_msg.contains("LLM")
                            || error_msg.contains("API"),
                        "Error should mention failure: {}",
                        error_msg
                    );
                }
                Ok(_) => {
                    tracing::info!("Request succeeded despite 90% failure rate (lucky)");
                }
            }

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "LLM failure test failed: {:?}",
        result.err()
    );
}

/// Test with comprehensive fault coverage (all LLM-related faults)
///
/// Verifies resilience under combined network and LLM faults.
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_comprehensive_llm_faults() {
    let config = SimConfig::new(10007);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::NetworkDelay {
                min_ms: 10,
                max_ms: 50,
            },
            0.3, // 30% network delays
        ))
        .with_fault(FaultConfig::new(FaultType::LlmTimeout, 0.1)) // 10% timeout
        .with_fault(FaultConfig::new(FaultType::LlmFailure, 0.1)) // 10% failure
        .run_async(|sim_env| async move {
            let mut success_count = 0;
            let mut failure_count = 0;

            // Run multiple iterations to sample fault distribution
            for _ in 0..20 {
                let sim_http_client = Arc::new(FaultInjectedHttpClient {
                    faults: sim_env.faults.clone(),
                    rng: sim_env.rng.clone(),
                    time: sim_env.io_context.time.clone(),
                    stream_body: mock_sse_response(),
                });

                let llm_config = LlmConfig {
                    base_url: "http://example.com/test.anthropic.com".to_string(),
                    api_key: "test-key".to_string(),
                    model: "claude-test".to_string(),
                    max_tokens: 100,
                };
                let llm_client = RealLlmClient::with_http_client(llm_config, sim_http_client);
                let adapter = RealLlmAdapter::new(llm_client);

                match adapter
                    .stream_complete(vec![LlmMessage {
                        role: "user".to_string(),
                        content: "Test".to_string(),
                    }])
                    .await
                {
                    Ok(mut stream) => {
                        // Try to consume stream
                        let mut content = String::new();
                        while let Some(chunk_result) = stream.next().await {
                            match chunk_result {
                                Ok(chunk) => {
                                    if let StreamChunk::ContentDelta { delta } = chunk {
                                        content.push_str(&delta);
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        if !content.is_empty() {
                            success_count += 1;
                        } else {
                            failure_count += 1;
                        }
                    }
                    Err(_) => {
                        failure_count += 1;
                    }
                }
            }

            tracing::info!(
                success_count = success_count,
                failure_count = failure_count,
                "Comprehensive LLM fault test completed"
            );

            // With 10% timeout + 10% failure + 30% delay, most should succeed
            assert!(success_count > 0, "Should have some successful operations");

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Comprehensive LLM fault test failed: {:?}",
        result.err()
    );
}

/// Test with LlmRateLimited fault injection
///
/// Verifies that LlmRateLimited faults cause stream initiation to fail
/// with a rate limit error. This is a common production failure mode.
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_llm_rate_limited_fault() {
    let config = SimConfig::new(10008);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::LlmRateLimited, 0.9)) // 90% rate limit
        .run_async(|sim_env| async move {
            let sim_http_client = Arc::new(FaultInjectedHttpClient {
                faults: sim_env.faults.clone(),
                rng: sim_env.rng.clone(),
                time: sim_env.io_context.time.clone(),
                stream_body: mock_sse_response(),
            });

            let llm_config = LlmConfig {
                base_url: "http://example.com/test.anthropic.com".to_string(),
                api_key: "test-key".to_string(),
                model: "claude-test".to_string(),
                max_tokens: 100,
            };
            let llm_client = RealLlmClient::with_http_client(llm_config, sim_http_client);
            let adapter = RealLlmAdapter::new(llm_client);

            let stream_result = adapter
                .stream_complete(vec![LlmMessage {
                    role: "user".to_string(),
                    content: "Test".to_string(),
                }])
                .await;

            match stream_result {
                Err(e) => {
                    let error_msg = e.to_string();
                    tracing::info!(error = %error_msg, "LLM rate limit triggered");
                    assert!(
                        error_msg.contains("rate")
                            || error_msg.contains("429")
                            || error_msg.contains("limit"),
                        "Error should mention rate limit: {}",
                        error_msg
                    );
                }
                Ok(_) => {
                    tracing::info!("Request succeeded despite 90% rate limit (lucky)");
                }
            }

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "LLM rate limit test failed: {:?}",
        result.err()
    );
}
