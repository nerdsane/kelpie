//! Simulated HTTP client for DST tests
//!
//! Provides a fault-injectable HTTP client that wraps HTTP operations with
//! deterministic fault injection for testing resilience under various failure modes.
//!
//! # Fault Types Supported
//! - `NetworkPacketLoss`: Simulates connection failures
//! - `NetworkDelay`: Simulates network latency with deterministic delays
//! - `LlmTimeout`: Simulates LLM API timeouts
//! - `LlmFailure`: Simulates LLM API failures
//! - `LlmRateLimited`: Simulates LLM API rate limiting (429 responses)
//!
//! # Usage
//! ```ignore
//! let http_client = Arc::new(FaultInjectedHttpClient::new(
//!     sim_env.faults.clone(),
//!     sim_env.rng.clone(),
//!     sim_env.io_context.time.clone(),
//!     mock_sse_response(&["Hello", " world"]),
//! ));
//! ```

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{self, Stream};
use kelpie_core::{RngProvider, TimeProvider};
use kelpie_dst::{DeterministicRng, FaultInjector, FaultType};
use kelpie_server::http::{HttpClient, HttpRequest, HttpResponse};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

/// HTTP client with fault injection for DST
///
/// Wraps HTTP operations with deterministic fault injection based on
/// the configured `FaultInjector`. Uses `TimeProvider` for deterministic
/// delays that advance the simulation clock.
#[allow(dead_code)] // Exported for use by other test files
pub struct FaultInjectedHttpClient {
    faults: Arc<FaultInjector>,
    rng: Arc<DeterministicRng>,
    time: Arc<dyn TimeProvider>,
    stream_body: String,
}

#[allow(dead_code)] // Exported for use by other test files
impl FaultInjectedHttpClient {
    /// Create a new fault-injected HTTP client
    ///
    /// # Arguments
    /// * `faults` - Fault injector from simulation environment
    /// * `rng` - Deterministic RNG for delay calculations
    /// * `time` - Time provider for deterministic sleeps
    /// * `stream_body` - Pre-built SSE response body for streaming
    pub fn new(
        faults: Arc<FaultInjector>,
        rng: Arc<DeterministicRng>,
        time: Arc<dyn TimeProvider>,
        stream_body: String,
    ) -> Self {
        Self {
            faults,
            rng,
            time,
            stream_body,
        }
    }

    /// Inject faults before HTTP operations
    ///
    /// Checks the fault injector for applicable faults and either:
    /// - Returns an error (for failure faults like NetworkPacketLoss, LlmTimeout)
    /// - Adds delay (for NetworkDelay)
    /// - Returns rate limit response info (for LlmRateLimited)
    async fn inject_faults(&self) -> Result<Option<RateLimitInfo>, String> {
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
                    return Ok(Some(RateLimitInfo {
                        retry_after_ms: 60_000,
                    }));
                }
                _ => {}
            }
        }

        Ok(None)
    }
}

/// Rate limit information returned when LlmRateLimited fault triggers
#[allow(dead_code)] // Exported for use by other test files
pub struct RateLimitInfo {
    pub retry_after_ms: u64,
}

#[async_trait]
impl HttpClient for FaultInjectedHttpClient {
    async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, String> {
        match self.inject_faults().await? {
            Some(rate_limit) => {
                // Return 429 response
                let mut headers = HashMap::new();
                headers.insert(
                    "Retry-After".to_string(),
                    (rate_limit.retry_after_ms / 1000).to_string(),
                );
                Ok(HttpResponse {
                    status: 429,
                    headers,
                    body: b"Rate limited".to_vec(),
                })
            }
            None => Ok(HttpResponse {
                status: 200,
                headers: HashMap::new(),
                body: Vec::new(),
            }),
        }
    }

    async fn send_streaming(
        &self,
        _request: HttpRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Bytes, String>> + Send>>, String> {
        match self.inject_faults().await? {
            Some(_rate_limit) => {
                // Rate limiting on streaming - return error
                return Err("Rate limited (429)".to_string());
            }
            None => {
                let stream = stream::iter(vec![Ok(Bytes::from(self.stream_body.clone()))]);
                Ok(Box::pin(stream))
            }
        }
    }
}

/// Build mock Anthropic SSE response with specified tokens
///
/// Creates a properly formatted SSE response that the Anthropic API client
/// can parse. Tokens are emitted as content_block_delta events.
///
/// # Arguments
/// * `tokens` - Slice of token strings to include in the response
///
/// # Returns
/// A string containing the full SSE event stream
#[allow(dead_code)] // Exported for use by other test files
pub fn mock_sse_response(tokens: &[&str]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_sse_response_format() {
        let response = mock_sse_response(&["Hello", " world"]);
        assert!(response.contains("event: message_start"));
        assert!(response.contains("\"text\":\"Hello\""));
        assert!(response.contains("\"text\":\" world\""));
        assert!(response.contains("event: message_stop"));
    }

    #[test]
    fn test_mock_sse_response_escapes_quotes() {
        let response = mock_sse_response(&["Say \"hello\""]);
        assert!(response.contains("\\\"hello\\\""));
    }
}
