//! Sandbox pool for pre-warming and fast allocation
//!
//! TigerStyle: Explicit pool sizing with backpressure.

use crate::config::SandboxConfig;
use crate::error::{SandboxError, SandboxResult};
use crate::traits::{Sandbox, SandboxFactory, SandboxState};
use kelpie_core::Runtime;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore};

/// Default minimum pool size
pub const POOL_SIZE_MIN_DEFAULT: usize = 2;

/// Default maximum pool size
pub const POOL_SIZE_MAX_DEFAULT: usize = 10;

/// Default acquire timeout (5 seconds)
pub const POOL_ACQUIRE_TIMEOUT_MS_DEFAULT: u64 = 5_000;

/// Pool configuration
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Minimum number of warm sandboxes to maintain
    pub size_min: usize,
    /// Maximum number of sandboxes (warm + in-use)
    pub size_max: usize,
    /// Configuration for new sandboxes
    pub sandbox_config: SandboxConfig,
    /// Timeout for acquiring a sandbox
    pub acquire_timeout_ms: u64,
    /// How often to check pool health
    pub health_check_interval_ms: u64,
}

impl PoolConfig {
    /// Create a new pool configuration
    pub fn new(sandbox_config: SandboxConfig) -> Self {
        Self {
            size_min: POOL_SIZE_MIN_DEFAULT,
            size_max: POOL_SIZE_MAX_DEFAULT,
            sandbox_config,
            acquire_timeout_ms: POOL_ACQUIRE_TIMEOUT_MS_DEFAULT,
            health_check_interval_ms: 30_000,
        }
    }

    /// Set minimum pool size
    pub fn with_min_size(mut self, size: usize) -> Self {
        self.size_min = size;
        self
    }

    /// Set maximum pool size
    pub fn with_max_size(mut self, size: usize) -> Self {
        self.size_max = size;
        self
    }

    /// Set acquire timeout
    pub fn with_acquire_timeout(mut self, timeout: Duration) -> Self {
        self.acquire_timeout_ms = timeout.as_millis() as u64;
        self
    }

    /// Validate configuration
    pub fn validate(&self) -> SandboxResult<()> {
        if self.size_min > self.size_max {
            return Err(SandboxError::ConfigError {
                reason: format!(
                    "size_min ({}) cannot exceed size_max ({})",
                    self.size_min, self.size_max
                ),
            });
        }
        if self.size_max == 0 {
            return Err(SandboxError::ConfigError {
                reason: "size_max must be greater than 0".to_string(),
            });
        }
        Ok(())
    }
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self::new(SandboxConfig::default())
    }
}

/// Pool statistics
#[derive(Debug, Clone, Default)]
pub struct PoolStats {
    /// Number of warm sandboxes ready for use
    pub warm_count: usize,
    /// Number of sandboxes currently in use
    pub in_use_count: usize,
    /// Total sandboxes created
    pub total_created: u64,
    /// Total sandboxes acquired
    pub total_acquired: u64,
    /// Total sandboxes returned
    pub total_returned: u64,
    /// Total sandboxes destroyed (due to errors or limits)
    pub total_destroyed: u64,
    /// Acquire timeouts
    pub acquire_timeouts: u64,
}

/// A pool of pre-warmed sandboxes
///
/// Maintains a pool of ready-to-use sandboxes for fast allocation.
/// Automatically creates new sandboxes when the pool is depleted
/// and destroys excess sandboxes when returned.
pub struct SandboxPool<F: SandboxFactory> {
    /// Factory for creating new sandboxes
    factory: Arc<F>,
    /// Pool configuration
    config: PoolConfig,
    /// Warm (ready) sandboxes
    warm: Mutex<VecDeque<F::Sandbox>>,
    /// Semaphore to limit total sandboxes
    capacity: Semaphore,
    /// Statistics
    stats: PoolStatsInner,
}

/// Internal statistics with atomic counters
struct PoolStatsInner {
    total_created: AtomicU64,
    total_acquired: AtomicU64,
    total_returned: AtomicU64,
    total_destroyed: AtomicU64,
    acquire_timeouts: AtomicU64,
}

