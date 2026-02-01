//! Time abstraction for deterministic testing
//!
//! TigerStyle: Explicit time control, trait-based abstraction.
//!
//! Provides TimeProvider trait with two implementations:
//! - SimTime: Uses SimClock, advances time instantly (no real delays)
//! - RealTime: Uses tokio::time, for production and non-DST tests
//!
//! ## Deterministic Scheduling (Issue #15)
//!
//! With madsim as the default runtime for kelpie-dst, SimTime now yields
//! to madsim's deterministic scheduler instead of tokio's non-deterministic one.
//! This ensures same seed = same task interleaving order.

// Allow tokio usage in DST framework code (this IS the abstraction layer)
#![allow(clippy::disallowed_methods)]
//!
//! ## Why This Exists
//!
//! DST tests were using `tokio::time::sleep()` which:
//! - Uses wall-clock time (non-deterministic)
//! - Ignores SimClock (time doesn't advance in simulation)
//! - Makes tests slow (real delays add up)
//! - Causes flaky tests (race conditions)
//!
//! ## Solution
//!
//! Replace `tokio::time::sleep(dur)` with `time_provider.sleep(dur)`:
//! - In DST: Advances SimClock + yields to madsim (instant, deterministic)
//! - In production: Uses tokio::time (real delays)

use async_trait::async_trait;
use kelpie_core::TimeProvider;
use std::sync::Arc;
use std::time::Duration;

use crate::clock::SimClock;

/// Simulated time provider for DST
///
/// Advances SimClock instantly and yields to tokio scheduler.
/// No real delays, fully deterministic.
#[derive(Clone, Debug)]
pub struct SimTime {
    /// Reference to simulation clock
    clock: Arc<SimClock>,
}

impl SimTime {
    /// Create a new SimTime from a SimClock
    pub fn new(clock: Arc<SimClock>) -> Self {
        Self { clock }
    }

    /// Get the underlying SimClock
    pub fn clock(&self) -> &SimClock {
        &self.clock
    }
}

#[async_trait]
impl TimeProvider for SimTime {
    fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    async fn sleep_ms(&self, ms: u64) {
        // Advance SimClock by the requested duration
        self.clock.advance_ms(ms);

        // Yield to scheduler to allow other tasks to run
        // This is critical: without yielding, we'd have busy loops
        //
        // IMPORTANT (Issue #15): Use madsim's yield when madsim feature is enabled.
        // madsim provides deterministic scheduling, ensuring same seed = same order.
        #[cfg(madsim)]
        {
            // madsim doesn't have yield_now(), but sleep(0) has the same effect
            madsim::time::sleep(Duration::from_millis(0)).await;
        }

        #[cfg(not(madsim))]
        {
            tokio::task::yield_now().await;
        }

        // Postcondition: time has advanced
        debug_assert!(self.clock.now_ms() > 0 || ms == 0);
    }
}

/// Real time provider for production
///
/// Uses tokio::time::sleep for actual delays.
/// Used in production and non-DST tests.
#[derive(Debug, Clone)]
pub struct RealTime;

impl RealTime {
    /// Create a new RealTime provider
    pub fn new() -> Self {
        Self
    }
}

impl Default for RealTime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TimeProvider for RealTime {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    async fn sleep_ms(&self, ms: u64) {
        // Precondition: duration should be reasonable
        assert!(ms <= 3600 * 1000, "sleep duration too long (>1 hour)");

        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // SimTime tests (use madsim for deterministic scheduling)
    // =========================================================================

    #[madsim::test]
    async fn test_sim_time_advances_clock() {
        let clock = Arc::new(SimClock::from_millis(1000));
        let time = SimTime::new(clock.clone());

        let before = time.now_ms();
        time.sleep_ms(500).await;
        let after = time.now_ms();

        assert_eq!(before, 1000);
        assert_eq!(after, 1500);
        assert_eq!(after - before, 500);
    }

    #[madsim::test]
    async fn test_sim_time_multiple_sleeps() {
        let clock = Arc::new(SimClock::from_millis(0));
        let time = SimTime::new(clock.clone());

        time.sleep_ms(100).await;
        assert_eq!(time.now_ms(), 100);

        time.sleep_ms(250).await;
        assert_eq!(time.now_ms(), 350);

        time.sleep_ms(50).await;
        assert_eq!(time.now_ms(), 400);
    }

    #[madsim::test]
    async fn test_sim_time_zero_duration() {
        let clock = Arc::new(SimClock::from_millis(1000));
        let time = SimTime::new(clock.clone());

        let before = time.now_ms();
        time.sleep_ms(0).await;
        let after = time.now_ms();

        // Zero sleep should not advance time but should yield
        assert_eq!(before, after);
    }

    #[madsim::test]
    async fn test_sim_time_yields_to_scheduler() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let clock = Arc::new(SimClock::from_millis(0));
        let time = SimTime::new(clock.clone());

        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = flag.clone();

        // Spawn a task that sets the flag (using madsim for determinism)
        madsim::task::spawn(async move {
            flag_clone.store(true, Ordering::SeqCst);
        });

        // Sleep should yield, allowing the spawned task to run
        time.sleep_ms(1).await;

        // Flag should be set (spawned task ran)
        assert!(flag.load(Ordering::SeqCst));
    }

    #[madsim::test]
    async fn test_sim_time_concurrent_sleeps() {
        let clock = Arc::new(SimClock::from_millis(0));
        let time1 = SimTime::new(clock.clone());
        let time2 = SimTime::new(clock.clone());

        // Spawn two concurrent tasks that sleep (using madsim for determinism)
        let handle1 = madsim::task::spawn({
            let time = time1.clone();
            async move {
                time.sleep_ms(100).await;
                time.now_ms()
            }
        });

        let handle2 = madsim::task::spawn({
            let time = time2.clone();
            async move {
                time.sleep_ms(50).await;
                time.now_ms()
            }
        });

        let result1 = handle1.await.unwrap();
        let result2 = handle2.await.unwrap();

        // Both should have advanced the shared clock
        // Final clock should be sum of both sleeps
        let final_time = time1.now_ms();
        assert_eq!(final_time, 150); // 100 + 50

        // With madsim, task ordering is deterministic
        // But the results depend on which task runs first
        assert!((50..=150).contains(&result1));
        assert!((50..=150).contains(&result2));
    }

    // =========================================================================
    // RealTime tests (use tokio - these test actual wall-clock behavior)
    //
    // Note: RealTime is for production use, not DST. These tests verify
    // that RealTime actually sleeps using real wall-clock time.
    // =========================================================================

    /// Test RealTime actually sleeps (wall-clock)
    ///
    /// This test is ignored by default because it uses real wall-clock time
    /// and doesn't benefit from DST's deterministic scheduling.
    #[tokio::test]
    #[ignore = "Uses real wall-clock time, not suitable for DST"]
    async fn test_real_time_actually_sleeps() {
        let time = RealTime::new();

        let start = std::time::Instant::now();
        time.sleep_ms(50).await;
        let elapsed = start.elapsed();

        // Should have actually slept (within 20ms tolerance)
        assert!(elapsed.as_millis() >= 45);
        assert!(elapsed.as_millis() <= 70);
    }

    /// Test RealTime now_ms returns reasonable values
    ///
    /// This test is ignored by default because it uses real wall-clock time.
    #[tokio::test]
    #[ignore = "Uses real wall-clock time, not suitable for DST"]
    async fn test_real_time_now_ms() {
        let time = RealTime::new();

        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let now = time.now_ms();

        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // now_ms should be between before and after (within 100ms)
        assert!(now >= before);
        assert!(now <= after + 100);
    }
}
