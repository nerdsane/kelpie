//! Idempotency token handling for exactly-once semantics
//!
//! TLA+ Spec Reference: `docs/tla/KelpieHttpApi.tla`
//!
//! This module implements the idempotency layer that ensures HTTP requests
//! with the same idempotency key return the same response and execute at most once.
//!
//! # Invariants (from TLA+ spec)
//!
//! - **IdempotencyGuarantee**: Same token → same response
//! - **ExactlyOnceExecution**: Mutations execute ≤1 time per token
//! - **DurableOnSuccess**: Success → response survives restart
//!
//! # Known Limitations
//!
//! - **In-memory storage**: The current implementation stores cached responses
//!   in-memory only. Responses are lost on server restart, which means the
//!   `DurableOnSuccess` invariant is only satisfied within a single server
//!   lifetime. For production deployments requiring true durability across
//!   restarts, implement persistent storage using FoundationDB.
//!
//! # TigerStyle
//!
//! - All constants have explicit units
//! - Explicit error handling (no unwrap in production paths)
//! - Bounded data structures with explicit limits

use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{HeaderMap, Request, Response, StatusCode},
    middleware::Next,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// =============================================================================
// Constants (TigerStyle: Explicit with units)
// =============================================================================

/// Idempotency token expiry time in milliseconds (1 hour)
pub const IDEMPOTENCY_TOKEN_EXPIRY_MS: u64 = 3_600_000;

/// Maximum number of cached idempotency responses
pub const IDEMPOTENCY_CACHE_ENTRIES_MAX: usize = 100_000;

/// Header name for idempotency key (Stripe-style)
pub const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

/// Alternative header name (common convention)
pub const IDEMPOTENCY_KEY_HEADER_ALT: &str = "x-idempotency-key";

/// Maximum idempotency key length in bytes
pub const IDEMPOTENCY_KEY_LENGTH_MAX: usize = 256;

/// Maximum cached response body size in bytes (1MB)
pub const CACHED_RESPONSE_BODY_BYTES_MAX: usize = 1_048_576;

/// Timeout for in-progress requests in milliseconds (5 minutes)
/// After this time, in-progress requests are considered abandoned and can be retried.
pub const IN_PROGRESS_TIMEOUT_MS: u64 = 300_000;

// =============================================================================
// Cached Response Types
// =============================================================================

/// A cached HTTP response for idempotency
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResponse {
    /// HTTP status code
    pub status: u16,
    /// Response body (JSON serialized)
    pub body: Vec<u8>,
    /// Response headers that should be replayed (content-type, etc.)
    pub headers: Vec<(String, String)>,
    /// When this response was created (milliseconds since epoch)
    pub created_at_ms: u64,
}

impl CachedResponse {
    /// Create a new cached response
    pub fn new(status: u16, body: Vec<u8>, headers: Vec<(String, String)>, now_ms: u64) -> Self {
        // TigerStyle: Precondition assertions
        assert!((100..600).contains(&status), "invalid HTTP status code");
        assert!(
            body.len() <= CACHED_RESPONSE_BODY_BYTES_MAX,
            "response body too large for caching"
        );

        Self {
            status,
            body,
            headers,
            created_at_ms: now_ms,
        }
    }

    /// Check if this cached response has expired
    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.created_at_ms) > IDEMPOTENCY_TOKEN_EXPIRY_MS
    }

    /// Convert to an axum Response
    pub fn to_response(&self) -> Response<Body> {
        let mut response = Response::builder()
            .status(StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR));

        // Add cached headers
        for (name, value) in &self.headers {
            if let Ok(header_name) = name.parse::<axum::http::header::HeaderName>() {
                if let Ok(header_value) = value.parse::<axum::http::header::HeaderValue>() {
                    response = response.header(header_name, header_value);
                }
            }
        }

        // Add marker header to indicate this is a cached response
        response = response.header("x-idempotency-replayed", "true");

        response
            .body(Body::from(self.body.clone()))
            .unwrap_or_else(|_| {
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::empty())
                    .unwrap()
            })
    }
}

// =============================================================================
// Cache Entry State
// =============================================================================

