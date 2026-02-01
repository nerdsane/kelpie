//! Lease management for single-activation guarantees
//!
//! TigerStyle: Explicit lease semantics matching TLA+ spec (KelpieLease.tla).
//!
//! # Invariants
//!
//! - LeaseUniqueness: At most one node holds a valid lease per actor
//! - RenewalRequiresOwnership: Only lease holder can renew
//! - ExpiredLeaseClaimable: Expired leases don't block new acquisition
//!
//! # Example
//!
//! ```rust,ignore
//! use kelpie_registry::{MemoryLeaseManager, LeaseManager, LeaseConfig};
//! use kelpie_core::actor::ActorId;
//!
//! let config = LeaseConfig::default();
//! let lease_mgr = MemoryLeaseManager::new(config, clock);
//!
//! // Acquire lease
//! let lease = lease_mgr.acquire(&node_id, &actor_id).await?;
//!
//! // Renew before expiry
//! lease_mgr.renew(&node_id, &actor_id).await?;
//!
//! // Release when done
//! lease_mgr.release(&node_id, &actor_id).await?;
//! ```

use crate::error::{RegistryError, RegistryResult};
use crate::node::NodeId;
use async_trait::async_trait;
use kelpie_core::actor::ActorId;
use kelpie_core::io::TimeProvider;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// =============================================================================
// Constants (TigerStyle: explicit units in names)
// =============================================================================

/// Default lease duration in milliseconds
pub const LEASE_DURATION_MS_DEFAULT: u64 = 30_000; // 30 seconds

/// Minimum lease duration in milliseconds
pub const LEASE_DURATION_MS_MIN: u64 = 1_000; // 1 second

/// Maximum lease duration in milliseconds
pub const LEASE_DURATION_MS_MAX: u64 = 300_000; // 5 minutes

// =============================================================================
// Lease Configuration
// =============================================================================

/// Configuration for lease management
#[derive(Debug, Clone)]
pub struct LeaseConfig {
    /// Lease duration in milliseconds
    pub duration_ms: u64,
}

impl LeaseConfig {
    /// Create a new lease config with specified duration
    pub fn new(duration_ms: u64) -> Self {
        // TigerStyle: assertions for bounds
        assert!(
            duration_ms >= LEASE_DURATION_MS_MIN,
            "lease duration must be >= {}ms",
            LEASE_DURATION_MS_MIN
        );
        assert!(
            duration_ms <= LEASE_DURATION_MS_MAX,
            "lease duration must be <= {}ms",
            LEASE_DURATION_MS_MAX
        );

        Self { duration_ms }
    }

    /// Create config for testing with short duration
    pub fn for_testing() -> Self {
        Self { duration_ms: 5_000 } // 5 seconds for fast tests
    }
}

impl Default for LeaseConfig {
    fn default() -> Self {
        Self {
            duration_ms: LEASE_DURATION_MS_DEFAULT,
        }
    }
}

// =============================================================================
// Lease Structure
// =============================================================================

/// A lease granting exclusive ownership of an actor to a node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lease {
    /// The actor this lease is for
    pub actor_id: ActorId,
    /// The node holding this lease
    pub holder: NodeId,
    /// When this lease expires (Unix timestamp ms)
    pub expiry_ms: u64,
    /// When this lease was acquired (Unix timestamp ms)
    pub acquired_at_ms: u64,
    /// Number of times this lease has been renewed
    pub renewal_count: u64,
}

impl Lease {
    /// Create a new lease
    pub fn new(actor_id: ActorId, holder: NodeId, now_ms: u64, duration_ms: u64) -> Self {
        // TigerStyle: preconditions
        assert!(duration_ms > 0, "lease duration must be positive");
        assert!(
            now_ms.checked_add(duration_ms).is_some(),
            "expiry would overflow"
        );

        let lease = Self {
            actor_id,
            holder,
            expiry_ms: now_ms + duration_ms,
            acquired_at_ms: now_ms,
            renewal_count: 0,
        };

        // TigerStyle: postconditions
        debug_assert!(lease.expiry_ms > now_ms);
        debug_assert_eq!(lease.renewal_count, 0);

        lease
    }

