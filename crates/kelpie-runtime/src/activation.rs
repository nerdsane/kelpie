//! Actor activation and lifecycle management
//!
//! TigerStyle: Explicit lifecycle states, single activation guarantee.

use crate::mailbox::{Envelope, Mailbox};
use bytes::Bytes;
use kelpie_core::actor::{
    Actor, ActorContext, ActorId, ArcContextKV, BufferedKVOp, BufferingContextKV,
};
use kelpie_core::constants::{ACTOR_IDLE_TIMEOUT_MS_DEFAULT, ACTOR_INVOCATION_TIMEOUT_MS_MAX};
use kelpie_core::error::{Error, Result};
use kelpie_core::io::{TimeProvider, WallClockTime};
use kelpie_storage::{ActorKV, ScopedKV};
use serde::{de::DeserializeOwned, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, instrument, warn};

/// State key for actor's serialized state
const STATE_KEY: &[u8] = b"__state__";

/// Actor lifecycle state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationState {
    /// Actor is being activated (loading state)
    Activating,
    /// Actor is active and ready to process messages
    Active,
    /// Actor is being deactivated (persisting state)
    Deactivating,
    /// Actor has been deactivated
    Deactivated,
}

impl std::fmt::Display for ActivationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActivationState::Activating => write!(f, "activating"),
            ActivationState::Active => write!(f, "active"),
            ActivationState::Deactivating => write!(f, "deactivating"),
            ActivationState::Deactivated => write!(f, "deactivated"),
        }
    }
}

/// Statistics for an active actor
///
/// Uses monotonic timestamps (u64 ms) for DST compatibility.
#[derive(Debug, Clone, Default)]
pub struct ActivationStats {
    /// When the actor was activated (monotonic ms)
    pub activated_at_ms: Option<u64>,
    /// Last time the actor processed a message (monotonic ms)
    pub last_activity_at_ms: Option<u64>,
    /// Total invocations processed
    pub invocation_count: u64,
    /// Total invocation errors
    pub error_count: u64,
    /// Total time spent processing in ms (for average calculation)
    pub total_processing_time_ms: u64,
}

impl ActivationStats {
    /// Create new stats with activation time (uses production wall clock)
    pub fn new() -> Self {
        Self::with_time(&WallClockTime::new())
    }

    /// Create new stats with custom time provider (for DST)
    pub fn with_time(time: &dyn TimeProvider) -> Self {
        Self {
            activated_at_ms: Some(time.monotonic_ms()),
            last_activity_at_ms: None,
            invocation_count: 0,
            error_count: 0,
            total_processing_time_ms: 0,
        }
    }

    /// Record an invocation (uses wall clock time)
    ///
    /// For DST compatibility, use `record_invocation_with_time` instead.
    pub fn record_invocation(&mut self, duration_ms: u64, is_error: bool) {
        self.record_invocation_with_time(duration_ms, is_error, &WallClockTime::new());
    }

    /// Record an invocation with time provider (for DST)
    pub fn record_invocation_with_time(
        &mut self,
        duration_ms: u64,
        is_error: bool,
        time: &dyn TimeProvider,
    ) {
        self.last_activity_at_ms = Some(time.monotonic_ms());
        self.invocation_count = self.invocation_count.wrapping_add(1);
        self.total_processing_time_ms = self.total_processing_time_ms.saturating_add(duration_ms);
        if is_error {
            self.error_count = self.error_count.wrapping_add(1);
        }
    }

    /// Get idle time (time since last activity) using time provider
    pub fn idle_time_ms(&self, time: &dyn TimeProvider) -> u64 {
        let now_ms = time.monotonic_ms();
        match self.last_activity_at_ms {
            Some(t) => now_ms.saturating_sub(t),
            None => self
                .activated_at_ms
                .map(|t| now_ms.saturating_sub(t))
                .unwrap_or(0),
        }
    }

    /// Get average processing time per invocation
    pub fn average_processing_time(&self) -> Duration {
        if self.invocation_count == 0 {
            Duration::ZERO
        } else {
            Duration::from_millis(self.total_processing_time_ms / self.invocation_count)
        }
    }
}

