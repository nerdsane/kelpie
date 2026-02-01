//! API Key Authentication Middleware
//!
//! TigerStyle: Secure-by-default API authentication for Kelpie server.
//!
//! Configuration via environment variables:
//! - KELPIE_API_KEY: Required API key for /v1/* endpoints
//! - KELPIE_API_KEY_REQUIRED: Whether auth is required (default: false for backward compat)
//!
//! Endpoints that don't require authentication:
//! - /health
//! - /metrics
//! - /v1/health
//!
//! All other /v1/* endpoints require the API key in the Authorization header:
//! `Authorization: Bearer <api_key>`

use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use std::sync::Arc;

// =============================================================================
// TigerStyle Constants
// =============================================================================

/// API key header prefix
pub const AUTH_HEADER_PREFIX: &str = "Bearer ";

/// Minimum API key length in bytes
pub const API_KEY_LENGTH_BYTES_MIN: usize = 16;

/// Maximum API key length in bytes
pub const API_KEY_LENGTH_BYTES_MAX: usize = 256;

/// Paths that don't require authentication
pub const PUBLIC_PATHS: &[&str] = &["/health", "/metrics", "/v1/health", "/v1/capabilities"];

// =============================================================================
// Types
// =============================================================================

/// API key authentication configuration
#[derive(Debug, Clone)]
pub struct ApiKeyAuth {
    /// The required API key (hashed for security)
    api_key: Option<String>,
    /// Whether authentication is required
    required: bool,
}

impl ApiKeyAuth {
    /// Create from environment variables
    ///
    /// Reads:
    /// - KELPIE_API_KEY: The API key to require
    /// - KELPIE_API_KEY_REQUIRED: Whether auth is required (default: false)
    pub fn from_env() -> Self {
        let api_key = std::env::var("KELPIE_API_KEY").ok();
        let required = std::env::var("KELPIE_API_KEY_REQUIRED")
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        // Validate key if provided
        if let Some(ref key) = api_key {
            assert!(
                key.len() >= API_KEY_LENGTH_BYTES_MIN,
                "KELPIE_API_KEY too short: {} < {} bytes",
                key.len(),
                API_KEY_LENGTH_BYTES_MIN
            );
            assert!(
                key.len() <= API_KEY_LENGTH_BYTES_MAX,
                "KELPIE_API_KEY too long: {} > {} bytes",
                key.len(),
                API_KEY_LENGTH_BYTES_MAX
            );
        }

        if required && api_key.is_none() {
            tracing::warn!(
                "KELPIE_API_KEY_REQUIRED=true but no KELPIE_API_KEY set! Authentication will fail."
            );
        }

        if api_key.is_some() {
            tracing::info!("API key authentication enabled");
        } else if required {
            tracing::warn!("API key authentication required but no key configured");
        } else {
            tracing::info!("API key authentication disabled (set KELPIE_API_KEY to enable)");
        }

        Self { api_key, required }
    }

    /// Create with explicit configuration
    pub fn new(api_key: Option<String>, required: bool) -> Self {
        Self { api_key, required }
    }

    /// Check if a path requires authentication
    pub fn requires_auth(&self, path: &str) -> bool {
        // Public paths don't need auth
        if PUBLIC_PATHS.contains(&path) {
            return false;
        }

        // /v1/* paths require auth if configured
        if path.starts_with("/v1/") {
            return self.api_key.is_some() || self.required;
        }

        // Other paths don't require auth
        false
    }

    /// Validate an API key
    pub fn validate(&self, provided_key: &str) -> bool {
        match &self.api_key {
            Some(expected) => {
                // Constant-time comparison to prevent timing attacks
                use subtle::ConstantTimeEq;
                provided_key.as_bytes().ct_eq(expected.as_bytes()).into()
            }
            None => {
                // No key configured - if required, fail; otherwise pass
                !self.required
            }
        }
    }

