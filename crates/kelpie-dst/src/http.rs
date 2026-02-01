//! Simulated HTTP Client for DST
//!
//! TigerStyle: Deterministic HTTP simulation with fault injection.
//!
//! This module provides a simulated HTTP client for DST that:
//! - Injects faults based on FaultInjector configuration
//! - Uses deterministic RNG for reproducible behavior
//! - Records all requests for verification
//! - Supports configurable mock responses

use crate::fault::{FaultInjector, FaultType};
use crate::rng::DeterministicRng;
use async_trait::async_trait;
use kelpie_core::http::{HttpClient, HttpError, HttpRequest, HttpResponse, HttpResult};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

// =============================================================================
// TigerStyle Constants
// =============================================================================

/// Maximum recorded requests (to prevent memory issues)
const RECORDED_REQUESTS_MAX: usize = 10_000;

// =============================================================================
// SimHttpClient
// =============================================================================

/// Simulated HTTP client for deterministic testing
///
/// Features:
/// - Fault injection at request time
/// - Configurable mock responses by URL pattern
/// - Request recording for verification
/// - Deterministic behavior with seeded RNG
pub struct SimHttpClient {
    /// Fault injector (shared)
    faults: Arc<FaultInjector>,
    /// Deterministic RNG (for future response variations)
    #[allow(dead_code)]
    rng: RwLock<DeterministicRng>,
    /// Mock responses by URL pattern (prefix match)
    mock_responses: RwLock<HashMap<String, MockResponse>>,
    /// Recorded requests
    recorded_requests: RwLock<Vec<RecordedRequest>>,
    /// Request counter
    request_count: AtomicU64,
    /// Default response for unmatched URLs
    default_response: RwLock<MockResponse>,
}

/// Mock response configuration
#[derive(Debug, Clone)]
pub struct MockResponse {
    /// HTTP status code
    pub status: u16,
    /// Response body
    pub body: String,
    /// Response headers
    pub headers: HashMap<String, String>,
}

impl MockResponse {
    /// Create a successful JSON response
    pub fn json(body: impl Into<String>) -> Self {
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        Self {
            status: 200,
            body: body.into(),
            headers,
        }
    }

    /// Create a successful text response
    pub fn text(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            body: body.into(),
            headers: HashMap::new(),
        }
    }

    /// Create an error response
    pub fn error(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into(),
            headers: HashMap::new(),
        }
    }

    /// Create a 404 Not Found response
    pub fn not_found() -> Self {
        Self::error(404, "Not Found")
    }

    /// Create a 500 Internal Server Error response
    pub fn server_error() -> Self {
        Self::error(500, "Internal Server Error")
    }
}

impl Default for MockResponse {
    fn default() -> Self {
        Self::json(r#"{"status": "ok"}"#)
    }
}

/// Recorded HTTP request for verification
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    /// Request URL
    pub url: String,
    /// HTTP method
    pub method: String,
    /// Request headers
    pub headers: HashMap<String, String>,
    /// Request body
    pub body: Option<String>,
    /// Timestamp (monotonic counter)
    pub timestamp: u64,
    /// Whether a fault was injected
    pub fault_injected: Option<String>,
    /// Response status (if successful)
    pub response_status: Option<u16>,
}

impl SimHttpClient {
    /// Create a new simulated HTTP client
    pub fn new(rng: DeterministicRng, faults: Arc<FaultInjector>) -> Self {
        Self {
            faults,
            rng: RwLock::new(rng),
            mock_responses: RwLock::new(HashMap::new()),
            recorded_requests: RwLock::new(Vec::new()),
            request_count: AtomicU64::new(0),
            default_response: RwLock::new(MockResponse::default()),
        }
    }

    /// Set a mock response for a URL pattern (prefix match)
    pub async fn mock_url(&self, url_pattern: impl Into<String>, response: MockResponse) {
        let mut mocks = self.mock_responses.write().await;
        mocks.insert(url_pattern.into(), response);
    }

    /// Set the default response for unmatched URLs
    pub async fn set_default_response(&self, response: MockResponse) {
        *self.default_response.write().await = response;
    }

    /// Get all recorded requests
    pub async fn get_requests(&self) -> Vec<RecordedRequest> {
        self.recorded_requests.read().await.clone()
    }