/// An active actor instance
///
/// TigerStyle: Single activation guarantee - only one ActiveActor per ActorId
/// can exist in the cluster at any time.
pub struct ActiveActor<A, S>
where
    A: Actor<State = S>,
    S: Serialize + DeserializeOwned + Default + Send + Sync,
{
    /// The actor's unique identifier
    pub id: ActorId,
    /// The actor implementation
    actor: A,
    /// The actor's context (state + KV access)
    context: ActorContext<S>,
    /// The actor's mailbox
    mailbox: Mailbox,
    /// Current lifecycle state
    state: ActivationState,
    /// Statistics
    stats: ActivationStats,
    /// Idle timeout before deactivation
    idle_timeout: Duration,
    /// Scoped KV store for persistence (bound to this actor)
    kv: ScopedKV,
    /// Time provider for DST compatibility
    time: Arc<dyn TimeProvider>,
}

impl<A, S> ActiveActor<A, S>
where
    A: Actor<State = S>,
    S: Serialize + DeserializeOwned + Default + Send + Sync + Clone,
{
    /// Activate an actor using production wall clock
    ///
    /// For DST, use `activate_with_time`.
    #[instrument(skip(actor, kv), fields(actor_id = %id), level = "info")]
    pub async fn activate(id: ActorId, actor: A, kv: Arc<dyn ActorKV>) -> Result<Self> {
        Self::activate_with_time(id, actor, kv, Arc::new(WallClockTime::new())).await
    }

    /// Activate an actor with custom time provider (for DST)
    ///
    /// Loads state from storage and calls on_activate.
    #[instrument(skip(actor, kv, time), fields(actor_id = %id), level = "info")]
    pub async fn activate_with_time(
        id: ActorId,
        actor: A,
        kv: Arc<dyn ActorKV>,
        time: Arc<dyn TimeProvider>,
    ) -> Result<Self> {
        debug!(actor_id = %id, "Activating actor");

        // Create a scoped KV bound to this actor
        let scoped_kv = ScopedKV::new(id.clone(), kv.clone());

        // Create a second scoped KV for the context (ScopedKV doesn't implement Clone)
        let context_kv = ScopedKV::new(id.clone(), kv);

        let mut active = Self {
            id: id.clone(),
            actor,
            context: ActorContext::with_default_state(id.clone(), Box::new(context_kv)),
            mailbox: Mailbox::new(),
            state: ActivationState::Activating,
            stats: ActivationStats::with_time(time.as_ref()),
            idle_timeout: Duration::from_millis(ACTOR_IDLE_TIMEOUT_MS_DEFAULT),
            time,
            kv: scoped_kv,
        };

        // Load state from storage
        active.load_state().await?;

        // Call on_activate hook
        if let Err(e) = active.actor.on_activate(&mut active.context).await {
            error!(actor_id = %active.id, error = %e, "on_activate failed");
            active.state = ActivationState::Deactivated;
            return Err(e);
        }

        active.state = ActivationState::Active;
        info!(actor_id = %active.id, "Actor activated");

        Ok(active)
    }

    /// Load state from storage
    async fn load_state(&mut self) -> Result<()> {
        match self.kv.get(STATE_KEY).await {
            Ok(Some(bytes)) => {
                let state: S = serde_json::from_slice(&bytes).map_err(|e| Error::Internal {
                    message: format!("Failed to deserialize actor state: {}", e),
                })?;
                self.context.state = state;
                debug!(actor_id = %self.id, "Loaded state from storage");
            }
            Ok(None) => {
                debug!(actor_id = %self.id, "No existing state, using default");
            }
            Err(e) => {
                warn!(actor_id = %self.id, error = %e, "Failed to load state, using default");
                // Continue with default state - this allows recovery from storage issues
            }
        }
        Ok(())
    }

    /// Save state to storage
    async fn save_state(&mut self) -> Result<()> {
        let bytes = serde_json::to_vec(&self.context.state).map_err(|e| Error::Internal {
            message: format!("Failed to serialize actor state: {}", e),
        })?;

        self.kv
            .set(STATE_KEY, &bytes)
            .await
            .map_err(|e| Error::Internal {
                message: format!("Failed to persist actor state: {}", e),
            })?;

        debug!(actor_id = %self.id, "Saved state to storage");
        Ok(())
    }

    /// Process an invocation using the actor's time provider
    ///
    /// State AND KV operations are persisted atomically within a single transaction
    /// after each successful invocation. This ensures crash safety - if the node
    /// crashes, either all changes (state + KV) are persisted or none are.
    ///
    /// TigerStyle: Transactional state + KV persistence, 2+ assertions.
    #[instrument(skip(self, payload), fields(actor_id = %self.id, operation), level = "info")]
    pub async fn process_invocation(&mut self, operation: &str, payload: Bytes) -> Result<Bytes> {
        self.process_invocation_with_time(operation, payload, self.time.clone())
            .await
    }

    /// Process an invocation with external time provider (for DST)
    ///
    /// State AND KV operations are persisted atomically within a single transaction
    /// after each successful invocation. This ensures crash safety - if the node
    /// crashes, either all changes (state + KV) are persisted or none are.
    ///
    /// TigerStyle: Transactional state + KV persistence, 2+ assertions.
    #[instrument(skip(self, payload, time), fields(actor_id = %self.id, operation), level = "info")]
    pub async fn process_invocation_with_time(
        &mut self,
        operation: &str,
        payload: Bytes,
        time: Arc<dyn TimeProvider>,
    ) -> Result<Bytes> {
        // Preconditions
        assert!(
            self.state == ActivationState::Active,
            "Can only process invocations when active"
        );
        assert!(!operation.is_empty(), "operation cannot be empty");

        // Use time provider for DST determinism
        let start_ms = time.monotonic_ms();

        // CRITICAL: Snapshot state BEFORE invoke for rollback on failure
        // If transaction fails, we must restore state to match what's persisted
        let state_snapshot = self.context.state.clone();

        // Create a buffering KV to capture all KV operations during invoke
        let buffering_kv = Arc::new(BufferingContextKV::new(
            // Create a new ScopedKV for the buffering wrapper to read from
            Box::new(ScopedKV::new(self.id.clone(), self.kv.underlying_kv())),
        ));

        // Swap in the buffering KV (wrapped in Arc for sharing)
        let original_kv = self
            .context
            .swap_kv(Box::new(ArcContextKV(buffering_kv.clone())));

        // Execute the actor's invoke with the buffering KV
        let runtime = kelpie_core::current_runtime();
        let result = kelpie_core::Runtime::timeout(
            &runtime,
            Duration::from_millis(ACTOR_INVOCATION_TIMEOUT_MS_MAX),
            self.actor
                .invoke(&mut self.context, operation, payload.clone()),
        )
        .await;

        // Restore the original KV
        let _ = self.context.swap_kv(original_kv);

        // Drain buffered operations from our Arc reference
        let buffered_ops = buffering_kv.drain_buffer();

        let duration_ms = time.monotonic_ms().saturating_sub(start_ms);

        // On successful invocation, persist state AND KV atomically in a transaction
        let final_result = match result {
            Ok(Ok(response)) => {
                // Save state + KV in a single transaction (crash-safe, atomic)
                match self.save_all_transactional(&buffered_ops).await {
                    Ok(()) => Ok(response),
                    Err(e) => {
                        // Transaction failed - neither state nor KV persisted
                        // ROLLBACK: Restore state to match what's persisted
                        self.context.state = state_snapshot;
                        error!(
                            actor_id = %self.id,
                            operation = %operation,
                            error = %e,
                            "Failed to persist state and KV after invocation, state rolled back"
                        );
                        Err(e)
                    }
                }
            }
            Ok(Err(e)) => {
                // Actor returned an error - don't persist any changes
                // ROLLBACK: Restore state to match what's persisted
                self.context.state = state_snapshot;
                debug!(
                    actor_id = %self.id,
                    operation = %operation,
                    error = %e,
                    buffered_kv_ops = buffered_ops.len(),
                    "Invocation failed, state rolled back"
                );
                Err(e)
            }
            Err(_) => {
                // Timeout - rollback state as well
                self.context.state = state_snapshot;
                Err(Error::OperationTimedOut {
                    operation: operation.to_string(),
                    timeout_ms: ACTOR_INVOCATION_TIMEOUT_MS_MAX,
                })
            }
        };

        self.stats
            .record_invocation_with_time(duration_ms, final_result.is_err(), time.as_ref());
        final_result
    }

    /// Save state AND buffered KV operations atomically in a single transaction
    ///
    /// This ensures that all changes made during an invocation (both state and KV)
    /// are persisted atomically. If the transaction fails, neither state nor KV
    /// changes are persisted.
    ///
    /// TigerStyle: Atomic persistence of all invocation changes.
    async fn save_all_transactional(&mut self, buffered_ops: &[BufferedKVOp]) -> Result<()> {
        let state_bytes = serde_json::to_vec(&self.context.state).map_err(|e| Error::Internal {
            message: format!("Failed to serialize actor state: {}", e),
        })?;

        // Begin transaction
        let mut txn = self
            .kv
            .begin_transaction()
            .await
            .map_err(|e| Error::Internal {
                message: format!("Failed to begin transaction: {}", e),
            })?;

        // Apply all buffered KV operations to the transaction
        for op in buffered_ops {
            match op {
                BufferedKVOp::Set { key, value } => {
                    txn.set(key, value).await.map_err(|e| Error::Internal {
                        message: format!("Failed to set KV in transaction: {}", e),
                    })?;
                }
                BufferedKVOp::Delete { key } => {
                    txn.delete(key).await.map_err(|e| Error::Internal {
                        message: format!("Failed to delete KV in transaction: {}", e),
                    })?;
                }
            }
        }

        // Set state within transaction
        txn.set(STATE_KEY, &state_bytes)
            .await
            .map_err(|e| Error::Internal {
                message: format!("Failed to set state in transaction: {}", e),
            })?;

        // Commit atomically - all KV ops + state together
        txn.commit().await.map_err(|e| Error::Internal {
            message: format!("Failed to commit transaction: {}", e),
        })?;

        debug!(
            actor_id = %self.id,
            kv_ops = buffered_ops.len(),
            "State and KV persisted atomically in transaction"
        );
        Ok(())
    }

    /// Enqueue a message in the mailbox
    pub fn enqueue(
        &mut self,
        envelope: Envelope,
    ) -> std::result::Result<(), crate::mailbox::MailboxFullError> {
        self.mailbox.push(envelope)
    }

    /// Get the next message from the mailbox
    pub fn dequeue(&mut self) -> Option<Envelope> {
        self.mailbox.pop()
    }

    /// Check if the actor has pending messages
    pub fn has_pending_messages(&self) -> bool {
        !self.mailbox.is_empty()
    }

    /// Get the number of pending messages
    pub fn pending_message_count(&self) -> usize {
        self.mailbox.len()
    }

    /// Deactivate the actor
    ///
    /// Calls on_deactivate, persists state, and rejects pending messages.
    #[instrument(skip(self), fields(actor_id = %self.id), level = "info")]
    pub async fn deactivate(&mut self) -> Result<()> {
        if self.state == ActivationState::Deactivated {
            return Ok(());
        }

        debug!(actor_id = %self.id, "Deactivating actor");
        self.state = ActivationState::Deactivating;

        // Call on_deactivate hook
        if let Err(e) = self.actor.on_deactivate(&mut self.context).await {
            error!(actor_id = %self.id, error = %e, "on_deactivate failed");
            // Continue with deactivation despite error
        }

        // Persist state
        if let Err(e) = self.save_state().await {
            error!(actor_id = %self.id, error = %e, "Failed to save state during deactivation");
            // Continue with deactivation despite error
        }

        // Reject pending messages
        let pending = self.mailbox.drain();
        for envelope in pending {
            let _ = envelope.reply_tx.send(Err(Error::ActorDeactivated {
                actor_id: self.id.clone(),
            }));
        }

        self.state = ActivationState::Deactivated;
        info!(
            actor_id = %self.id,
            invocations = self.stats.invocation_count,
            errors = self.stats.error_count,
            "Actor deactivated"
        );

        Ok(())
    }

    /// Check if the actor should be deactivated due to idle timeout
    pub fn should_deactivate(&self) -> bool {
        self.state == ActivationState::Active
            && self.mailbox.is_empty()
            && self.stats.idle_time_ms(self.time.as_ref()) > self.idle_timeout.as_millis() as u64
    }

    /// Get the current activation state
    pub fn activation_state(&self) -> ActivationState {
        self.state
    }

    /// Get the actor's statistics
    pub fn stats(&self) -> &ActivationStats {
        &self.stats
    }

    /// Set the idle timeout
    pub fn set_idle_timeout(&mut self, timeout: Duration) {
        debug_assert!(
            timeout.as_millis() <= kelpie_core::constants::ACTOR_IDLE_TIMEOUT_MS_MAX as u128
        );
        self.idle_timeout = timeout;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use kelpie_storage::MemoryKV;

    #[derive(Debug, Default, Clone, Serialize, serde::Deserialize)]
    struct CounterState {
        count: i64,
    }

    struct CounterActor;

    #[async_trait]
    impl Actor for CounterActor {
        type State = CounterState;

        async fn invoke(
            &self,
            ctx: &mut ActorContext<Self::State>,
            operation: &str,
            _payload: Bytes,
        ) -> Result<Bytes> {
            match operation {
                "increment" => {
                    ctx.state.count += 1;
                    Ok(Bytes::from(ctx.state.count.to_string()))
                }
                "get" => Ok(Bytes::from(ctx.state.count.to_string())),
                _ => Err(Error::InvalidOperation {
                    operation: operation.to_string(),
                }),
            }
        }
    }

    fn create_kv() -> Arc<MemoryKV> {
        Arc::new(MemoryKV::new())
    }

    #[tokio::test]
    async fn test_actor_activation() {
        let id = ActorId::new("test", "counter-1").unwrap();
        let kv = create_kv();

        let active = ActiveActor::activate(id.clone(), CounterActor, kv)
            .await
            .unwrap();

        assert_eq!(active.activation_state(), ActivationState::Active);
        assert_eq!(active.id, id);
    }

    #[tokio::test]
    async fn test_actor_invocation() {
        let id = ActorId::new("test", "counter-2").unwrap();
        let kv = create_kv();

        let mut active = ActiveActor::activate(id, CounterActor, kv).await.unwrap();

        let result = active
            .process_invocation("increment", Bytes::new())
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("1"));

        let result = active
            .process_invocation("increment", Bytes::new())
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("2"));

        let result = active
            .process_invocation("get", Bytes::new())
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("2"));
    }

    #[tokio::test]
    async fn test_actor_state_persistence() {
        let id = ActorId::new("test", "counter-3").unwrap();
        let kv = create_kv();

        // Activate, increment, deactivate
        {
            let mut active = ActiveActor::activate(id.clone(), CounterActor, kv.clone())
                .await
                .unwrap();
            active
                .process_invocation("increment", Bytes::new())
                .await
                .unwrap();
            active
                .process_invocation("increment", Bytes::new())
                .await
                .unwrap();
            active.deactivate().await.unwrap();
        }

        // Reactivate - state should be restored
        {
            let mut active = ActiveActor::activate(id, CounterActor, kv).await.unwrap();
            let result = active
                .process_invocation("get", Bytes::new())
                .await
                .unwrap();
            assert_eq!(result, Bytes::from("2"));
        }
    }

    #[tokio::test]
    async fn test_actor_deactivation() {
        let id = ActorId::new("test", "counter-4").unwrap();
        let kv = create_kv();

        let mut active = ActiveActor::activate(id, CounterActor, kv).await.unwrap();
        assert_eq!(active.activation_state(), ActivationState::Active);

        active.deactivate().await.unwrap();
        assert_eq!(active.activation_state(), ActivationState::Deactivated);
    }

    #[test]
    fn test_activation_stats() {
        let time = WallClockTime::new();
        let mut stats = ActivationStats::with_time(&time);

        assert_eq!(stats.invocation_count, 0);
        assert_eq!(stats.error_count, 0);

        stats.record_invocation_with_time(10, false, &time);
        stats.record_invocation_with_time(20, true, &time);

        assert_eq!(stats.invocation_count, 2);
        assert_eq!(stats.error_count, 1);
        assert_eq!(stats.total_processing_time_ms, 30);
        assert_eq!(stats.average_processing_time(), Duration::from_millis(15));
    }

    // Actor that uses KV storage for additional data beyond its state
    #[derive(Debug, Default, Clone, Serialize, serde::Deserialize)]
    struct KVActorState {
        initialized: bool,
    }

    struct KVTestActor;

    #[async_trait]
    impl Actor for KVTestActor {
        type State = KVActorState;

        async fn invoke(
            &self,
            ctx: &mut ActorContext<Self::State>,
            operation: &str,
            payload: Bytes,
        ) -> Result<Bytes> {
            match operation {
                "store" => {
                    // Store a value using KV operations
                    let key = b"user_data";
                    ctx.kv_set(key, &payload).await?;
                    ctx.state.initialized = true;
                    Ok(Bytes::from("stored"))
                }
                "load" => {
                    // Load a value using KV operations
                    let key = b"user_data";
                    match ctx.kv_get(key).await? {
                        Some(data) => Ok(data),
                        None => Ok(Bytes::from("not_found")),
                    }
                }
                "delete" => {
                    let key = b"user_data";
                    ctx.kv_delete(key).await?;
                    Ok(Bytes::from("deleted"))
                }
                "exists" => {
                    let key = b"user_data";
                    let exists = ctx.kv_exists(key).await?;
                    Ok(Bytes::from(if exists { "true" } else { "false" }))
                }
                "list_keys" => {
                    // Store multiple keys with prefix
                    ctx.kv_set(b"item:1", b"first").await?;
                    ctx.kv_set(b"item:2", b"second").await?;
                    ctx.kv_set(b"other:1", b"other").await?;

                    let keys = ctx.kv_list_keys(b"item:").await?;
                    Ok(Bytes::from(keys.len().to_string()))
                }
                _ => Err(Error::InvalidOperation {
                    operation: operation.to_string(),
                }),
            }
        }
    }

    #[tokio::test]
    async fn test_actor_kv_operations() {
        let id = ActorId::new("test", "kv-actor-1").unwrap();
        let kv = create_kv();

        let mut active = ActiveActor::activate(id, KVTestActor, kv).await.unwrap();

        // Store data
        let result = active
            .process_invocation("store", Bytes::from("hello world"))
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("stored"));

        // Check exists
        let result = active
            .process_invocation("exists", Bytes::new())
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("true"));

        // Load data
        let result = active
            .process_invocation("load", Bytes::new())
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("hello world"));

        // Delete data
        let result = active
            .process_invocation("delete", Bytes::new())
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("deleted"));

        // Check not exists
        let result = active
            .process_invocation("exists", Bytes::new())
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("false"));

        // Load returns not_found
        let result = active
            .process_invocation("load", Bytes::new())
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("not_found"));
    }

    #[tokio::test]
    async fn test_actor_kv_persistence() {
        let id = ActorId::new("test", "kv-actor-2").unwrap();
        let kv = create_kv();

        // First activation: store data and deactivate
        {
            let mut active = ActiveActor::activate(id.clone(), KVTestActor, kv.clone())
                .await
                .unwrap();
            active
                .process_invocation("store", Bytes::from("persistent data"))
                .await
                .unwrap();
            active.deactivate().await.unwrap();
        }

        // Second activation: data should still be there (KV is separate from state)
        {
            let mut active = ActiveActor::activate(id, KVTestActor, kv).await.unwrap();
            let result = active
                .process_invocation("load", Bytes::new())
                .await
                .unwrap();
            assert_eq!(result, Bytes::from("persistent data"));
        }
    }

    #[tokio::test]
    async fn test_actor_kv_list_keys() {
        let id = ActorId::new("test", "kv-actor-3").unwrap();
        let kv = create_kv();

        let mut active = ActiveActor::activate(id, KVTestActor, kv).await.unwrap();

        // list_keys stores 3 keys: item:1, item:2, other:1
        // and returns count of keys with prefix "item:"
        let result = active
            .process_invocation("list_keys", Bytes::new())
            .await
            .unwrap();
        assert_eq!(result, Bytes::from("2")); // item:1 and item:2
    }
}
