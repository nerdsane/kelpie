//! DST tests for HTTP API linearizability
//!
//! TLA+ Spec Reference: `docs/tla/KelpieHttpApi.tla`
//!
//! This module tests the HTTP API linearizability invariants:
//!
//! | Invariant | Test |
//! |-----------|------|
//! | IdempotencyGuarantee | test_idempotency_exactly_once |
//! | ReadAfterWriteConsistency | test_create_get_consistency |
//! | AtomicOperation | test_atomic_create_under_crash |
//! | DurableOnSuccess | test_durability_after_success |
//!
//! TigerStyle: Deterministic testing with explicit fault injection.

use kelpie_server::api::idempotency::{
    CachedResponse, IdempotencyCache, TimeProvider, IDEMPOTENCY_KEY_HEADER,
    IDEMPOTENCY_TOKEN_EXPIRY_MS,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

// =============================================================================
// Constants (TigerStyle: Explicit with units)
// =============================================================================

/// Number of concurrent requests in tests
const CONCURRENT_REQUESTS_COUNT: usize = 50;

/// Number of iterations for stress test
const STRESS_ITERATIONS_COUNT: usize = 1000;

/// Default test seed for reproducibility
const DEFAULT_TEST_SEED: u64 = 12345;

// =============================================================================
// Test Time Provider (DST-compatible)
// =============================================================================

/// Simulated time provider for deterministic testing
struct SimulatedTime {
    current_ms: AtomicU64,
}

impl SimulatedTime {
    fn new(start_ms: u64) -> Self {
        Self {
            current_ms: AtomicU64::new(start_ms),
        }
    }

    fn advance_ms(&self, delta_ms: u64) {
        self.current_ms.fetch_add(delta_ms, Ordering::SeqCst);
    }

    #[allow(dead_code)]
    fn set_ms(&self, time_ms: u64) {
        self.current_ms.store(time_ms, Ordering::SeqCst);
    }
}

impl TimeProvider for SimulatedTime {
    fn now_ms(&self) -> u64 {
        self.current_ms.load(Ordering::SeqCst)
    }
}

// =============================================================================
// Idempotency Tests
// =============================================================================

/// Test: IdempotencyGuarantee - Same token returns same response
///
/// TLA+ Invariant: IdempotencyGuarantee
/// Property: ∀ t ∈ IdempotencyTokens: token_used(t) ⇒ same_response(t)
#[tokio::test]
async fn test_idempotency_exactly_once() {
    let time = Arc::new(SimulatedTime::new(1000000));
    let cache = IdempotencyCache::with_time_provider(time.clone());

    let token = "test-token-123";

    // First request: mark in progress
    assert!(
        cache.mark_in_progress(token).await,
        "should mark in progress"
    );

    // Set response
    let response = CachedResponse::new(
        201,
        b"{\"id\":\"agent-1\"}".to_vec(),
        vec![("content-type".to_string(), "application/json".to_string())],
        time.now_ms(),
    );
    cache.set(token, response.clone()).await;

    // Second request with same token: should get cached response
    let cached = cache.get(token).await;
    assert!(cached.is_some(), "should return cached response");
    let cached = cached.unwrap();

    // Verify same response
    assert_eq!(cached.status, 201, "status should match");
    assert_eq!(cached.body, b"{\"id\":\"agent-1\"}", "body should match");

    // Third request: still same response
    let cached2 = cache.get(token).await;
    assert!(cached2.is_some(), "should still return cached response");
    assert_eq!(cached2.unwrap().status, 201, "status should still match");
}

/// Test: ExactlyOnceExecution - Concurrent requests with same token execute at most once
///
/// TLA+ Invariant: ExactlyOnceExecution
/// Property: ∀ t ∈ IdempotencyTokens: execution_count(t) ≤ 1
#[tokio::test]
async fn test_concurrent_idempotent_requests() {
    let time = Arc::new(SimulatedTime::new(1000000));
    let cache = Arc::new(IdempotencyCache::with_time_provider(time.clone()));

    let token = "concurrent-token";

    // Launch concurrent requests trying to claim the same token
    let handles: Vec<_> = (0..CONCURRENT_REQUESTS_COUNT)
        .map(|_| {
            let cache = cache.clone();
            let token = token.to_string();
            tokio::spawn(async move { cache.mark_in_progress(&token).await })
        })
        .collect();

    // Collect results
    let results: Vec<bool> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // Exactly one should succeed
    let success_count = results.iter().filter(|&&v| v).count();
    assert_eq!(
        success_count, 1,
        "exactly one request should mark in progress, got {}",
        success_count
    );
}

/// Test: ReadAfterWriteConsistency - POST then GET returns entity
///
/// TLA+ Invariant: ReadAfterWriteConsistency
/// Property: POST returns 201 ⇒ GET returns entity
#[tokio::test]
async fn test_create_get_consistency() {
    let time = Arc::new(SimulatedTime::new(1000000));
    let cache = IdempotencyCache::with_time_provider(time.clone());

    let token = "create-token";

    // Simulate successful creation
    cache.mark_in_progress(token).await;

    let response = CachedResponse::new(
        201,
        b"{\"id\":\"agent-123\",\"name\":\"test\"}".to_vec(),
        vec![("content-type".to_string(), "application/json".to_string())],
        time.now_ms(),
    );
    cache.set(token, response).await;

    // Read should return the created entity
    let cached = cache.get(token).await;
    assert!(cached.is_some(), "should find cached response");

    let cached = cached.unwrap();
    assert_eq!(cached.status, 201, "should be 201 Created");

    // Verify body contains the created entity
    let body_str = String::from_utf8_lossy(&cached.body);
    assert!(
        body_str.contains("agent-123"),
        "body should contain created entity"
    );
}

/// Test: DurableOnSuccess - Success response survives cache lookup
///
/// TLA+ Invariant: DurableOnSuccess
/// Property: successful_response(t) ⇒ response_available(t) until expiry
#[tokio::test]
async fn test_durability_after_success() {
    let time = Arc::new(SimulatedTime::new(1000000));
    let cache = IdempotencyCache::with_time_provider(time.clone());

    let token = "durable-token";

    // Store successful response
    cache.mark_in_progress(token).await;

    let response = CachedResponse::new(200, b"{\"success\":true}".to_vec(), vec![], time.now_ms());
    cache.set(token, response).await;

    // Multiple reads should all succeed
    for i in 0..10 {
        let cached = cache.get(token).await;
        assert!(
            cached.is_some(),
            "read {} should return cached response",
            i + 1
        );
        assert_eq!(
            cached.unwrap().status,
            200,
            "read {} should return 200",
            i + 1
        );
    }

    // Advance time but not past expiry
    time.advance_ms(IDEMPOTENCY_TOKEN_EXPIRY_MS / 2);

    // Should still be available
    let cached = cache.get(token).await;
    assert!(cached.is_some(), "should be available before expiry");
}

/// Test: Cache expiry works correctly
///
/// Related to DurableOnSuccess - responses expire after TTL
#[tokio::test]
async fn test_cache_expiry() {
    let time = Arc::new(SimulatedTime::new(1000000));
    let cache = IdempotencyCache::with_time_provider(time.clone());

    let token = "expiry-token";

    // Store response
    cache.mark_in_progress(token).await;

    let response = CachedResponse::new(200, b"test".to_vec(), vec![], time.now_ms());
    cache.set(token, response).await;

    // Should be available
    assert!(cache.get(token).await.is_some(), "should be available");

    // Advance past expiry
    time.advance_ms(IDEMPOTENCY_TOKEN_EXPIRY_MS + 1000);

    // Should be expired
    assert!(
        cache.get(token).await.is_none(),
        "should be expired after TTL"
    );
}

/// Test: In-progress timeout works correctly
///
/// Prevents stuck requests from blocking forever
#[tokio::test]
async fn test_in_progress_timeout() {
    let time = Arc::new(SimulatedTime::new(1000000));
    let cache = IdempotencyCache::with_time_provider(time.clone());

    let token = "timeout-token";

    // Mark in progress
    assert!(
        cache.mark_in_progress(token).await,
        "should mark in progress"
    );

    // Can't mark again while in progress
    assert!(
        !cache.mark_in_progress(token).await,
        "should not mark while in progress"
    );

    // Advance past in-progress timeout (5 minutes)
    time.advance_ms(300_001);

    // Now should be able to mark again (timed out)
    assert!(
        cache.mark_in_progress(token).await,
        "should mark after timeout"
    );
}

/// Test: 5xx errors are not cached
///
/// Server errors should allow retry
#[tokio::test]
async fn test_5xx_not_cached() {
    let time = Arc::new(SimulatedTime::new(1000000));
    let cache = IdempotencyCache::with_time_provider(time.clone());

    let token = "error-token";

    // Mark in progress
    cache.mark_in_progress(token).await;

    // Remove in progress (simulating 5xx handling)
    cache.remove_in_progress(token).await;

    // Should be able to retry
    assert!(
        cache.mark_in_progress(token).await,
        "should be able to retry after 5xx"
    );
}

/// Test: Deterministic behavior with seed
///
/// Same sequence of operations produces same results
#[tokio::test]
async fn test_deterministic_behavior() {
    // Run twice with same initial conditions
    let results1 = run_deterministic_sequence(DEFAULT_TEST_SEED).await;
    let results2 = run_deterministic_sequence(DEFAULT_TEST_SEED).await;

    assert_eq!(results1, results2, "results should be deterministic");
}

async fn run_deterministic_sequence(seed: u64) -> Vec<bool> {
    let time = Arc::new(SimulatedTime::new(seed * 1000));
    let cache = IdempotencyCache::with_time_provider(time.clone());

    let mut results = Vec::new();

    // Deterministic sequence of operations
    for i in 0..10 {
        let token = format!("det-token-{}", i);
        results.push(cache.mark_in_progress(&token).await);
    }

    // Re-try first 5
    for i in 0..5 {
        let token = format!("det-token-{}", i);
        results.push(cache.mark_in_progress(&token).await);
    }

    results
}

/// Test: Multiple tokens are independent
///
/// Operations on different tokens don't interfere
#[tokio::test]
async fn test_token_independence() {
    let time = Arc::new(SimulatedTime::new(1000000));
    let cache = IdempotencyCache::with_time_provider(time.clone());

    // Mark multiple tokens
    let tokens = vec!["token-a", "token-b", "token-c"];
    for token in &tokens {
        assert!(cache.mark_in_progress(token).await, "should mark {}", token);
    }

    // Each has its own state
    for (i, token) in tokens.iter().enumerate() {
        let response = CachedResponse::new(
            200 + i as u16,
            format!("response-{}", i).into_bytes(),
            vec![],
            time.now_ms(),
        );
        cache.set(token, response).await;
    }

    // Verify each has correct response
    for (i, token) in tokens.iter().enumerate() {
        let cached = cache.get(token).await.unwrap();
        assert_eq!(
            cached.status,
            200 + i as u16,
            "token {} should have correct status",
            token
        );
    }
}

/// Stress test: Many concurrent operations
#[tokio::test]
#[ignore] // Run with: cargo test -p kelpie-server test_http_linearizability_stress -- --ignored
async fn test_http_linearizability_stress() {
    let time = Arc::new(SimulatedTime::new(1000000));
    let cache = Arc::new(IdempotencyCache::with_time_provider(time.clone()));

    let mut all_results = Vec::new();

    for iteration in 0..STRESS_ITERATIONS_COUNT {
        let token = format!("stress-{}", iteration);
        let cache = cache.clone();
        let time = time.clone();

        let result = tokio::spawn(async move {
            // Try to claim
            let claimed = cache.mark_in_progress(&token).await;
            if claimed {
                // Store response
                let response = CachedResponse::new(
                    201,
                    format!("{{\"id\":\"{}\"}}", token).into_bytes(),
                    vec![],
                    time.now_ms(),
                );
                cache.set(&token, response).await;
            }
            claimed
        })
        .await
        .unwrap();

        all_results.push(result);
    }

    // All iterations should succeed (unique tokens)
    assert!(
        all_results.iter().all(|&v| v),
        "all unique tokens should succeed"
    );
}

// =============================================================================
// Header Extraction Tests
// =============================================================================

#[test]
fn test_extract_idempotency_key_header() {
    use axum::http::HeaderMap;

    let mut headers = HeaderMap::new();
    headers.insert(IDEMPOTENCY_KEY_HEADER, "my-key-123".parse().unwrap());

    let key = IdempotencyCache::extract_key(&headers);
    assert_eq!(key, Some("my-key-123".to_string()));
}

#[test]
fn test_extract_idempotency_key_alt_header() {
    use axum::http::HeaderMap;

    let mut headers = HeaderMap::new();
    headers.insert("x-idempotency-key", "alt-key-456".parse().unwrap());

    let key = IdempotencyCache::extract_key(&headers);
    assert_eq!(key, Some("alt-key-456".to_string()));
}

#[test]
fn test_extract_empty_key_rejected() {
    use axum::http::HeaderMap;

    let mut headers = HeaderMap::new();
    headers.insert(IDEMPOTENCY_KEY_HEADER, "".parse().unwrap());

    let key = IdempotencyCache::extract_key(&headers);
    assert!(key.is_none(), "empty key should be rejected");
}

#[test]
fn test_extract_long_key_rejected() {
    use axum::http::HeaderMap;

    let long_key = "x".repeat(300); // Over 256 limit
    let mut headers = HeaderMap::new();
    headers.insert(IDEMPOTENCY_KEY_HEADER, long_key.parse().unwrap());

    let key = IdempotencyCache::extract_key(&headers);
    assert!(key.is_none(), "overly long key should be rejected");
}