    /// Check if the lease is valid at the given time
    pub fn is_valid(&self, now_ms: u64) -> bool {
        self.expiry_ms > now_ms
    }

    /// Check if the lease has expired at the given time
    pub fn is_expired(&self, now_ms: u64) -> bool {
        self.expiry_ms <= now_ms
    }

    /// Renew this lease, extending its expiry
    pub fn renew(&mut self, now_ms: u64, duration_ms: u64) {
        // TigerStyle: preconditions
        assert!(duration_ms > 0, "lease duration must be positive");
        assert!(self.is_valid(now_ms), "cannot renew expired lease");
        assert!(
            now_ms.checked_add(duration_ms).is_some(),
            "expiry would overflow"
        );

        self.expiry_ms = now_ms + duration_ms;
        self.renewal_count = self.renewal_count.saturating_add(1);

        // TigerStyle: postconditions
        debug_assert!(self.expiry_ms > now_ms);
    }

    /// Get remaining time on the lease in milliseconds
    pub fn remaining_ms(&self, now_ms: u64) -> u64 {
        self.expiry_ms.saturating_sub(now_ms)
    }
}

// =============================================================================
// LeaseManager Trait
// =============================================================================

/// Trait for lease management operations
#[async_trait]
pub trait LeaseManager: Send + Sync {
    /// Attempt to acquire a lease for an actor.
    ///
    /// Returns Ok(Lease) if acquisition succeeds, Err if another node holds a valid lease.
    async fn acquire(&self, node_id: &NodeId, actor_id: &ActorId) -> RegistryResult<Lease>;

    /// Renew an existing lease.
    ///
    /// Only the current holder can renew. Returns Err if not holder or lease expired.
    async fn renew(&self, node_id: &NodeId, actor_id: &ActorId) -> RegistryResult<Lease>;

    /// Release a lease voluntarily (graceful deactivation).
    ///
    /// Returns Ok if released, Err if not holder.
    async fn release(&self, node_id: &NodeId, actor_id: &ActorId) -> RegistryResult<()>;

    /// Check if a node holds a valid lease for an actor.
    async fn is_valid(&self, node_id: &NodeId, actor_id: &ActorId) -> bool;

    /// Get the current lease for an actor, if any.
    async fn get_lease(&self, actor_id: &ActorId) -> Option<Lease>;

    /// Get all active leases held by a node.
    async fn get_leases_for_node(&self, node_id: &NodeId) -> Vec<Lease>;
}

// =============================================================================
// MemoryLeaseManager Implementation
// =============================================================================

/// In-memory lease manager for testing and single-node deployments
pub struct MemoryLeaseManager {
    /// Lease configuration
    config: LeaseConfig,
    /// Time provider for time operations (DST-compatible)
    time: Arc<dyn TimeProvider>,
    /// Active leases by actor ID
    leases: RwLock<HashMap<String, Lease>>,
}

impl MemoryLeaseManager {
    /// Create a new memory lease manager
    pub fn new(config: LeaseConfig, time: Arc<dyn TimeProvider>) -> Self {
        Self {
            config,
            time,
            leases: RwLock::new(HashMap::new()),
        }
    }

    /// Create with default config
    pub fn with_time(time: Arc<dyn TimeProvider>) -> Self {
        Self::new(LeaseConfig::default(), time)
    }

    /// Get current time from time provider
    fn now_ms(&self) -> u64 {
        self.time.now_ms()
    }
}

impl std::fmt::Debug for MemoryLeaseManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryLeaseManager")
            .field("config", &self.config)
            .finish()
    }
}

