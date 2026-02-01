//! Runtime Abstraction for Deterministic Testing
//!
//! TigerStyle: Explicit runtime abstraction for swapping tokio (prod) and madsim (test).
//!
//! This module provides a `Runtime` trait that abstracts async runtime operations:
//! - Task spawning (spawn)
//! - Time operations (sleep, now, elapsed)
//! - Task yielding (yield_now)
//!
//! ## Architecture
//!
//! ```text
//! Production:          Testing:
//! TokioRuntime -----> Runtime <----- MadsimRuntime
//!     (wall clock)    (trait)        (virtual time)
//! ```
//!
//! ## Usage
//!
//! ```rust,no_run
//! use kelpie_core::runtime::{Runtime, current_runtime};
//!
//! async fn my_function() {
//!     let runtime = current_runtime();
//!
//!     // Sleep for 100ms (real or virtual depending on runtime)
//!     runtime.sleep(std::time::Duration::from_millis(100)).await;
//!
//!     // Spawn a task
//!     let handle = runtime.spawn(async {
//!         println!("Hello from spawned task");
//!     });
//!
//!     handle.await.unwrap();
//! }
//! ```

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// JoinHandle for spawned tasks
///
/// This abstracts over tokio::task::JoinHandle and madsim::task::JoinHandle
pub type JoinHandle<T> = Pin<Box<dyn Future<Output = Result<T, JoinError>> + Send>>;

/// Error from joining a task
#[derive(Debug, thiserror::Error)]
pub enum JoinError {
    #[error("task panicked")]
    Panicked,
    #[error("task cancelled")]
    Cancelled,
}

/// Instant in time (real or virtual)
///
/// TigerStyle: Explicit time representation for deterministic testing
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant {
    /// Milliseconds since epoch (or virtual epoch)
    pub millis: u64,
}

impl Instant {
    /// Create a new instant from milliseconds
    pub fn from_millis(millis: u64) -> Self {
        Self { millis }
    }

    /// Get duration elapsed since this instant
    pub fn elapsed(&self, now: Instant) -> Duration {
        assert!(now.millis >= self.millis, "now must be >= self for elapsed");
        Duration::from_millis(now.millis - self.millis)
    }
}

/// Runtime abstraction trait
///
/// TigerStyle: Explicit trait for runtime operations with clear contracts.
///
/// Implementations:
/// - `TokioRuntime`: Production runtime using wall-clock time
/// - `MadsimRuntime`: Test runtime using virtual time
///
/// Note: This trait is NOT dyn-safe due to spawn's generic parameter.
/// Use concrete types (TokioRuntime, MadsimRuntime) or current_runtime() factory.
#[async_trait::async_trait]
pub trait Runtime: Send + Sync + Clone {
    /// Get current instant (real or virtual time)
    fn now(&self) -> Instant;

    /// Sleep for a duration
    ///
    /// Preconditions:
    /// - duration must be < 1 hour (safety limit)
    ///
    /// Postconditions:
    /// - Task resumes after duration has elapsed
    /// - For tokio: duration of real wall-clock time
    /// - For madsim: duration of virtual time (instant in real time)
    async fn sleep(&self, duration: Duration);

    /// Yield control to the scheduler
    ///
    /// This allows other tasks to run. In deterministic runtimes (madsim),
    /// this is critical for ensuring tasks make progress.
    async fn yield_now(&self);

    /// Spawn a new task
    ///
    /// The task runs concurrently with the current task.
    ///
    /// Preconditions:
    /// - Future must be Send + 'static
    ///
    /// Returns:
    /// - JoinHandle that can be awaited for task completion
    fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static;

    /// Run a future with a timeout
    ///
    /// Preconditions:
    /// - duration must be < 1 hour (safety limit)
    ///
    /// Returns:
    /// - Ok(T) if the future completed within the timeout
    /// - Err(()) if the timeout elapsed before completion
    async fn timeout<F>(&self, duration: Duration, future: F) -> Result<F::Output, ()>
    where
        F: Future + Send,
        F::Output: Send;
}

// Note: We cannot have a single current_runtime() that returns a trait object
// because Runtime is not dyn-safe. Instead, use conditional compilation directly:
//
// #[cfg(madsim)]
// use kelpie_core::runtime::MadsimRuntime as CurrentRuntime;
//
// #[cfg(not(madsim))]
// use kelpie_core::runtime::TokioRuntime as CurrentRuntime;

// =============================================================================
// TokioRuntime (Production)
// =============================================================================

/// Production runtime using tokio
///
/// TigerStyle: Thin wrapper over tokio with explicit contracts.
#[derive(Debug, Clone)]
pub struct TokioRuntime;