impl Default for PoolStatsInner {
    fn default() -> Self {
        Self {
            total_created: AtomicU64::new(0),
            total_acquired: AtomicU64::new(0),
            total_returned: AtomicU64::new(0),
            total_destroyed: AtomicU64::new(0),
            acquire_timeouts: AtomicU64::new(0),
        }
    }
}

impl<F: SandboxFactory> SandboxPool<F> {
    /// Create a new sandbox pool
    pub fn new(factory: F, config: PoolConfig) -> SandboxResult<Self> {
        config.validate()?;

        Ok(Self {
            factory: Arc::new(factory),
            capacity: Semaphore::new(config.size_max),
            config,
            warm: Mutex::new(VecDeque::new()),
            stats: PoolStatsInner::default(),
        })
    }

    /// Initialize the pool with pre-warmed sandboxes
    pub async fn warm_up(&self) -> SandboxResult<()> {
        let mut warm = self.warm.lock().await;
        let current = warm.len();

        for _ in current..self.config.size_min {
            // Acquire a permit
            let permit = self.capacity.try_acquire();
            if permit.is_err() {
                break; // At capacity
            }
            // Don't drop the permit - we're adding to the pool
            permit.unwrap().forget();

            match self.create_sandbox().await {
                Ok(sandbox) => {
                    warm.push_back(sandbox);
                }
                Err(e) => {
                    // Release the permit we acquired
                    self.capacity.add_permits(1);
                    tracing::warn!("Failed to warm up sandbox: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Acquire a sandbox from the pool
    ///
    /// Returns a warm sandbox if available, otherwise creates a new one.
    /// Blocks if pool is at capacity until a sandbox is returned.
    pub async fn acquire(&self) -> SandboxResult<F::Sandbox> {
        let timeout_duration = Duration::from_millis(self.config.acquire_timeout_ms);

        // Try to get a warm sandbox first
        {
            let mut warm = self.warm.lock().await;
            if let Some(sandbox) = warm.pop_front() {
                self.stats.total_acquired.fetch_add(1, Ordering::Relaxed);
                return Ok(sandbox);
            }
        }

        // No warm sandbox, try to create a new one
        let permit = match kelpie_core::current_runtime()
            .timeout(timeout_duration, self.capacity.acquire())
            .await
        {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => {
                // Semaphore closed - shouldn't happen
                return Err(SandboxError::Internal {
                    message: "Pool semaphore closed".to_string(),
                });
            }
            Err(_) => {
                // Timeout
                self.stats.acquire_timeouts.fetch_add(1, Ordering::Relaxed);
                return Err(SandboxError::PoolAcquireTimeout {
                    timeout_ms: self.config.acquire_timeout_ms,
                });
            }
        };

        // Double-check for warm sandbox (someone might have returned one)
        {
            let mut warm = self.warm.lock().await;
            if let Some(sandbox) = warm.pop_front() {
                // Release the permit we acquired
                drop(permit);
                self.stats.total_acquired.fetch_add(1, Ordering::Relaxed);
                return Ok(sandbox);
            }
        }

        // Create a new sandbox
        permit.forget(); // We're using this slot
        match self.create_sandbox().await {
            Ok(sandbox) => {
                self.stats.total_acquired.fetch_add(1, Ordering::Relaxed);
                Ok(sandbox)
            }
            Err(e) => {
                // Release the slot
                self.capacity.add_permits(1);
                Err(e)
            }
        }
    }

    /// Return a sandbox to the pool
    ///
    /// If the sandbox is healthy and pool isn't at warm capacity,
    /// keeps it for reuse. Otherwise destroys it.
    pub async fn release(&self, mut sandbox: F::Sandbox) {
        self.stats.total_returned.fetch_add(1, Ordering::Relaxed);

        // Check if sandbox is still usable
        let is_healthy = sandbox.state() == SandboxState::Running
            && sandbox.health_check().await.unwrap_or(false);

        if !is_healthy {
            // Sandbox is not healthy, destroy it
            let _ = sandbox.destroy().await;
            self.stats.total_destroyed.fetch_add(1, Ordering::Relaxed);
            self.capacity.add_permits(1);
            return;
        }

        // Check if we should keep it warm
        let mut warm = self.warm.lock().await;
        if warm.len() < self.config.size_min {
            warm.push_back(sandbox);
        } else {
            // Pool is at warm capacity, destroy excess
            drop(warm);
            let _ = sandbox.destroy().await;
            self.stats.total_destroyed.fetch_add(1, Ordering::Relaxed);
            self.capacity.add_permits(1);
        }
    }

    /// Get pool statistics
    pub async fn stats(&self) -> PoolStats {
        let warm = self.warm.lock().await;
        let warm_count = warm.len();
        let in_use = self.config.size_max - self.capacity.available_permits() - warm_count;

        PoolStats {
            warm_count,
            in_use_count: in_use,
            total_created: self.stats.total_created.load(Ordering::Relaxed),
            total_acquired: self.stats.total_acquired.load(Ordering::Relaxed),
            total_returned: self.stats.total_returned.load(Ordering::Relaxed),
            total_destroyed: self.stats.total_destroyed.load(Ordering::Relaxed),
            acquire_timeouts: self.stats.acquire_timeouts.load(Ordering::Relaxed),
        }
    }

    /// Get pool configuration
    pub fn config(&self) -> &PoolConfig {
        &self.config
    }

    /// Drain the pool, destroying all warm sandboxes
    pub async fn drain(&self) {
        let mut warm = self.warm.lock().await;
        while let Some(mut sandbox) = warm.pop_front() {
            let _ = sandbox.destroy().await;
            self.stats.total_destroyed.fetch_add(1, Ordering::Relaxed);
            self.capacity.add_permits(1);
        }
    }

    /// Create a new sandbox using the factory
    async fn create_sandbox(&self) -> SandboxResult<F::Sandbox> {
        let sandbox = self
            .factory
            .create(self.config.sandbox_config.clone())
            .await?;
        self.stats.total_created.fetch_add(1, Ordering::Relaxed);
        Ok(sandbox)
    }
}

/// RAII guard that returns sandbox to pool on drop
#[allow(dead_code)]
pub struct PooledSandbox<F: SandboxFactory + 'static> {
    sandbox: Option<F::Sandbox>,
    pool: Arc<SandboxPool<F>>,
}

#[allow(dead_code)]
impl<F: SandboxFactory + 'static> PooledSandbox<F>
where
    F::Sandbox: 'static,
{
    /// Create a new pooled sandbox guard
    pub fn new(sandbox: F::Sandbox, pool: Arc<SandboxPool<F>>) -> Self {
        Self {
            sandbox: Some(sandbox),
            pool,
        }
    }

    /// Get a reference to the sandbox
    pub fn sandbox(&self) -> &F::Sandbox {
        self.sandbox.as_ref().expect("sandbox already taken")
    }

    /// Get a mutable reference to the sandbox
    pub fn sandbox_mut(&mut self) -> &mut F::Sandbox {
        self.sandbox.as_mut().expect("sandbox already taken")
    }

    /// Take the sandbox out without returning to pool
    pub fn take(mut self) -> F::Sandbox {
        self.sandbox.take().expect("sandbox already taken")
    }
}

impl<F: SandboxFactory + 'static> Drop for PooledSandbox<F>
where
    F::Sandbox: 'static,
{
    fn drop(&mut self) {
        if let Some(sandbox) = self.sandbox.take() {
            let pool = Arc::clone(&self.pool);
            // Spawn release task in background - we don't need to wait for completion in drop
            #[allow(clippy::let_underscore_future)]
            let _ = kelpie_core::current_runtime().spawn(async move {
                pool.release(sandbox).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockSandboxFactory;

    #[tokio::test]
    async fn test_pool_config_validation() {
        let config = PoolConfig::default().with_min_size(10).with_max_size(5);

        assert!(config.validate().is_err());

        let config = PoolConfig::default().with_min_size(2).with_max_size(10);

        assert!(config.validate().is_ok());
    }

    #[tokio::test]
    async fn test_pool_warm_up() {
        let factory = MockSandboxFactory::new();
        let config = PoolConfig::default().with_min_size(3).with_max_size(5);

        let pool = SandboxPool::new(factory, config).unwrap();
        pool.warm_up().await.unwrap();

        let stats = pool.stats().await;
        assert_eq!(stats.warm_count, 3);
        assert_eq!(stats.total_created, 3);
    }

    #[tokio::test]
    async fn test_pool_acquire_warm() {
        let factory = MockSandboxFactory::new();
        let config = PoolConfig::default().with_min_size(2).with_max_size(5);

        let pool = SandboxPool::new(factory, config).unwrap();
        pool.warm_up().await.unwrap();

        // Acquire should get a warm sandbox
        let sandbox = pool.acquire().await.unwrap();
        assert_eq!(sandbox.state(), SandboxState::Running);

        let stats = pool.stats().await;
        assert_eq!(stats.warm_count, 1);
        assert_eq!(stats.total_acquired, 1);
    }

    #[tokio::test]
    async fn test_pool_acquire_cold() {
        let factory = MockSandboxFactory::new();
        let config = PoolConfig::default().with_min_size(0).with_max_size(5);

        let pool = SandboxPool::new(factory, config).unwrap();
        // Don't warm up - pool is empty

        // Acquire should create a new sandbox
        let sandbox = pool.acquire().await.unwrap();
        assert_eq!(sandbox.state(), SandboxState::Running);

        let stats = pool.stats().await;
        assert_eq!(stats.total_created, 1);
        assert_eq!(stats.total_acquired, 1);
    }

    #[tokio::test]
    async fn test_pool_release_healthy() {
        let factory = MockSandboxFactory::new();
        let config = PoolConfig::default().with_min_size(2).with_max_size(5);

        let pool = SandboxPool::new(factory, config).unwrap();
        pool.warm_up().await.unwrap();

        let sandbox = pool.acquire().await.unwrap();
        pool.release(sandbox).await;

        let stats = pool.stats().await;
        assert_eq!(stats.warm_count, 2); // Back to min
        assert_eq!(stats.total_returned, 1);
    }

    #[tokio::test]
    async fn test_pool_release_excess() {
        let factory = MockSandboxFactory::new();
        let config = PoolConfig::default().with_min_size(1).with_max_size(5);

        let pool = SandboxPool::new(factory, config).unwrap();
        pool.warm_up().await.unwrap();

        // Acquire multiple sandboxes
        let s1 = pool.acquire().await.unwrap();
        let s2 = pool.acquire().await.unwrap();

        // Return both
        pool.release(s1).await;
        pool.release(s2).await;

        // Should only keep up to min_size
        let stats = pool.stats().await;
        assert_eq!(stats.warm_count, 1);
        assert!(stats.total_destroyed >= 1);
    }

    #[tokio::test]
    async fn test_pool_drain() {
        let factory = MockSandboxFactory::new();
        let config = PoolConfig::default().with_min_size(3).with_max_size(5);

        let pool = SandboxPool::new(factory, config).unwrap();
        pool.warm_up().await.unwrap();

        pool.drain().await;

        let stats = pool.stats().await;
        assert_eq!(stats.warm_count, 0);
        assert_eq!(stats.total_destroyed, 3);
    }

    #[tokio::test]
    async fn test_pool_stats() {
        let factory = MockSandboxFactory::new();
        let config = PoolConfig::default().with_min_size(2).with_max_size(5);

        let pool = SandboxPool::new(factory, config).unwrap();
        pool.warm_up().await.unwrap();

        let s1 = pool.acquire().await.unwrap();
        let s2 = pool.acquire().await.unwrap();

        let stats = pool.stats().await;
        assert_eq!(stats.warm_count, 0);
        assert_eq!(stats.in_use_count, 2);
        assert_eq!(stats.total_acquired, 2);

        pool.release(s1).await;
        pool.release(s2).await;
    }

    #[tokio::test]
    async fn test_pooled_sandbox_raii() {
        let factory = MockSandboxFactory::new();
        let config = PoolConfig::default().with_min_size(1).with_max_size(5);

        let pool = Arc::new(SandboxPool::new(factory, config).unwrap());
        pool.warm_up().await.unwrap();

        {
            let sandbox = pool.acquire().await.unwrap();
            let _guard = PooledSandbox::new(sandbox, Arc::clone(&pool));
            // Sandbox will be returned when guard is dropped
        }

        // Give the async release time to complete
        kelpie_core::TokioRuntime
            .sleep(Duration::from_millis(10))
            .await;

        let stats = pool.stats().await;
        assert_eq!(stats.total_returned, 1);
    }
}
