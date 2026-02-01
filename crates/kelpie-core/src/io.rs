//! I/O Abstraction Layer for Deterministic Simulation Testing
//!
//! TigerStyle: All external I/O goes through abstraction traits.
//!
//! This module provides the foundation for true DST (Deterministic Simulation Testing)
//! by abstracting all non-deterministic operations:
//!
//! - **Time**: Wall clock vs simulated time
//! - **Random**: std::rand vs deterministic RNG
//!
//! The same business logic code runs in both production and DST modes.
//! Only the I/O implementations differ.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │         Business Logic (SAME CODE)                  │
//! └──────────────────────┬──────────────────────────────┘
//!                        │
//! ┌──────────────────────▼──────────────────────────────┐
//! │              I/O Abstraction Layer                  │
//! │  TimeProvider, RngProvider                          │
//! └──────────────────────┬──────────────────────────────┘
//!                        │
//!           ┌────────────┴────────────┐
//!           │                         │
//!     ┌─────▼─────┐            ┌─────▼─────┐
//!     │ Production│            │    DST    │
//!     │ WallClock │            │ SimClock  │
//!     │ StdRng    │            │ DetRng    │
//!     └───────────┘            └───────────┘
//! ```

use crate::runtime::Runtime;
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

// ============================================================================
// Time Provider
// ============================================================================

/// Time provider abstraction for DST
///
/// All code that needs current time or sleep MUST use this trait.
/// Never use `std::time::SystemTime::now()` or `chrono::Utc::now()` directly.
///
/// # Implementations
///
/// - `WallClockTime`: Production - uses system clock
/// - `SimClock` (in kelpie-dst): DST - deterministic, manually advanced
#[async_trait]
pub trait TimeProvider: Send + Sync + std::fmt::Debug {
    /// Get current time in milliseconds since epoch
    fn now_ms(&self) -> u64;

    /// Sleep for the specified duration
    ///
    /// In production: actual tokio::time::sleep
    /// In DST: advances simulated time, returns immediately
    async fn sleep_ms(&self, ms: u64);

    /// Get monotonic timestamp (for measuring durations)
    fn monotonic_ms(&self) -> u64 {
        self.now_ms()
    }
}

/// Production time provider using wall clock
#[derive(Debug, Clone, Default)]
pub struct WallClockTime;

impl WallClockTime {
    /// Create a new wall clock time provider
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl TimeProvider for WallClockTime {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    async fn sleep_ms(&self, ms: u64) {
        crate::current_runtime()
            .sleep(std::time::Duration::from_millis(ms))
            .await;
    }
}

// ============================================================================
// RNG Provider
// ============================================================================

/// Random number generator abstraction for DST
///
/// All code that needs randomness MUST use this trait.
/// Never use `rand::thread_rng()` or `uuid::Uuid::new_v4()` directly.
///
/// # Implementations
///
/// - `StdRngProvider`: Production - uses thread-local RNG
/// - `DeterministicRng` (in kelpie-dst): DST - seeded, reproducible
pub trait RngProvider: Send + Sync + std::fmt::Debug {
    /// Generate a random u64
    fn next_u64(&self) -> u64;

    /// Generate a random f64 in [0, 1)
    fn next_f64(&self) -> f64 {
        // Default implementation using next_u64
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Generate a random UUID string
    fn gen_uuid(&self) -> String {
        // Generate 128 bits of randomness
        let high = self.next_u64();
        let low = self.next_u64();

        // Format as UUID v4
        let bytes = [
            ((high >> 56) & 0xff) as u8,
            ((high >> 48) & 0xff) as u8,
            ((high >> 40) & 0xff) as u8,
            ((high >> 32) & 0xff) as u8,
            ((high >> 24) & 0xff) as u8,
            ((high >> 16) & 0xff) as u8,
            (((high >> 8) & 0x0f) | 0x40) as u8, // Version 4
            (high & 0xff) as u8,
            (((low >> 56) & 0x3f) | 0x80) as u8, // Variant 1
            ((low >> 48) & 0xff) as u8,
            ((low >> 40) & 0xff) as u8,
            ((low >> 32) & 0xff) as u8,
            ((low >> 24) & 0xff) as u8,
            ((low >> 16) & 0xff) as u8,
            ((low >> 8) & 0xff) as u8,
            (low & 0xff) as u8,
        ];

        format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3],
            bytes[4], bytes[5],
            bytes[6], bytes[7],
            bytes[8], bytes[9],
            bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
        )
    }

    /// Generate random boolean with given probability of true
    fn gen_bool(&self, probability: f64) -> bool {
        assert!(
            (0.0..=1.0).contains(&probability),
            "probability must be in [0, 1]"
        );
        self.next_f64() < probability
    }

    /// Generate random u64 in range [min, max)
    fn gen_range(&self, min: u64, max: u64) -> u64 {
        assert!(min < max, "min must be less than max");
        let range = max - min;
        min + (self.next_u64() % range)
    }
}

/// Production RNG provider using thread-local RNG
///
/// Uses atomic counter for thread-safety without locks.
/// Not cryptographically secure - use for non-security randomness only.
#[derive(Debug)]
pub struct StdRngProvider {
    /// Simple counter-based state for thread-safe randomness
    state: AtomicU64,
}