    /// Get request count
    pub fn request_count(&self) -> u64 {
        self.request_count.load(Ordering::SeqCst)
    }

    /// Clear recorded requests
    pub async fn clear_requests(&self) {
        self.recorded_requests.write().await.clear();
    }

    /// Check for fault injection
    fn check_fault(&self, operation: &str) -> Option<FaultType> {
        self.faults.should_inject(operation)
    }

    /// Find mock response for URL
    async fn find_mock_response(&self, url: &str) -> MockResponse {
        let mocks = self.mock_responses.read().await;

        // Try exact match first
        if let Some(resp) = mocks.get(url) {
            return resp.clone();
        }

        // Try prefix match
        for (pattern, resp) in mocks.iter() {
            if url.starts_with(pattern) {
                return resp.clone();
            }
        }

        // Return default
        self.default_response.read().await.clone()
    }

    /// Record a request
    async fn record_request(
        &self,
        request: &HttpRequest,
        fault: Option<&str>,
        response_status: Option<u16>,
    ) {
        let mut requests = self.recorded_requests.write().await;

        // Limit recorded requests to prevent memory issues
        if requests.len() >= RECORDED_REQUESTS_MAX {
            requests.remove(0);
        }

        let timestamp = self.request_count.fetch_add(1, Ordering::SeqCst);

        requests.push(RecordedRequest {
            url: request.url.clone(),
            method: request.method.to_string(),
            headers: request.headers.clone(),
            body: request.body.clone(),
            timestamp,
            fault_injected: fault.map(|s| s.to_string()),
            response_status,
        });
    }
}

