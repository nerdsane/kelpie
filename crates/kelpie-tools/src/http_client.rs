//! HTTP Client Abstraction
//!
//! TigerStyle: Abstract HTTP client trait for DST compatibility.
//!
//! This module provides an abstraction over HTTP clients to enable:
//! - Production use with reqwest
//! - DST testing with SimHttp (deterministic, injectable faults)
//!
//! # Example
//!
//! ```rust,ignore
//! use kelpie_tools::http_client::{HttpClient, ReqwestHttpClient};
//! use std::sync::Arc;
//!
//! // Production
//! let client: Arc<dyn HttpClient> = Arc::new(ReqwestHttpClient::new());
//!
//! // DST testing
//! #[cfg(feature = "dst")]
//! let client: Arc<dyn HttpClient> = Arc::new(SimHttpClient::new(fault_injector));
//! ```

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

// =============================================================================
// Re-exports from kelpie-core
// =============================================================================

// Re-export core HTTP types for backwards compatibility
pub use kelpie_core::http::{
    HttpClient, HttpError, HttpMethod, HttpRequest, HttpResponse, HttpResult,
    HTTP_CLIENT_RESPONSE_BYTES_MAX, HTTP_CLIENT_TIMEOUT_MS_DEFAULT,
};

// =============================================================================
// Reqwest Implementation
// =============================================================================

/// Production HTTP client using reqwest
pub struct ReqwestHttpClient {
    client: reqwest::Client,
}

impl ReqwestHttpClient {
    /// Create a new reqwest HTTP client
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(HTTP_CLIENT_TIMEOUT_MS_DEFAULT))
            .build()
            .expect("Failed to create HTTP client");

        Self { client }
    }

    /// Create with custom timeout
    pub fn with_timeout(timeout: Duration) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("Failed to create HTTP client");

        Self { client }
    }
}

impl Default for ReqwestHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn execute(&self, request: HttpRequest) -> HttpResult<HttpResponse> {
        // Build request
        let mut builder = match request.method {
            HttpMethod::Get => self.client.get(&request.url),
            HttpMethod::Post => self.client.post(&request.url),
            HttpMethod::Put => self.client.put(&request.url),
            HttpMethod::Patch => self.client.patch(&request.url),
            HttpMethod::Delete => self.client.delete(&request.url),
        };

        // Add headers
        for (key, value) in &request.headers {
            builder = builder.header(key, value);
        }

        // Add body
        if let Some(body) = request.body {
            builder = builder.body(body);
        }

        // Set timeout
        builder = builder.timeout(request.timeout);

        // Execute
        let response = builder.send().await.map_err(|e| {
            if e.is_timeout() {
                HttpError::Timeout {
                    timeout_ms: request.timeout.as_millis() as u64,
                }
            } else if e.is_connect() {
                HttpError::ConnectionFailed {
                    reason: e.to_string(),
                }
            } else {
                HttpError::RequestFailed {
                    reason: e.to_string(),
                }
            }
        })?;

        let status = response.status().as_u16();

        // Extract headers
        let mut headers = HashMap::new();
        for (key, value) in response.headers() {
            if let Ok(v) = value.to_str() {
                headers.insert(key.to_string(), v.to_string());
            }
        }

        // Get body
        let body = response
            .text()
            .await
            .map_err(|e| HttpError::RequestFailed {
                reason: e.to_string(),
            })?;

        // Check size
        if body.len() as u64 > HTTP_CLIENT_RESPONSE_BYTES_MAX {
            return Err(HttpError::ResponseTooLarge {
                size: body.len() as u64,
                max: HTTP_CLIENT_RESPONSE_BYTES_MAX,
            });
        }

        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

// =============================================================================
// Default Client Factory
// =============================================================================

/// Create the default HTTP client for production use
pub fn default_http_client() -> Arc<dyn HttpClient> {
    Arc::new(ReqwestHttpClient::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_request_builder() {
        let req = HttpRequest::get("https://example.com")
            .with_header("Authorization", "Bearer token")
            .with_timeout(Duration::from_secs(10));

        assert_eq!(req.method, HttpMethod::Get);
        assert_eq!(req.url, "https://example.com");
        assert_eq!(
            req.headers.get("Authorization"),
            Some(&"Bearer token".to_string())
        );
        assert_eq!(req.timeout, Duration::from_secs(10));
    }

    #[test]
    fn test_http_response() {
        let resp = HttpResponse::new(200, r#"{"key": "value"}"#);

        assert!(resp.is_success());
        assert_eq!(resp.status, 200);

        let json = resp.json().unwrap();
        assert_eq!(json["key"], "value");
    }

    #[test]
    fn test_http_response_not_success() {
        let resp = HttpResponse::new(404, "Not Found");
        assert!(!resp.is_success());
    }

    #[test]
    fn test_http_method_display() {
        assert_eq!(HttpMethod::Get.to_string(), "GET");
        assert_eq!(HttpMethod::Post.to_string(), "POST");
        assert_eq!(HttpMethod::Put.to_string(), "PUT");
        assert_eq!(HttpMethod::Patch.to_string(), "PATCH");
        assert_eq!(HttpMethod::Delete.to_string(), "DELETE");
    }
}
