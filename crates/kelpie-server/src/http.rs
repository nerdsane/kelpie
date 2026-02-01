//! HTTP Client Abstraction for DST
//!
//! TigerStyle: All HTTP goes through abstraction traits for deterministic simulation.
//!
//! This module provides:
//! - HttpClient trait: Abstraction over HTTP requests
//! - ReqwestHttpClient: Production implementation using reqwest
//! - SimHttpClient: DST implementation using SimNetwork (to be implemented)
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │              LlmClient (SAME CODE)                  │
//! └──────────────────────┬──────────────────────────────┘
//!                        │
//! ┌──────────────────────▼──────────────────────────────┐
//! │              HttpClient Trait                       │
//! └──────────────────────┬──────────────────────────────┘
//!                        │
//!           ┌────────────┴────────────┐
//!           │                         │
//!     ┌─────▼─────┐            ┌─────▼─────┐
//!     │ Production│            │    DST    │
//!     │  reqwest  │            │SimHttpClient│
//!     │           │            │(SimNetwork)│
//!     └───────────┘            └───────────┘
//! ```

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::Stream;
#[cfg(feature = "dst")]
use futures::StreamExt;
#[cfg(feature = "dst")]
use kelpie_core::RngProvider;
use std::collections::HashMap;
use std::pin::Pin;

/// HTTP method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

impl HttpMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
        }
    }
}

/// HTTP request
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Vec<u8>>,
}

impl HttpRequest {
    pub fn new(method: HttpMethod, url: String) -> Self {
        Self {
            method,
            url,
            headers: HashMap::new(),
            body: None,
        }
    }

    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    pub fn json<T: serde::Serialize>(mut self, body: &T) -> Result<Self, String> {
        let json =
            serde_json::to_vec(body).map_err(|e| format!("JSON serialization failed: {}", e))?;
        self.body = Some(json);
        self.headers
            .insert("Content-Type".to_string(), "application/json".to_string());
        Ok(self)
    }

    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }
}

/// HTTP response
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn text(&self) -> Result<String, String> {
        String::from_utf8(self.body.clone()).map_err(|e| format!("Invalid UTF-8: {}", e))
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, String> {
        serde_json::from_slice(&self.body)
            .map_err(|e| format!("JSON deserialization failed: {}", e))
    }
}

/// HTTP client trait for DST abstraction
///
/// All HTTP operations MUST go through this trait.
/// Never use reqwest directly in business logic.
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// Send an HTTP request and get response
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, String>;

    /// Send an HTTP request and get streaming response
    ///
    /// Returns a stream of byte chunks as they arrive.
    /// Used for Server-Sent Events (SSE) streaming.
    async fn send_streaming(
        &self,
        request: HttpRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Bytes, String>> + Send>>, String>;
}

/// Production HTTP client using reqwest
#[derive(Debug, Clone)]
pub struct ReqwestHttpClient {
    client: reqwest::Client,
}

impl ReqwestHttpClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for ReqwestHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, String> {
        // Build reqwest request
        let method = match request.method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Put => reqwest::Method::PUT,
            HttpMethod::Delete => reqwest::Method::DELETE,
        };

        let mut req_builder = self.client.request(method, &request.url);

        // Add headers
        for (key, value) in request.headers {
            req_builder = req_builder.header(key, value);
        }

        // Add body
        if let Some(body) = request.body {
            req_builder = req_builder.body(body);
        }

        // Send request
        let response = req_builder
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        // Convert response
        let status = response.status().as_u16();
        let headers: HashMap<String, String> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let body = response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read response body: {}", e))?
            .to_vec();

        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }

    async fn send_streaming(
        &self,
        request: HttpRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Bytes, String>> + Send>>, String> {
        // Build reqwest request
        let method = match request.method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Put => reqwest::Method::PUT,
            HttpMethod::Delete => reqwest::Method::DELETE,
        };

        let mut req_builder = self.client.request(method, &request.url);

        // Add headers
        for (key, value) in request.headers {
            req_builder = req_builder.header(key, value);
        }

        // Add body
        if let Some(body) = request.body {
            req_builder = req_builder.body(body);
        }

        // Send request
        let response = req_builder
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        // Check status
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("API error {}: {}", status, body));
        }

        // Convert to byte stream
        let byte_stream = response.bytes_stream();
        let stream = Box::pin(futures::stream::StreamExt::map(byte_stream, |result| {
            result.map_err(|e| format!("Stream error: {}", e))
        }));

        Ok(stream)
    }
}

/// Simulated HTTP client for DST (Phase 7.8 REDO - proper DST)
///
/// This client wraps HTTP operations with fault injection:
/// - NetworkDelay: Adds delays before/after requests (using tokio::time::sleep)
/// - NetworkPacketLoss: Randomly fails requests
/// - NetworkMessageReorder: Not applicable to HTTP (request-response)
///
/// Uses real HTTP (mockito) underneath but adds simulated faults.
///
/// Note: Uses tokio::time::sleep for delays instead of SimClock.sleep_ms() because
/// SimClock requires manual advancement which doesn't happen in async HTTP context.
#[cfg(feature = "dst")]
pub struct SimHttpClient {
    /// Underlying HTTP client (usually mockito-backed reqwest)
    inner: ReqwestHttpClient,
    /// Fault injector
    fault_injector: std::sync::Arc<kelpie_dst::FaultInjector>,
    /// RNG for fault decisions
    rng: std::sync::Arc<kelpie_dst::DeterministicRng>,
}