#[async_trait]
impl LeaseManager for MemoryLeaseManager {
    async fn acquire(&self, node_id: &NodeId, actor_id: &ActorId) -> RegistryResult<Lease> {
        // TigerStyle: preconditions
        assert!(!node_id.as_str().is_empty(), "node_id cannot be empty");
        assert!(!actor_id.id().is_empty(), "actor_id cannot be empty");

        let now_ms = self.now_ms();
        let key = actor_id.qualified_name();

        let mut leases = self.leases.write().await;

        // Check if there's an existing valid lease
        if let Some(existing) = leases.get(&key) {
            if existing.is_valid(now_ms) {
                // Lease exists and is valid - cannot acquire
                return Err(RegistryError::LeaseHeldByOther {
                    actor_id: actor_id.qualified_name(),
                    holder: existing.holder.as_str().to_string(),
                    expiry_ms: existing.expiry_ms,
                });
            }
            // Lease exists but expired - can be claimed
        }

        // Create new lease
        let lease = Lease::new(
            actor_id.clone(),
            node_id.clone(),
            now_ms,
            self.config.duration_ms,
        );

        leases.insert(key, lease.clone());

        // TigerStyle: postcondition
        debug_assert!(lease.is_valid(now_ms));

        Ok(lease)
    }

    async fn renew(&self, node_id: &NodeId, actor_id: &ActorId) -> RegistryResult<Lease> {
        // TigerStyle: preconditions
        assert!(!node_id.as_str().is_empty(), "node_id cannot be empty");
        assert!(!actor_id.id().is_empty(), "actor_id cannot be empty");

        let now_ms = self.now_ms();
        let key = actor_id.qualified_name();

        let mut leases = self.leases.write().await;

        let lease = leases
            .get_mut(&key)
            .ok_or_else(|| RegistryError::LeaseNotFound {
                actor_id: actor_id.qualified_name(),
            })?;

        // Check holder matches
        if lease.holder != *node_id {
            return Err(RegistryError::NotLeaseHolder {
                actor_id: actor_id.qualified_name(),
                holder: lease.holder.as_str().to_string(),
                requester: node_id.as_str().to_string(),
            });
        }

        // Check lease is still valid
        if lease.is_expired(now_ms) {
            return Err(RegistryError::LeaseExpired {
                actor_id: actor_id.qualified_name(),
                expiry_ms: lease.expiry_ms,
            });
        }

        // Renew the lease
        lease.renew(now_ms, self.config.duration_ms);

        // TigerStyle: postcondition
        debug_assert!(lease.is_valid(now_ms));

        Ok(lease.clone())
    }

    async fn release(&self, node_id: &NodeId, actor_id: &ActorId) -> RegistryResult<()> {
        // TigerStyle: preconditions
        assert!(!node_id.as_str().is_empty(), "node_id cannot be empty");
        assert!(!actor_id.id().is_empty(), "actor_id cannot be empty");

        let key = actor_id.qualified_name();

        let mut leases = self.leases.write().await;

        let lease = leases
            .get(&key)
            .ok_or_else(|| RegistryError::LeaseNotFound {
                actor_id: actor_id.qualified_name(),
            })?;

        // Check holder matches
        if lease.holder != *node_id {
            return Err(RegistryError::NotLeaseHolder {
                actor_id: actor_id.qualified_name(),
                holder: lease.holder.as_str().to_string(),
                requester: node_id.as_str().to_string(),
            });
        }

        // Remove the lease
        leases.remove(&key);

        Ok(())
    }

    async fn is_valid(&self, node_id: &NodeId, actor_id: &ActorId) -> bool {
        let now_ms = self.now_ms();
        let key = actor_id.qualified_name();

        let leases = self.leases.read().await;

        leases
            .get(&key)
            .map(|lease| lease.holder == *node_id && lease.is_valid(now_ms))
            .unwrap_or(false)
    }