/// State of an idempotency cache entry
#[derive(Debug, Clone)]
enum CacheEntryState {
    /// Request is currently being processed
    InProgress {
        /// When processing started (for timeout detection)
        started_at_ms: u64,
    },
    /// Request completed, response is cached
    Completed(CachedResponse),
}

/// An entry in the idempotency cache
#[derive(Debug, Clone)]
struct CacheEntry {
    /// Current state
    state: CacheEntryState,
    /// Last access time (for LRU eviction)
    last_accessed_ms: u64,
}

// =============================================================================
// Idempotency Cache
// =============================================================================

/// In-memory idempotency cache
///
/// TigerStyle: Thread-safe with explicit locking.
/// Uses RwLock for concurrent reads, exclusive writes.
pub struct IdempotencyCache {
    /// Cache entries by key
    cache: RwLock<HashMap<String, CacheEntry>>,
    /// Current time provider (for DST compatibility)
    time_provider: Arc<dyn TimeProvider>,
}

/// Time provider trait for DST compatibility
pub trait TimeProvider: Send + Sync {
    /// Get current time in milliseconds since epoch
    fn now_ms(&self) -> u64;
}

/// Wall clock time provider for production
pub struct WallClockTime;

impl TimeProvider for WallClockTime {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

impl IdempotencyCache {
    /// Create a new idempotency cache with wall clock time
    pub fn new() -> Self {
        Self::with_time_provider(Arc::new(WallClockTime))
    }