#[cfg(feature = "dst")]
impl SimHttpClient {
    pub fn new(
        fault_injector: std::sync::Arc<kelpie_dst::FaultInjector>,
        rng: std::sync::Arc<kelpie_dst::DeterministicRng>,
    ) -> Self {
        Self {
            inner: ReqwestHttpClient::new(),
            fault_injector,
            rng,
        }
    }

    /// Inject network faults before HTTP operation
    async fn inject_network_faults(&self) -> Result<(), String> {
        // Check for packet loss
        if let Some(fault) = self.fault_injector.should_inject("http_send") {
            match fault {
                kelpie_dst::FaultType::NetworkPacketLoss => {
                    tracing::debug!("HTTP request dropped: packet loss fault");
                    return Err("Network packet loss".to_string());
                }
                kelpie_dst::FaultType::LlmTimeout => {
                    tracing::debug!("HTTP request failed: simulated LLM timeout");
                    return Err("LLM request timed out".to_string());
                }
                kelpie_dst::FaultType::LlmRateLimited => {
                    tracing::debug!("HTTP request failed: simulated LLM rate limit");
                    return Err("LLM rate limited".to_string());
                }
                kelpie_dst::FaultType::NetworkDelay { min_ms, max_ms } => {
                    // Calculate delay
                    let delay_ms = if min_ms == max_ms {
                        min_ms
                    } else {
                        self.rng.as_ref().gen_range(min_ms, max_ms)
                    };
                    tracing::debug!(delay_ms = delay_ms, "HTTP request delayed");

                    // Use tokio::time::sleep instead of SimClock.sleep_ms()
                    // SimClock.sleep_ms() waits for manual clock advancement which doesn't
                    // happen in async HTTP context, causing deadlock.
                    #[allow(clippy::disallowed_methods)] // Intentional - see comment above
                    tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                }
                _ => {}
            }
        }

        Ok(())
    }
}

#[cfg(feature = "dst")]
#[async_trait]
impl HttpClient for SimHttpClient {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, String> {
        // Inject faults BEFORE request
        self.inject_network_faults().await?;

        // Make actual HTTP request (through mockito)
        let result = self.inner.send(request).await;

        // Inject faults AFTER request
        self.inject_network_faults().await?;

        result
    }

    async fn send_streaming(
        &self,
        request: HttpRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Bytes, String>> + Send>>, String> {
        // Inject faults BEFORE streaming request
        self.inject_network_faults().await?;

        // Make actual HTTP streaming request
        let stream = self.inner.send_streaming(request).await?;

        // Wrap stream to inject faults on each chunk
        let fault_injector = self.fault_injector.clone();
        let rng = self.rng.clone();

        let faulty_stream = stream.then(move |chunk_result| {
            let fault_injector = fault_injector.clone();
            let rng = rng.clone();

            async move {
                // Inject faults on each chunk
                if let Some(fault) = fault_injector.should_inject("http_stream_chunk") {
                    match fault {
                        kelpie_dst::FaultType::NetworkPacketLoss => {
                            tracing::debug!("HTTP chunk dropped: packet loss fault");
                            return Err("Network packet loss".to_string());
                        }
                        kelpie_dst::FaultType::NetworkDelay { min_ms, max_ms } => {
                            let delay_ms = if min_ms == max_ms {
                                min_ms
                            } else {
                                rng.as_ref().gen_range(min_ms, max_ms)
                            };
                            tracing::debug!(delay_ms = delay_ms, "HTTP chunk delayed");

                            // Use tokio::time::sleep instead of SimClock.sleep_ms()
                            // SimClock.sleep_ms() waits for manual clock advancement which doesn't
                            // happen in async HTTP context, causing deadlock.
                            #[allow(clippy::disallowed_methods)] // Intentional - see comment above
                            tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                        }
                        _ => {}
                    }
                }

                chunk_result
            }
        });

        Ok(Box::pin(faulty_stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_method_as_str() {
        assert_eq!(HttpMethod::Get.as_str(), "GET");
        assert_eq!(HttpMethod::Post.as_str(), "POST");
    }

    #[test]
    fn test_http_request_builder() {
        let req = HttpRequest::new(HttpMethod::Post, "https://example.com".to_string())
            .header("Authorization", "Bearer token");

        assert_eq!(req.method, HttpMethod::Post);
        assert_eq!(req.url, "https://example.com");
        assert_eq!(
            req.headers.get("Authorization"),
            Some(&"Bearer token".to_string())
        );
    }

    #[test]
    fn test_http_response_is_success() {
        let resp = HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: vec![],
        };
        assert!(resp.is_success());

        let resp = HttpResponse {
            status: 404,
            headers: HashMap::new(),
            body: vec![],
        };
        assert!(!resp.is_success());
    }
}
