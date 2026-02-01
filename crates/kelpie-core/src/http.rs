//! HTTP Client Abstraction
//!
//! TigerStyle: Abstract HTTP client trait for DST compatibility.
//!
//! This module provides an abstraction over HTTP clients to enable:
//! - Production use with reqwest (in kelpie-tools)
//! - DST testing with SimHttp (in kelpie-dst)

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

// =============================================================================
// TigerStyle Constants
// =============================================================================

/// Default HTTP timeout in milliseconds
pub const HTTP_CLIENT_TIMEOUT_MS_DEFAULT: u64 = 30_000;

/// Maximum response body size in bytes
pub const HTTP_CLIENT_RESPONSE_BYTES_MAX: u64 = 10 * 1024 * 1024; // 10MB

// =============================================================================
// HTTP Method
// =============================================================================

/// HTTP request method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpMethod::Get => write!(f, "GET"),
            HttpMethod::Post => write!(f, "POST"),
            HttpMethod::Put => write!(f, "PUT"),
            HttpMethod::Patch => write!(f, "PATCH"),
            HttpMethod::Delete => write!(f, "DELETE"),
        }
    }
}

// =============================================================================
// HTTP Request
// =============================================================================

/// HTTP request configuration
#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// HTTP method
    pub method: HttpMethod,
    /// Request URL
    pub url: String,
    /// Request headers
    pub headers: HashMap<String, String>,
    /// Request body (for POST/PUT/PATCH)
    pub body: Option<String>,
    /// Request timeout
    pub timeout: Duration,
}

impl HttpRequest {
    /// Create a new GET request
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: HttpMethod::Get,
            url: url.into(),
            headers: HashMap::new(),
            body: None,
            timeout: Duration::from_millis(HTTP_CLIENT_TIMEOUT_MS_DEFAULT),
        }
    }

    /// Create a new POST request
    pub fn post(url: impl Into<String>) -> Self {
        Self {
            method: HttpMethod::Post,
            url: url.into(),
            headers: HashMap::new(),
            body: None,
            timeout: Duration::from_millis(HTTP_CLIENT_TIMEOUT_MS_DEFAULT),
        }
    }

    /// Set request body
    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Set JSON body
    pub fn with_json_body(mut self, json: &Value) -> Self {
        self.body = Some(serde_json::to_string(json).unwrap_or_default());
        self.headers
            .insert("Content-Type".to_string(), "application/json".to_string());
        self
    }

    /// Add a header
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Set timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

// =============================================================================
// HTTP Response
// =============================================================================

/// HTTP response
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code
    pub status: u16,
    /// Response headers
    pub headers: HashMap<String, String>,
    /// Response body
    pub body: String,
}

impl HttpResponse {
    /// Create a new response
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            body: body.into(),
        }
    }

    /// Check if status is success (2xx)
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Parse body as JSON
    pub fn json(&self) -> Result<Value, serde_json::Error> {
        serde_json::from_str(&self.body)
    }

    /// Add a header
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }
}

// =============================================================================
// HTTP Error
// =============================================================================

/// HTTP client errors
#[derive(Debug, Clone)]
pub enum HttpError {
    /// Request timed out
    Timeout { timeout_ms: u64 },
    /// Connection failed
    ConnectionFailed { reason: String },
    /// Request failed
    RequestFailed { reason: String },
    /// Response too large
    ResponseTooLarge { size: u64, max: u64 },
    /// Invalid URL
    InvalidUrl { url: String },
    /// DST fault injection
    FaultInjected { fault: String },
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Timeout { timeout_ms } => {
                write!(f, "HTTP request timed out after {}ms", timeout_ms)
            }
            HttpError::ConnectionFailed { reason } => {
                write!(f, "HTTP connection failed: {}", reason)
            }
            HttpError::RequestFailed { reason } => write!(f, "HTTP request failed: {}", reason),
            HttpError::ResponseTooLarge { size, max } => {
                write!(
                    f,
                    "HTTP response too large: {} bytes (max: {} bytes)",
                    size, max
                )
            }
            HttpError::InvalidUrl { url } => write!(f, "Invalid URL: {}", url),
            HttpError::FaultInjected { fault } => write!(f, "DST fault injected: {}", fault),
        }
    }
}

impl std::error::Error for HttpError {}

/// HTTP client result type
pub type HttpResult<T> = Result<T, HttpError>;

// =============================================================================
// HTTP Client Trait
// =============================================================================

/// Abstract HTTP client trait
///
/// This trait allows swapping HTTP implementations for testing.
/// Production code uses ReqwestHttpClient (in kelpie-tools),
/// DST tests use SimHttpClient (in kelpie-dst).
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// Execute an HTTP request
    async fn execute(&self, request: HttpRequest) -> HttpResult<HttpResponse>;

    /// Convenience method for GET requests
    async fn get(&self, url: &str) -> HttpResult<HttpResponse> {
        self.execute(HttpRequest::get(url)).await
    }

    /// Convenience method for POST with JSON body
    async fn post_json(&self, url: &str, body: &Value) -> HttpResult<HttpResponse> {
        self.execute(HttpRequest::post(url).with_json_body(body))
            .await
    }
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