    async fn get_lease(&self, actor_id: &ActorId) -> Option<Lease> {
        let key = actor_id.qualified_name();
        let leases = self.leases.read().await;
        leases.get(&key).cloned()
    }

    async fn get_leases_for_node(&self, node_id: &NodeId) -> Vec<Lease> {
        let now_ms = self.now_ms();
        let leases = self.leases.read().await;

        leases
            .values()
            .filter(|lease| lease.holder == *node_id && lease.is_valid(now_ms))
            .cloned()
            .collect()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Test clock with controllable time (implements TimeProvider)
    struct TestClock {
        time_ms: AtomicU64,
    }

    impl TestClock {
        fn new(initial_ms: u64) -> Self {
            Self {
                time_ms: AtomicU64::new(initial_ms),
            }
        }

        fn advance(&self, ms: u64) {
            self.time_ms.fetch_add(ms, Ordering::SeqCst);
        }
    }

    impl std::fmt::Debug for TestClock {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "TestClock({})", self.time_ms.load(Ordering::SeqCst))
        }
    }

    #[async_trait]
    impl TimeProvider for TestClock {
        fn now_ms(&self) -> u64 {
            self.time_ms.load(Ordering::SeqCst)
        }

        async fn sleep_ms(&self, ms: u64) {
            self.time_ms.fetch_add(ms, Ordering::SeqCst);
        }

        fn monotonic_ms(&self) -> u64 {
            self.now_ms()
        }
    }

    fn test_node_id(n: u32) -> NodeId {
        NodeId::new(format!("node-{}", n)).unwrap()
    }

    fn test_actor_id(n: u32) -> ActorId {
        ActorId::new("test", format!("actor-{}", n)).unwrap()
    }

    #[test]
    fn test_lease_creation() {
        let actor_id = test_actor_id(1);
        let node_id = test_node_id(1);

        let lease = Lease::new(actor_id.clone(), node_id.clone(), 1000, 5000);

        assert_eq!(lease.actor_id, actor_id);
        assert_eq!(lease.holder, node_id);
        assert_eq!(lease.acquired_at_ms, 1000);
        assert_eq!(lease.expiry_ms, 6000);
        assert_eq!(lease.renewal_count, 0);
    }

    #[test]
    fn test_lease_validity() {
        let lease = Lease::new(test_actor_id(1), test_node_id(1), 1000, 5000);

        assert!(lease.is_valid(1000));
        assert!(lease.is_valid(5999));
        assert!(!lease.is_valid(6000));
        assert!(!lease.is_valid(7000));
    }

    #[test]
    fn test_lease_renewal() {
        let mut lease = Lease::new(test_actor_id(1), test_node_id(1), 1000, 5000);
        assert_eq!(lease.expiry_ms, 6000);

        lease.renew(3000, 5000);
        assert_eq!(lease.expiry_ms, 8000);
        assert_eq!(lease.renewal_count, 1);
    }

    #[test]
    fn test_lease_remaining_time() {
        let lease = Lease::new(test_actor_id(1), test_node_id(1), 1000, 5000);

        assert_eq!(lease.remaining_ms(1000), 5000);
        assert_eq!(lease.remaining_ms(3000), 3000);
        assert_eq!(lease.remaining_ms(6000), 0);
        assert_eq!(lease.remaining_ms(7000), 0);
    }

    #[tokio::test]
    async fn test_memory_lease_manager_acquire() {
        let clock = Arc::new(TestClock::new(1000));
        let mgr = MemoryLeaseManager::new(LeaseConfig::for_testing(), clock);

        let node_id = test_node_id(1);
        let actor_id = test_actor_id(1);

        let lease = mgr.acquire(&node_id, &actor_id).await.unwrap();
        assert_eq!(lease.holder, node_id);
        assert!(lease.is_valid(1000));
    }

    #[tokio::test]
    async fn test_memory_lease_manager_acquire_conflict() {
        let clock = Arc::new(TestClock::new(1000));
        let mgr = MemoryLeaseManager::new(LeaseConfig::for_testing(), clock);

        let node1 = test_node_id(1);
        let node2 = test_node_id(2);
        let actor_id = test_actor_id(1);

        // Node 1 acquires
        mgr.acquire(&node1, &actor_id).await.unwrap();

        // Node 2 tries to acquire - should fail
        let result = mgr.acquire(&node2, &actor_id).await;
        assert!(matches!(
            result,
            Err(RegistryError::LeaseHeldByOther { .. })
        ));
    }

    #[tokio::test]
    async fn test_memory_lease_manager_acquire_after_expiry() {
        let clock = Arc::new(TestClock::new(1000));
        let mgr = MemoryLeaseManager::new(LeaseConfig::for_testing(), clock.clone());

        let node1 = test_node_id(1);
        let node2 = test_node_id(2);
        let actor_id = test_actor_id(1);

        // Node 1 acquires
        mgr.acquire(&node1, &actor_id).await.unwrap();

        // Advance time past expiry
        clock.advance(6000);

        // Node 2 can now acquire
        let lease = mgr.acquire(&node2, &actor_id).await.unwrap();
        assert_eq!(lease.holder, node2);
    }

    #[tokio::test]
    async fn test_memory_lease_manager_renew() {
        let clock = Arc::new(TestClock::new(1000));
        let mgr = MemoryLeaseManager::new(LeaseConfig::for_testing(), clock.clone());

        let node_id = test_node_id(1);
        let actor_id = test_actor_id(1);

        // Acquire
        mgr.acquire(&node_id, &actor_id).await.unwrap();

        // Advance time but not past expiry
        clock.advance(2000);

        // Renew
        let renewed = mgr.renew(&node_id, &actor_id).await.unwrap();
        assert_eq!(renewed.renewal_count, 1);
        assert!(renewed.is_valid(3000));
    }

    #[tokio::test]
    async fn test_memory_lease_manager_renew_wrong_holder() {
        let clock = Arc::new(TestClock::new(1000));
        let mgr = MemoryLeaseManager::new(LeaseConfig::for_testing(), clock);

        let node1 = test_node_id(1);
        let node2 = test_node_id(2);
        let actor_id = test_actor_id(1);

        // Node 1 acquires
        mgr.acquire(&node1, &actor_id).await.unwrap();

        // Node 2 tries to renew - should fail
        let result = mgr.renew(&node2, &actor_id).await;
        assert!(matches!(result, Err(RegistryError::NotLeaseHolder { .. })));
    }

    #[tokio::test]
    async fn test_memory_lease_manager_release() {
        let clock = Arc::new(TestClock::new(1000));
        let mgr = MemoryLeaseManager::new(LeaseConfig::for_testing(), clock);

        let node_id = test_node_id(1);
        let actor_id = test_actor_id(1);

        // Acquire
        mgr.acquire(&node_id, &actor_id).await.unwrap();
        assert!(mgr.is_valid(&node_id, &actor_id).await);

        // Release
        mgr.release(&node_id, &actor_id).await.unwrap();
        assert!(!mgr.is_valid(&node_id, &actor_id).await);
    }

    #[tokio::test]
    async fn test_memory_lease_manager_is_valid() {
        let clock = Arc::new(TestClock::new(1000));
        let mgr = MemoryLeaseManager::new(LeaseConfig::for_testing(), clock.clone());

        let node_id = test_node_id(1);
        let actor_id = test_actor_id(1);

        // Not valid before acquisition
        assert!(!mgr.is_valid(&node_id, &actor_id).await);

        // Valid after acquisition
        mgr.acquire(&node_id, &actor_id).await.unwrap();
        assert!(mgr.is_valid(&node_id, &actor_id).await);

        // Not valid after expiry
        clock.advance(6000);
        assert!(!mgr.is_valid(&node_id, &actor_id).await);
    }
}