    /// Create a new idempotency cache with custom time provider (for DST)
    pub fn with_time_provider(time_provider: Arc<dyn TimeProvider>) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            time_provider,
        }
    }

    /// Get current time in milliseconds (DST-compatible)
    pub fn now_ms(&self) -> u64 {
        self.time_provider.now_ms()
    }

    /// Extract idempotency key from request headers
    pub fn extract_key(headers: &HeaderMap) -> Option<String> {
        // Try primary header first, then alternative
        headers
            .get(IDEMPOTENCY_KEY_HEADER)
            .or_else(|| headers.get(IDEMPOTENCY_KEY_HEADER_ALT))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty() && s.len() <= IDEMPOTENCY_KEY_LENGTH_MAX)
    }

    /// Get a cached response if available and not expired
    pub async fn get(&self, key: &str) -> Option<CachedResponse> {
        let now_ms = self.time_provider.now_ms();
        let mut cache = self.cache.write().await;

        if let Some(entry) = cache.get_mut(key) {
            match &entry.state {
                CacheEntryState::Completed(response) => {
                    if response.is_expired(now_ms) {
                        // Expired - remove from cache
                        cache.remove(key);
                        return None;
                    }
                    // Update last accessed time
                    entry.last_accessed_ms = now_ms;
                    return Some(response.clone());
                }
                CacheEntryState::InProgress { started_at_ms } => {
                    // Request is in progress
                    // Check for timeout (treat as abandoned)
                    if now_ms.saturating_sub(*started_at_ms) > IN_PROGRESS_TIMEOUT_MS {
                        // Timed out - allow retry
                        cache.remove(key);
                        return None;
                    }
                    // Still in progress - caller should wait or return conflict
                    // For simplicity, we'll let this fall through as None
                    // A more sophisticated implementation could use a semaphore
                    return None;
                }
            }
        }

        None
    }

    /// Mark a request as in-progress
    ///
    /// Returns true if successfully marked, false if already exists
    pub async fn mark_in_progress(&self, key: &str) -> bool {
        let now_ms = self.time_provider.now_ms();
        let mut cache = self.cache.write().await;

        // Evict expired entries if we're at capacity
        if cache.len() >= IDEMPOTENCY_CACHE_ENTRIES_MAX {
            self.evict_expired_sync(&mut cache, now_ms);
        }

        // Still at capacity? Evict oldest entries
        if cache.len() >= IDEMPOTENCY_CACHE_ENTRIES_MAX {
            self.evict_lru_sync(&mut cache);
        }

        // Check if key already exists
        if let Some(entry) = cache.get(key) {
            match &entry.state {
                CacheEntryState::Completed(response) => {
                    if !response.is_expired(now_ms) {
                        return false; // Already completed
                    }
                    // Expired - allow overwrite
                }
                CacheEntryState::InProgress { started_at_ms } => {
                    if now_ms.saturating_sub(*started_at_ms) <= IN_PROGRESS_TIMEOUT_MS {
                        return false; // Still in progress
                    }
                    // Timed out - allow overwrite
                }
            }
        }

        // Insert in-progress entry
        cache.insert(
            key.to_string(),
            CacheEntry {
                state: CacheEntryState::InProgress {
                    started_at_ms: now_ms,
                },
                last_accessed_ms: now_ms,
            },
        );

        true
    }

    /// Store a completed response
    pub async fn set(&self, key: &str, response: CachedResponse) {
        let now_ms = self.time_provider.now_ms();
        let mut cache = self.cache.write().await;

        cache.insert(
            key.to_string(),
            CacheEntry {
                state: CacheEntryState::Completed(response),
                last_accessed_ms: now_ms,
            },
        );
    }

    /// Remove an in-progress marker (on error)
    pub async fn remove_in_progress(&self, key: &str) {
        let mut cache = self.cache.write().await;
        if let Some(entry) = cache.get(key) {
            if matches!(entry.state, CacheEntryState::InProgress { .. }) {
                cache.remove(key);
            }
        }
    }

    /// Evict expired entries (called with lock held)
    fn evict_expired_sync(&self, cache: &mut HashMap<String, CacheEntry>, now_ms: u64) {
        cache.retain(|_, entry| match &entry.state {
            CacheEntryState::Completed(response) => !response.is_expired(now_ms),
            CacheEntryState::InProgress { started_at_ms } => {
                now_ms.saturating_sub(*started_at_ms) <= IN_PROGRESS_TIMEOUT_MS
            }
        });
    }

    /// Evict least recently used entries (called with lock held)
    fn evict_lru_sync(&self, cache: &mut HashMap<String, CacheEntry>) {
        // Find the oldest 10% of entries and remove them
        let target_count = cache.len() / 10;
        if target_count == 0 {
            return;
        }

        let mut entries: Vec<_> = cache
            .iter()
            .map(|(k, v)| (k.clone(), v.last_accessed_ms))
            .collect();
        entries.sort_by_key(|(_, ts)| *ts);

        for (key, _) in entries.into_iter().take(target_count) {
            cache.remove(&key);
        }
    }

    /// Get cache statistics (for monitoring)
    #[allow(dead_code)]
    pub async fn stats(&self) -> IdempotencyCacheStats {
        let cache = self.cache.read().await;
        let mut completed = 0;
        let mut in_progress = 0;

        for entry in cache.values() {
            match entry.state {
                CacheEntryState::Completed(_) => completed += 1,
                CacheEntryState::InProgress { .. } => in_progress += 1,
            }
        }

        IdempotencyCacheStats {
            total: cache.len(),
            completed,
            in_progress,
        }
    }
}

impl Default for IdempotencyCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Cache statistics (for monitoring)
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct IdempotencyCacheStats {
    pub total: usize,
    pub completed: usize,
    pub in_progress: usize,
}

// =============================================================================
// Middleware
// =============================================================================

