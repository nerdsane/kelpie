//! Deterministic clock for simulation
//!
//! TigerStyle: Explicit time control, no system time dependencies.
//!
//! SimClock implements the `TimeProvider` trait from kelpie-core, enabling
//! the same business logic code to run with either wall clock (production)
//! or simulated clock (DST).

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use kelpie_core::TimeProvider;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

/// Deterministic simulation clock
///
/// Provides controlled time progression for deterministic testing.
/// Time only advances when explicitly told to, enabling reproducible tests.
#[derive(Debug, Clone)]
pub struct SimClock {
    /// Current time in milliseconds since epoch
    current_time_ms: Arc<AtomicU64>,
    /// Notify waiters when time advances
    notify: Arc<Notify>,
}

impl SimClock {
    /// Create a new SimClock starting at the given time
    pub fn new(start_time: DateTime<Utc>) -> Self {
        let ms = start_time.timestamp_millis() as u64;
        Self {
            current_time_ms: Arc::new(AtomicU64::new(ms)),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Create a new SimClock starting at Unix epoch
    pub fn from_epoch() -> Self {
        Self::new(DateTime::from_timestamp(0, 0).unwrap())
    }

    /// Create a new SimClock starting at a specific millisecond timestamp
    pub fn from_millis(ms: u64) -> Self {
        Self {
            current_time_ms: Arc::new(AtomicU64::new(ms)),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Get the current time
    pub fn now(&self) -> DateTime<Utc> {
        let ms = self.current_time_ms.load(Ordering::SeqCst);
        DateTime::from_timestamp_millis(ms as i64).unwrap_or_else(|| {
            // Fallback for invalid timestamps
            DateTime::from_timestamp(0, 0).unwrap()
        })
    }

    /// Get the current time in milliseconds since epoch
    pub fn now_ms(&self) -> u64 {
        self.current_time_ms.load(Ordering::SeqCst)
    }

    /// Advance time by the given duration
    pub fn advance(&self, duration: Duration) {
        debug_assert!(duration >= Duration::zero(), "cannot go back in time");

        let delta_ms = duration.num_milliseconds() as u64;
        self.current_time_ms.fetch_add(delta_ms, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Advance time by the given number of milliseconds
    pub fn advance_ms(&self, ms: u64) {
        self.current_time_ms.fetch_add(ms, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Set the current time (use with caution)
    pub fn set(&self, time: DateTime<Utc>) {
        let ms = time.timestamp_millis() as u64;
        self.current_time_ms.store(ms, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Sleep until the specified duration has passed
    ///
    /// In simulation mode, this yields and waits for time to be advanced.
    pub async fn sleep(&self, duration: Duration) {
        let target_ms = self.now_ms() + duration.num_milliseconds() as u64;

        while self.now_ms() < target_ms {
            self.notify.notified().await;
        }
    }

    /// Sleep for the specified number of milliseconds
    pub async fn sleep_ms(&self, ms: u64) {
        let target_ms = self.now_ms() + ms;

        while self.now_ms() < target_ms {
            self.notify.notified().await;
        }
    }

    /// Check if a deadline has passed
    pub fn is_past(&self, deadline: DateTime<Utc>) -> bool {
        self.now() >= deadline
    }

    /// Check if a deadline (in ms) has passed
    pub fn is_past_ms(&self, deadline_ms: u64) -> bool {
        self.now_ms() >= deadline_ms
    }
}

impl Default for SimClock {
    fn default() -> Self {
        // Default to 2024-01-01 00:00:00 UTC for predictable test behavior
        Self::new(
            DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                .unwrap()
                .to_utc(),
        )
    }
}

// ============================================================================
// TimeProvider Implementation
// ============================================================================

/// SimClock implements TimeProvider for DST compatibility
///
/// This allows the same business logic code to use either:
/// - `WallClockTime` (production)
/// - `SimClock` (DST)
#[async_trait]
impl TimeProvider for SimClock {
    fn now_ms(&self) -> u64 {
        self.current_time_ms.load(Ordering::SeqCst)
    }

    async fn sleep_ms(&self, ms: u64) {
        let target_ms = self.now_ms() + ms;

        while self.now_ms() < target_ms {
            self.notify.notified().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clock_basic() {
        let clock = SimClock::default();
        let initial = clock.now();

        clock.advance(Duration::seconds(10));

        let after = clock.now();
        assert_eq!(after - initial, Duration::seconds(10));
    }

    #[test]
    fn test_clock_advance_ms() {
        let clock = SimClock::from_millis(0);
        assert_eq!(clock.now_ms(), 0);

        clock.advance_ms(1000);
        assert_eq!(clock.now_ms(), 1000);

        clock.advance_ms(500);
        assert_eq!(clock.now_ms(), 1500);
    }

    #[test]
    fn test_clock_is_past() {
        let clock = SimClock::from_millis(1000);

        assert!(clock.is_past_ms(500));
        assert!(clock.is_past_ms(1000));
        assert!(!clock.is_past_ms(1500));
    }

    /// Test clock sleep with madsim runtime
    ///
    /// Note: This test uses madsim for deterministic task scheduling.
    /// The SimClock's sleep_ms waits for manual time advancement via notify.
    #[madsim::test]
    #[allow(clippy::disallowed_methods)] // madsim intercepts tokio at compile time
    async fn test_clock_sleep() {
        let clock = SimClock::from_millis(0);
        let clock_clone = clock.clone();

        // Spawn a task that sleeps using SimClock
        let handle = madsim::task::spawn(async move {
            clock_clone.sleep_ms(100).await;
            clock_clone.now_ms()
        });

        // Advance time in steps
        // Use madsim's sleep(0) for yielding to allow other tasks to run
        madsim::time::sleep(std::time::Duration::from_millis(0)).await;
        clock.advance_ms(50);
        madsim::time::sleep(std::time::Duration::from_millis(0)).await;
        clock.advance_ms(50);
        madsim::time::sleep(std::time::Duration::from_millis(0)).await;

        let result = handle.await.unwrap();
        assert!(result >= 100);
    }
}