// Allow raw tokio calls in TokioRuntime implementation (this is the abstraction layer)
#[allow(clippy::disallowed_methods)]
#[async_trait::async_trait]
impl Runtime for TokioRuntime {
    fn now(&self) -> Instant {
        let std_instant = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before UNIX epoch");
        Instant::from_millis(std_instant.as_millis() as u64)
    }

    async fn sleep(&self, duration: Duration) {
        assert!(
            duration < Duration::from_secs(3600),
            "sleep duration too long (>1 hour)"
        );
        tokio::time::sleep(duration).await;
    }

    async fn yield_now(&self) {
        tokio::task::yield_now().await;
    }

    fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let handle = tokio::spawn(future);
        Box::pin(async move {
            handle.await.map_err(|e| {
                if e.is_panic() {
                    JoinError::Panicked
                } else {
                    JoinError::Cancelled
                }
            })
        })
    }

    async fn timeout<F>(&self, duration: Duration, future: F) -> Result<F::Output, ()>
    where
        F: Future + Send,
        F::Output: Send,
    {
        assert!(
            duration < Duration::from_secs(3600),
            "timeout duration too long (>1 hour)"
        );
        tokio::time::timeout(duration, future).await.map_err(|_| ())
    }
}

// =============================================================================
// MadsimRuntime (DST Testing)
// =============================================================================

/// Test runtime using madsim deterministic executor
///
/// TigerStyle: Virtual time for deterministic testing.
#[cfg(madsim)]
#[derive(Debug, Clone)]
pub struct MadsimRuntime;

#[cfg(madsim)]
#[async_trait::async_trait]
impl Runtime for MadsimRuntime {
    fn now(&self) -> Instant {
        // Get current virtual time as duration since simulation start
        // madsim::time::Instant represents time since simulation epoch
        let duration_since_epoch =
            madsim::time::Instant::now().duration_since(madsim::time::Instant::from_nanos(0));
        Instant::from_millis(duration_since_epoch.as_millis() as u64)
    }

    async fn sleep(&self, duration: Duration) {
        assert!(
            duration < Duration::from_secs(3600),
            "sleep duration too long (>1 hour)"
        );
        madsim::time::sleep(duration).await;
    }

    async fn yield_now(&self) {
        // madsim doesn't have yield_now, but sleep(0) has similar effect
        madsim::time::sleep(Duration::from_millis(0)).await;
    }

    fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let handle = madsim::task::spawn(future);
        Box::pin(async move { handle.await.map_err(|_| JoinError::Panicked) })
    }

    async fn timeout<F>(&self, duration: Duration, future: F) -> Result<F::Output, ()>
    where
        F: Future + Send,
        F::Output: Send,
    {
        assert!(
            duration < Duration::from_secs(3600),
            "timeout duration too long (>1 hour)"
        );
        madsim::time::timeout(duration, future)
            .await
            .map_err(|_| ())
    }
}

// =============================================================================
// Runtime Factory
// =============================================================================

/// Type alias for the current runtime
///
/// Resolves to MadsimRuntime when madsim feature is enabled (deterministic testing),
/// TokioRuntime otherwise (production).
///
/// Use this for generic type parameters like `AgentService<CurrentRuntime>`.
#[cfg(madsim)]
pub type CurrentRuntime = MadsimRuntime;

/// Type alias for the current runtime (TokioRuntime variant)
#[cfg(not(madsim))]
pub type CurrentRuntime = TokioRuntime;

/// Get the current runtime instance
///
/// Returns MadsimRuntime when madsim feature is enabled (for deterministic testing),
/// TokioRuntime otherwise (for production).
///
/// # Example
///
/// ```rust,no_run
/// use kelpie_core::runtime::{Runtime, current_runtime};
///
/// async fn my_function() {
///     let runtime = current_runtime();
///     runtime.sleep(std::time::Duration::from_millis(100)).await;
/// }
/// ```
///
/// # Testing
///
/// Run tests with madsim enabled for deterministic behavior:
/// ```bash
/// cargo test --features madsim
/// ```
#[cfg(madsim)]
pub fn current_runtime() -> MadsimRuntime {
    MadsimRuntime
}

/// Get the current runtime instance (TokioRuntime variant)
#[cfg(not(madsim))]
pub fn current_runtime() -> TokioRuntime {
    TokioRuntime
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tokio_runtime_sleep() {
        let runtime = TokioRuntime;
        let start = runtime.now();

        runtime.sleep(Duration::from_millis(10)).await;

        let elapsed = start.elapsed(runtime.now());
        assert!(
            elapsed >= Duration::from_millis(10),
            "Should sleep for at least 10ms"
        );
    }

    #[tokio::test]
    async fn test_tokio_runtime_spawn() {
        let runtime = TokioRuntime;

        let handle = runtime.spawn(async { 42 });

        let result = handle.await.unwrap();
        assert_eq!(result, 42);
    }
}