/// Idempotency middleware for axum
///
/// Checks for idempotency key in request headers and returns cached response
/// if available. Otherwise, lets the request through and caches the response.
pub async fn idempotency_middleware(
    State(cache): State<Arc<IdempotencyCache>>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    // Only apply to mutating methods
    if !is_mutating_method(request.method()) {
        return next.run(request).await;
    }

    // Extract idempotency key
    let key = match IdempotencyCache::extract_key(request.headers()) {
        Some(k) => k,
        None => {
            // No idempotency key - proceed without caching
            return next.run(request).await;
        }
    };

    // Check for cached response
    if let Some(cached) = cache.get(&key).await {
        tracing::debug!(key = %key, "returning cached idempotent response");
        return cached.to_response();
    }

    // Mark as in-progress
    if !cache.mark_in_progress(&key).await {
        // Already in progress or completed - return conflict
        tracing::warn!(key = %key, "idempotent request already in progress");
        return Response::builder()
            .status(StatusCode::CONFLICT)
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"error":"request with this idempotency key is already being processed"}"#,
            ))
            .unwrap();
    }

    // Execute the request
    let response = next.run(request).await;

    // Cache the response if successful (2xx) or client error (4xx)
    // Don't cache 5xx errors as they may be transient
    let status = response.status().as_u16();
    if (200..500).contains(&status) {
        // Extract response parts
        let (parts, body) = response.into_parts();

        // Read body
        match to_bytes(body, CACHED_RESPONSE_BODY_BYTES_MAX).await {
            Ok(bytes) => {
                // Extract headers to cache
                let headers: Vec<(String, String)> = parts
                    .headers
                    .iter()
                    .filter(|(name, _)| {
                        // Only cache content-related headers
                        let name_str = name.as_str().to_lowercase();
                        name_str == "content-type" || name_str.starts_with("x-")
                    })
                    .filter_map(|(name, value)| {
                        value
                            .to_str()
                            .ok()
                            .map(|v| (name.to_string(), v.to_string()))
                    })
                    .collect();

                // Create cached response (use cache's time provider for DST compatibility)
                let cached = CachedResponse::new(status, bytes.to_vec(), headers, cache.now_ms());

                cache.set(&key, cached).await;
                tracing::debug!(key = %key, status = status, "cached idempotent response");

                // Reconstruct response
                Response::from_parts(parts, Body::from(bytes))
            }
            Err(e) => {
                // Failed to read body - remove in-progress marker
                cache.remove_in_progress(&key).await;
                tracing::warn!(key = %key, error = %e, "failed to read response body for caching");

                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::from("failed to process response"))
                    .unwrap()
            }
        }
    } else {
        // 5xx error - remove in-progress marker, don't cache
        cache.remove_in_progress(&key).await;
        response
    }
}

/// Check if HTTP method is mutating (requires idempotency)
fn is_mutating_method(method: &axum::http::Method) -> bool {
    matches!(
        *method,
        axum::http::Method::POST | axum::http::Method::PUT | axum::http::Method::DELETE
    )
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_basic_operations() {
        let cache = IdempotencyCache::new();

        // Initially empty
        assert!(cache.get("key1").await.is_none());

        // Mark in progress
        assert!(cache.mark_in_progress("key1").await);

        // Can't mark again while in progress
        assert!(!cache.mark_in_progress("key1").await);

        // Set response
        let response = CachedResponse::new(
            200,
            b"test body".to_vec(),
            vec![("content-type".to_string(), "application/json".to_string())],
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        );
        cache.set("key1", response.clone()).await;

        // Get returns cached response
        let cached = cache.get("key1").await;
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().status, 200);
    }

    #[tokio::test]
    async fn test_extract_key() {
        let mut headers = HeaderMap::new();

        // No key
        assert!(IdempotencyCache::extract_key(&headers).is_none());

        // Primary header
        headers.insert(IDEMPOTENCY_KEY_HEADER, "test-key-123".parse().unwrap());
        assert_eq!(
            IdempotencyCache::extract_key(&headers),
            Some("test-key-123".to_string())
        );

        // Empty key is rejected
        headers.insert(IDEMPOTENCY_KEY_HEADER, "".parse().unwrap());
        assert!(IdempotencyCache::extract_key(&headers).is_none());
    }

    #[tokio::test]
    async fn test_cached_response_expiry() {
        // Create a response that's already expired
        let response = CachedResponse::new(
            200,
            b"test".to_vec(),
            vec![],
            0, // Created at epoch
        );

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        assert!(response.is_expired(now_ms));

        // Fresh response is not expired
        let fresh = CachedResponse::new(200, b"test".to_vec(), vec![], now_ms);
        assert!(!fresh.is_expired(now_ms));
    }

    #[test]
    fn test_is_mutating_method() {
        assert!(is_mutating_method(&axum::http::Method::POST));
        assert!(is_mutating_method(&axum::http::Method::PUT));
        assert!(is_mutating_method(&axum::http::Method::DELETE));
        assert!(!is_mutating_method(&axum::http::Method::GET));
        assert!(!is_mutating_method(&axum::http::Method::HEAD));
        assert!(!is_mutating_method(&axum::http::Method::OPTIONS));
    }
}