#[async_trait]
impl HttpClient for SimHttpClient {
    async fn execute(&self, request: HttpRequest) -> HttpResult<HttpResponse> {
        // Check for fault injection
        if let Some(fault) = self.check_fault("http_request") {
            let fault_name = fault.name();
            self.record_request(&request, Some(fault_name), None).await;

            return match fault {
                FaultType::HttpConnectionFail => Err(HttpError::ConnectionFailed {
                    reason: "DST fault injection: simulated connection failure".to_string(),
                }),
                FaultType::HttpTimeout { timeout_ms } => Err(HttpError::Timeout { timeout_ms }),
                FaultType::HttpServerError { status } => {
                    // Return error response instead of Err
                    Ok(HttpResponse::new(
                        status,
                        format!("DST fault injection: simulated {} error", status),
                    ))
                }
                FaultType::HttpResponseTooLarge { max_bytes } => Err(HttpError::ResponseTooLarge {
                    size: max_bytes + 1,
                    max: max_bytes,
                }),
                FaultType::HttpRateLimited { retry_after_ms: _ } => {
                    Ok(HttpResponse::new(429, "Rate limited").with_header("Retry-After", "60"))
                }
                _ => {
                    // Unknown fault type, proceed normally
                    let mock = self.find_mock_response(&request.url).await;
                    Ok(HttpResponse {
                        status: mock.status,
                        headers: mock.headers,
                        body: mock.body,
                    })
                }
            };
        }

        // No fault, return mock response
        let mock = self.find_mock_response(&request.url).await;
        self.record_request(&request, None, Some(mock.status)).await;

        Ok(HttpResponse {
            status: mock.status,
            headers: mock.headers,
            body: mock.body,
        })
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fault::{FaultConfig, FaultInjectorBuilder};

    fn create_no_fault_injector() -> Arc<FaultInjector> {
        let rng = DeterministicRng::new(42);
        Arc::new(FaultInjectorBuilder::new(rng).build())
    }

    fn create_http_fault_injector(probability: f64) -> Arc<FaultInjector> {
        let rng = DeterministicRng::new(42);
        Arc::new(
            FaultInjectorBuilder::new(rng)
                .with_http_faults(probability)
                .build(),
        )
    }

    #[tokio::test]
    async fn test_sim_http_basic_request() {
        let rng = DeterministicRng::new(42);
        let faults = create_no_fault_injector();
        let client = SimHttpClient::new(rng, faults);

        let response = client.get("https://example.com").await.unwrap();
        assert_eq!(response.status, 200);
        assert!(response.is_success());
    }

    #[tokio::test]
    async fn test_sim_http_mock_response() {
        let rng = DeterministicRng::new(42);
        let faults = create_no_fault_injector();
        let client = SimHttpClient::new(rng, faults);

        client
            .mock_url(
                "https://api.example.com",
                MockResponse::json(r#"{"data": "test"}"#),
            )
            .await;

        let response = client
            .get("https://api.example.com/endpoint")
            .await
            .unwrap();
        assert_eq!(response.status, 200);
        let json = response.json().unwrap();
        assert_eq!(json["data"], "test");
    }

    #[tokio::test]
    async fn test_sim_http_recorded_requests() {
        let rng = DeterministicRng::new(42);
        let faults = create_no_fault_injector();
        let client = SimHttpClient::new(rng, faults);

        client.get("https://example.com/1").await.unwrap();
        client.get("https://example.com/2").await.unwrap();

        let requests = client.get_requests().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url, "https://example.com/1");
        assert_eq!(requests[1].url, "https://example.com/2");
    }

    #[tokio::test]
    async fn test_sim_http_with_connection_fault() {
        let rng = DeterministicRng::new(42);
        let fault_rng = DeterministicRng::new(42);
        let faults = Arc::new(
            FaultInjectorBuilder::new(fault_rng)
                .with_fault(FaultConfig::new(FaultType::HttpConnectionFail, 1.0))
                .build(),
        );
        let client = SimHttpClient::new(rng, faults);

        let result = client.get("https://example.com").await;
        assert!(result.is_err());
        assert!(matches!(result, Err(HttpError::ConnectionFailed { .. })));
    }

    #[tokio::test]
    async fn test_sim_http_with_timeout_fault() {
        let rng = DeterministicRng::new(42);
        let fault_rng = DeterministicRng::new(42);
        let faults = Arc::new(
            FaultInjectorBuilder::new(fault_rng)
                .with_fault(FaultConfig::new(
                    FaultType::HttpTimeout { timeout_ms: 5000 },
                    1.0,
                ))
                .build(),
        );
        let client = SimHttpClient::new(rng, faults);

        let result = client.get("https://example.com").await;
        assert!(result.is_err());
        assert!(matches!(
            result,
            Err(HttpError::Timeout { timeout_ms: 5000 })
        ));
    }

    #[tokio::test]
    async fn test_sim_http_with_server_error_fault() {
        let rng = DeterministicRng::new(42);
        let fault_rng = DeterministicRng::new(42);
        let faults = Arc::new(
            FaultInjectorBuilder::new(fault_rng)
                .with_fault(FaultConfig::new(
                    FaultType::HttpServerError { status: 503 },
                    1.0,
                ))
                .build(),
        );
        let client = SimHttpClient::new(rng, faults);

        let result = client.get("https://example.com").await.unwrap();
        assert_eq!(result.status, 503);
        assert!(!result.is_success());
    }

    #[tokio::test]
    async fn test_sim_http_determinism() {
        let run_test = |seed: u64| async move {
            let rng = DeterministicRng::new(seed);
            let faults = create_http_fault_injector(0.5);
            let client = SimHttpClient::new(rng, faults);

            let mut results = Vec::new();
            for i in 0..10 {
                let result = client.get(&format!("https://example.com/{}", i)).await;
                results.push(result.is_ok());
            }
            results
        };

        let results1 = run_test(12345).await;
        let results2 = run_test(12345).await;

        assert_eq!(results1, results2, "SimHttp should be deterministic");
    }

    #[tokio::test]
    async fn test_sim_http_rate_limited() {
        let rng = DeterministicRng::new(42);
        let fault_rng = DeterministicRng::new(42);
        let faults = Arc::new(
            FaultInjectorBuilder::new(fault_rng)
                .with_fault(FaultConfig::new(
                    FaultType::HttpRateLimited {
                        retry_after_ms: 60_000,
                    },
                    1.0,
                ))
                .build(),
        );
        let client = SimHttpClient::new(rng, faults);

        let result = client.get("https://example.com").await.unwrap();
        assert_eq!(result.status, 429);
        assert_eq!(result.headers.get("Retry-After"), Some(&"60".to_string()));
    }
}