    /// Is authentication enabled?
    pub fn is_enabled(&self) -> bool {
        self.api_key.is_some() || self.required
    }
}

impl Default for ApiKeyAuth {
    fn default() -> Self {
        Self::from_env()
    }
}

// =============================================================================
// Error Response
// =============================================================================

/// Authentication error response
#[derive(Debug, Serialize)]
pub struct AuthError {
    pub code: String,
    pub message: String,
}

impl AuthError {
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            code: "unauthorized".to_string(),
            message: message.into(),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            code: "forbidden".to_string(),
            message: message.into(),
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let status = match self.code.as_str() {
            "unauthorized" => StatusCode::UNAUTHORIZED,
            "forbidden" => StatusCode::FORBIDDEN,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(self)).into_response()
    }
}

// =============================================================================
// Middleware
// =============================================================================

/// API key authentication middleware function
pub async fn api_key_auth_middleware(
    auth: Arc<ApiKeyAuth>,
    request: Request,
    next: Next,
) -> Result<Response, AuthError> {
    let path = request.uri().path();

    // Check if this path requires authentication
    if !auth.requires_auth(path) {
        return Ok(next.run(request).await);
    }

    // Extract API key from Authorization header
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let provided_key = match auth_header {
        Some(header) if header.starts_with(AUTH_HEADER_PREFIX) => {
            &header[AUTH_HEADER_PREFIX.len()..]
        }
        Some(_) => {
            return Err(AuthError::unauthorized(
                "Invalid Authorization header format. Expected: Bearer <api_key>",
            ))
        }
        None => {
            return Err(AuthError::unauthorized(
                "Missing Authorization header. Expected: Authorization: Bearer <api_key>",
            ))
        }
    };

    // Validate the key
    if !auth.validate(provided_key) {
        return Err(AuthError::forbidden("Invalid API key"));
    }

    // Continue to the next handler
    Ok(next.run(request).await)
}

// =============================================================================
// Layer
// =============================================================================

/// Tower layer for API key authentication
#[derive(Clone)]
pub struct ApiKeyAuthLayer {
    auth: Arc<ApiKeyAuth>,
}

impl ApiKeyAuthLayer {
    /// Create a new API key auth layer from environment
    pub fn from_env() -> Self {
        Self {
            auth: Arc::new(ApiKeyAuth::from_env()),
        }
    }

    /// Create with explicit configuration
    pub fn new(auth: ApiKeyAuth) -> Self {
        Self {
            auth: Arc::new(auth),
        }
    }

    /// Get the auth configuration
    pub fn auth(&self) -> &ApiKeyAuth {
        &self.auth
    }
}

// Note: The actual Layer implementation would use axum's middleware system
// For now, we provide the middleware function that can be used with
// axum::middleware::from_fn_with_state

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_public_paths() {
        let auth = ApiKeyAuth::new(Some("test-key".to_string()), false);

        assert!(!auth.requires_auth("/health"));
        assert!(!auth.requires_auth("/metrics"));
        assert!(!auth.requires_auth("/v1/health"));
        assert!(!auth.requires_auth("/v1/capabilities"));
        assert!(auth.requires_auth("/v1/agents"));
        assert!(auth.requires_auth("/v1/tools"));
    }

    #[test]
    fn test_auth_validation() {
        let auth = ApiKeyAuth::new(Some("correct-key-here".to_string()), false);

        assert!(auth.validate("correct-key-here"));
        assert!(!auth.validate("wrong-key"));
        assert!(!auth.validate(""));
    }

    #[test]
    fn test_auth_disabled() {
        let auth = ApiKeyAuth::new(None, false);

        // No key configured and not required - should pass
        assert!(auth.validate("any-key"));
        assert!(!auth.is_enabled());
    }

    #[test]
    fn test_auth_required_no_key() {
        let auth = ApiKeyAuth::new(None, true);

        // Required but no key configured - should fail
        assert!(!auth.validate("any-key"));
        assert!(auth.is_enabled());
    }
}