impl Default for StdRngProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl StdRngProvider {
    /// Create a new RNG provider seeded from system time
    pub fn new() -> Self {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);

        Self {
            state: AtomicU64::new(seed),
        }
    }

    /// Create with specific seed (for testing)
    pub fn with_seed(seed: u64) -> Self {
        Self {
            state: AtomicU64::new(seed),
        }
    }
}

impl RngProvider for StdRngProvider {
    fn next_u64(&self) -> u64 {
        // Simple xorshift64* algorithm
        let mut state = self.state.load(Ordering::Relaxed);
        loop {
            let mut x = state;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            let new_state = x;

            match self.state.compare_exchange_weak(
                state,
                new_state,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return x.wrapping_mul(0x2545F4914F6CDD1D),
                Err(s) => state = s,
            }
        }
    }
}

// ============================================================================
// I/O Context
// ============================================================================

/// Bundle of all I/O providers
///
/// Pass this through your application instead of individual providers.
/// Makes it easy to swap between production and DST modes.
#[derive(Clone)]
pub struct IoContext {
    /// Time provider
    pub time: Arc<dyn TimeProvider>,
    /// RNG provider
    pub rng: Arc<dyn RngProvider>,
}

impl std::fmt::Debug for IoContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IoContext")
            .field("time", &self.time)
            .field("rng", &format!("{:?}", self.rng))
            .finish()
    }
}

impl Default for IoContext {
    fn default() -> Self {
        Self::production()
    }
}

impl IoContext {
    /// Create production I/O context with real wall clock and RNG
    pub fn production() -> Self {
        Self {
            time: Arc::new(WallClockTime::new()),
            rng: Arc::new(StdRngProvider::new()),
        }
    }

    /// Create I/O context with custom providers
    pub fn new(time: Arc<dyn TimeProvider>, rng: Arc<dyn RngProvider>) -> Self {
        Self { time, rng }
    }

    /// Get current time in milliseconds
    pub fn now_ms(&self) -> u64 {
        self.time.now_ms()
    }

    /// Sleep for specified duration
    pub async fn sleep_ms(&self, ms: u64) {
        self.time.sleep_ms(ms).await;
    }

    /// Generate a UUID
    pub fn gen_uuid(&self) -> String {
        self.rng.gen_uuid()
    }

    /// Generate random boolean with probability
    pub fn gen_bool(&self, probability: f64) -> bool {
        self.rng.gen_bool(probability)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wall_clock_time_now_ms() {
        let clock = WallClockTime::new();
        let now = clock.now_ms();

        // Should be a reasonable timestamp (after 2020)
        assert!(now > 1577836800000); // Jan 1, 2020

        // Two calls should be close together
        let now2 = clock.now_ms();
        assert!(now2 >= now);
        assert!(now2 - now < 1000); // Within 1 second
    }

    #[tokio::test]
    async fn test_wall_clock_time_sleep() {
        let clock = WallClockTime::new();
        let start = clock.now_ms();

        clock.sleep_ms(10).await;

        let elapsed = clock.now_ms() - start;
        // Should have slept at least 10ms (allow some tolerance)
        assert!(elapsed >= 9, "elapsed: {}", elapsed);
    }

    #[test]
    fn test_std_rng_provider_deterministic_with_seed() {
        let rng1 = StdRngProvider::with_seed(12345);
        let rng2 = StdRngProvider::with_seed(12345);

        // Same seed should produce same sequence
        // Note: Due to atomic operations, we test single values
        let v1 = rng1.next_u64();
        let v2 = rng2.next_u64();

        // With fresh RNGs and same seed, first value should match
        assert_eq!(v1, v2);
    }

    #[test]
    fn test_std_rng_provider_gen_uuid() {
        let rng = StdRngProvider::with_seed(42);
        let uuid = rng.gen_uuid();

        // Should be valid UUID format
        assert_eq!(uuid.len(), 36);
        assert_eq!(&uuid[8..9], "-");
        assert_eq!(&uuid[13..14], "-");
        assert_eq!(&uuid[18..19], "-");
        assert_eq!(&uuid[23..24], "-");

        // Version should be 4
        assert!(uuid.chars().nth(14).unwrap() == '4');
    }

    #[test]
    fn test_std_rng_provider_gen_bool() {
        let rng = StdRngProvider::with_seed(42);

        // 0% probability should always be false
        for _ in 0..10 {
            assert!(!rng.gen_bool(0.0));
        }

        // 100% probability should always be true
        for _ in 0..10 {
            assert!(rng.gen_bool(1.0));
        }
    }

    #[test]
    fn test_std_rng_provider_gen_range() {
        let rng = StdRngProvider::with_seed(42);

        for _ in 0..100 {
            let value = rng.gen_range(10, 20);
            assert!(value >= 10);
            assert!(value < 20);
        }
    }

    #[test]
    fn test_io_context_production() {
        let ctx = IoContext::production();

        // Should have valid time
        let now = ctx.now_ms();
        assert!(now > 1577836800000);

        // Should generate valid UUID
        let uuid = ctx.gen_uuid();
        assert_eq!(uuid.len(), 36);
    }
}
