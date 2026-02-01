//! Actor abstractions for Kelpie
//!
//! TigerStyle: Explicit types, assertions, bounded operations.

use crate::constants::*;
use crate::error::{Error, Result};
use async_trait::async_trait;
use bytes::Bytes;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::fmt;
use std::hash::Hash;

// =============================================================================
// ActorId
// =============================================================================

/// Unique identifier for an actor
///
/// Actor IDs consist of a namespace and an id, providing logical grouping
/// and unique identification within the cluster.
///
/// # TigerStyle
/// - Explicit validation on construction
/// - Immutable after creation
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActorId {
    namespace: String,
    id: String,
}

impl ActorId {
    /// Create a new ActorId with validation
    ///
    /// # Errors
    /// Returns error if namespace or id exceeds length limits or contains invalid characters.
    pub fn new(namespace: impl Into<String>, id: impl Into<String>) -> Result<Self> {
        let namespace = namespace.into();
        let id = id.into();

        // TigerStyle: Explicit validation
        debug_assert!(!namespace.is_empty(), "namespace must not be empty");
        debug_assert!(!id.is_empty(), "id must not be empty");

        if namespace.len() > ACTOR_NAMESPACE_LENGTH_BYTES_MAX {
            return Err(Error::InvalidActorId {
                id: format!("{}:{}", namespace, id),
                reason: format!(
                    "namespace length {} exceeds limit {}",
                    namespace.len(),
                    ACTOR_NAMESPACE_LENGTH_BYTES_MAX
                ),
            });
        }

        if id.len() > ACTOR_ID_LENGTH_BYTES_MAX {
            return Err(Error::ActorIdTooLong {
                length: id.len(),
                limit: ACTOR_ID_LENGTH_BYTES_MAX,
            });
        }

        // Validate characters (alphanumeric, dash, underscore, dot)
        let valid_chars = |s: &str| {
            s.chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
        };

        if !valid_chars(&namespace) {
            return Err(Error::InvalidActorId {
                id: format!("{}:{}", namespace, id),
                reason: "namespace contains invalid characters".into(),
            });
        }

        if !valid_chars(&id) {
            return Err(Error::InvalidActorId {
                id: format!("{}:{}", namespace, id),
                reason: "id contains invalid characters".into(),
            });
        }

        Ok(Self { namespace, id })
    }

    /// Create ActorId without validation (for internal use only)
    ///
    /// # Safety
    /// Caller must ensure namespace and id are valid.
    #[doc(hidden)]
    pub fn new_unchecked(namespace: String, id: String) -> Self {
        debug_assert!(namespace.len() <= ACTOR_NAMESPACE_LENGTH_BYTES_MAX);
        debug_assert!(id.len() <= ACTOR_ID_LENGTH_BYTES_MAX);
        Self { namespace, id }
    }

    /// Get the namespace
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Get the id
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Get the full qualified name (namespace:id)
    pub fn qualified_name(&self) -> String {
        format!("{}:{}", self.namespace, self.id)
    }

    /// Convert to bytes for storage keys
    pub fn to_key_bytes(&self) -> Vec<u8> {
        let qualified = self.qualified_name();
        debug_assert!(
            qualified.len() <= ACTOR_ID_LENGTH_BYTES_MAX + ACTOR_NAMESPACE_LENGTH_BYTES_MAX + 1
        );
        qualified.into_bytes()
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.namespace, self.id)
    }
}

// =============================================================================
// ActorRef
// =============================================================================

/// Reference to an actor (location-transparent)
///
/// ActorRef provides location transparency - callers only need the actor's ID
/// to communicate with it. Routing and placement are handled internally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorRef {
    /// The actor's unique identifier
    pub id: ActorId,
    // Note: Internal routing hints may be added in future phases
    // but are hidden from the public API
}

impl ActorRef {
    /// Create a new ActorRef from an ActorId
    pub fn new(id: ActorId) -> Self {
        Self { id }
    }

    /// Create a new ActorRef with the given namespace and id
    pub fn from_parts(namespace: impl Into<String>, id: impl Into<String>) -> Result<Self> {
        Ok(Self {
            id: ActorId::new(namespace, id)?,
        })
    }
}

impl From<ActorId> for ActorRef {
    fn from(id: ActorId) -> Self {
        Self::new(id)
    }
}

// =============================================================================
// Actor Trait
// =============================================================================

/// Actor trait - implement to create actors
///
/// # TigerStyle
/// - Single-threaded execution guarantee (no concurrent invocations)
/// - Explicit lifecycle hooks (on_activate, on_deactivate)
/// - State is serializable for persistence
#[async_trait]
pub trait Actor: Send + Sync + 'static {
    /// The actor's state type
    ///
    /// Must be serializable for persistence and default-constructible
    /// for fresh actor activation.
    type State: Serialize + DeserializeOwned + Default + Send + Sync;

    /// Handle an invocation
    ///
    /// This is called for each message sent to the actor. Execution is
    /// guaranteed to be single-threaded - no concurrent invocations.
    ///
    /// # Arguments
    /// * `ctx` - Actor context providing state access and runtime operations
    /// * `operation` - The operation name
    /// * `payload` - The message payload as bytes
    ///
    /// # Returns
    /// Response payload as bytes, or an error
    async fn invoke(
        &self,
        ctx: &mut ActorContext<Self::State>,
        operation: &str,
        payload: Bytes,
    ) -> Result<Bytes>;

    /// Called when the actor is activated
    ///
    /// Override to perform initialization logic when an actor is
    /// loaded into memory.
    async fn on_activate(&self, _ctx: &mut ActorContext<Self::State>) -> Result<()> {
        Ok(())
    }

    /// Called before the actor is deactivated
    ///
    /// Override to perform cleanup logic before an actor is
    /// removed from memory.
    async fn on_deactivate(&self, _ctx: &mut ActorContext<Self::State>) -> Result<()> {
        Ok(())
    }
}

// =============================================================================
// ContextKV Trait
// =============================================================================

/// KV store interface available to actors through their context
///
/// This trait defines the storage operations available to actors.
/// It is automatically scoped to the actor's ID - keys are namespaced
/// per-actor and cannot collide with other actors' data.
///
/// # TigerStyle
/// - All operations are async and fallible
/// - Keys are bounded by ACTOR_KV_KEY_SIZE_BYTES_MAX
/// - Values are bounded by ACTOR_STATE_SIZE_BYTES_MAX
#[async_trait]
pub trait ContextKV: Send + Sync {
    /// Get a value by key
    ///
    /// Returns None if the key does not exist.
    async fn get(&self, key: &[u8]) -> Result<Option<Bytes>>;

    /// Set a key-value pair
    ///
    /// Overwrites any existing value for the key.
    async fn set(&self, key: &[u8], value: &[u8]) -> Result<()>;

    /// Delete a key
    ///
    /// No-op if the key does not exist.
    async fn delete(&self, key: &[u8]) -> Result<()>;

    /// Check if a key exists
    async fn exists(&self, key: &[u8]) -> Result<bool> {
        Ok(self.get(key).await?.is_some())
    }

    /// List keys with a given prefix
    ///
    /// Returns all keys that start with the given prefix.
    async fn list_keys(&self, prefix: &[u8]) -> Result<Vec<Vec<u8>>>;
}

/// No-op KV implementation for contexts without storage access
///
/// Used when an actor doesn't need KV access or for testing.
pub struct NoOpKV;

// =============================================================================
// BufferingContextKV
// =============================================================================

/// A KV operation that was buffered
#[derive(Debug, Clone)]
pub enum BufferedKVOp {
    Set { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

/// A ContextKV wrapper that buffers writes for transactional commit
///
/// This is used to make actor KV operations atomic with state persistence.
/// All set/delete operations are buffered and can be applied to a transaction
/// after the actor's invoke() completes.
///
/// Supports read-your-writes: get() returns buffered values if present.
pub struct BufferingContextKV {
    /// The underlying KV for reads not in the buffer
    underlying: Box<dyn ContextKV>,
    /// Buffered operations (set/delete)
    buffer: std::sync::RwLock<Vec<BufferedKVOp>>,
    /// Local cache of buffered values for read-your-writes
    /// Key -> Some(value) for sets, None for deletes
    cache: std::sync::RwLock<std::collections::HashMap<Vec<u8>, Option<Vec<u8>>>>,
}

impl BufferingContextKV {
    /// Create a new buffering KV wrapper
    pub fn new(underlying: Box<dyn ContextKV>) -> Self {
        Self {
            underlying,
            buffer: std::sync::RwLock::new(Vec::new()),
            cache: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Drain all buffered operations
    ///
    /// Returns the operations in order and clears the buffer.
    /// Used by the runtime to apply operations to a transaction.
    pub fn drain_buffer(&self) -> Vec<BufferedKVOp> {
        self.buffer.write().unwrap().drain(..).collect()
    }

    /// Check if there are any buffered operations
    pub fn has_buffered_ops(&self) -> bool {
        !self.buffer.read().unwrap().is_empty()
    }

    /// Take ownership of the underlying KV
    ///
    /// Consumes this wrapper and returns the underlying ContextKV.
    pub fn into_inner(self) -> Box<dyn ContextKV> {
        self.underlying
    }
}

/// Wrapper to use `Arc`<`BufferingContextKV`> as `Box`<`dyn ContextKV`>
///
/// This allows sharing a BufferingContextKV between the context and the runtime
/// so the runtime can drain the buffer after invoke() completes.
pub struct ArcContextKV(pub std::sync::Arc<BufferingContextKV>);

#[async_trait]
impl ContextKV for ArcContextKV {
    async fn get(&self, key: &[u8]) -> Result<Option<Bytes>> {
        self.0.get(key).await
    }

    async fn set(&self, key: &[u8], value: &[u8]) -> Result<()> {
        self.0.set(key, value).await
    }

    async fn delete(&self, key: &[u8]) -> Result<()> {
        self.0.delete(key).await
    }

    async fn exists(&self, key: &[u8]) -> Result<bool> {
        self.0.exists(key).await
    }

    async fn list_keys(&self, prefix: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.0.list_keys(prefix).await
    }
}

#[async_trait]
impl ContextKV for BufferingContextKV {
    async fn get(&self, key: &[u8]) -> Result<Option<Bytes>> {
        // Check cache first (read-your-writes)
        if let Some(cached) = self.cache.read().unwrap().get(key).cloned() {
            return Ok(cached.map(|v| Bytes::copy_from_slice(&v)));
        }
        // Fall back to underlying storage
        self.underlying.get(key).await
    }

    async fn set(&self, key: &[u8], value: &[u8]) -> Result<()> {
        // Buffer the operation
        self.buffer.write().unwrap().push(BufferedKVOp::Set {
            key: key.to_vec(),
            value: value.to_vec(),
        });
        // Update cache for read-your-writes
        self.cache
            .write()
            .unwrap()
            .insert(key.to_vec(), Some(value.to_vec()));
        Ok(())
    }

    async fn delete(&self, key: &[u8]) -> Result<()> {
        // Buffer the operation
        self.buffer
            .write()
            .unwrap()
            .push(BufferedKVOp::Delete { key: key.to_vec() });
        // Update cache (None means deleted)
        self.cache.write().unwrap().insert(key.to_vec(), None);
        Ok(())
    }

    async fn exists(&self, key: &[u8]) -> Result<bool> {
        // Check cache first
        if let Some(cached) = self.cache.read().unwrap().get(key).cloned() {
            return Ok(cached.is_some());
        }
        // Fall back to underlying storage
        self.underlying.exists(key).await
    }

    async fn list_keys(&self, prefix: &[u8]) -> Result<Vec<Vec<u8>>> {
        // Get keys from underlying storage
        let mut keys = self.underlying.list_keys(prefix).await?;

        // Add any new keys from buffer that match prefix
        let cache = self.cache.read().unwrap();
        for (key, value) in cache.iter() {
            if key.starts_with(prefix) {
                if value.is_some() && !keys.contains(key) {
                    keys.push(key.clone());
                } else if value.is_none() {
                    // Remove deleted keys
                    keys.retain(|k| k != key);
                }
            }
        }

        Ok(keys)
    }
}

#[async_trait]
impl ContextKV for NoOpKV {
    async fn get(&self, _key: &[u8]) -> Result<Option<Bytes>> {
        Ok(None)
    }

    async fn set(&self, _key: &[u8], _value: &[u8]) -> Result<()> {
        Ok(())
    }

    async fn delete(&self, _key: &[u8]) -> Result<()> {
        Ok(())
    }

    async fn list_keys(&self, _prefix: &[u8]) -> Result<Vec<Vec<u8>>> {
        Ok(Vec::new())
    }
}

// =============================================================================
// ActorContext
// =============================================================================

/// Context provided to an actor during invocation
///
/// Provides access to:
/// - Actor's persistent state
/// - Per-actor KV store (via kv() method)
/// - Ability to invoke other actors (future phase)
pub struct ActorContext<S> {
    /// The actor's unique identifier
    pub id: ActorId,

    /// The actor's in-memory state
    pub state: S,

    /// Per-actor KV store
    kv: Box<dyn ContextKV>,
}

impl<S> ActorContext<S>
where
    S: Serialize + DeserializeOwned + Default + Send + Sync,
{
    /// Create a new ActorContext with KV access
    pub fn new(id: ActorId, state: S, kv: Box<dyn ContextKV>) -> Self {
        Self { id, state, kv }
    }

    /// Create a new ActorContext with default state and KV access
    pub fn with_default_state(id: ActorId, kv: Box<dyn ContextKV>) -> Self {
        Self {
            id,
            state: S::default(),
            kv,
        }
    }

    /// Create a new ActorContext with default state and no KV access
    ///
    /// Useful for testing actors that don't use KV operations.
    pub fn with_default_state_no_kv(id: ActorId) -> Self {
        Self {
            id,
            state: S::default(),
            kv: Box::new(NoOpKV),
        }
    }

    /// Get a value from the actor's KV store
    ///
    /// Keys are automatically scoped to this actor - they cannot
    /// collide with other actors' data.
    pub async fn kv_get(&self, key: &[u8]) -> Result<Option<Bytes>> {
        debug_assert!(
            key.len() <= crate::constants::ACTOR_KV_KEY_SIZE_BYTES_MAX,
            "key exceeds maximum size"
        );
        self.kv.get(key).await
    }

    /// Set a value in the actor's KV store
    ///
    /// Keys are automatically scoped to this actor.
    pub async fn kv_set(&self, key: &[u8], value: &[u8]) -> Result<()> {
        debug_assert!(
            key.len() <= crate::constants::ACTOR_KV_KEY_SIZE_BYTES_MAX,
            "key exceeds maximum size"
        );
        debug_assert!(
            value.len() <= crate::constants::ACTOR_STATE_SIZE_BYTES_MAX,
            "value exceeds maximum size"
        );
        self.kv.set(key, value).await
    }

    /// Delete a value from the actor's KV store
    pub async fn kv_delete(&self, key: &[u8]) -> Result<()> {
        debug_assert!(
            key.len() <= crate::constants::ACTOR_KV_KEY_SIZE_BYTES_MAX,
            "key exceeds maximum size"
        );
        self.kv.delete(key).await
    }

    /// Check if a key exists in the actor's KV store
    pub async fn kv_exists(&self, key: &[u8]) -> Result<bool> {
        debug_assert!(
            key.len() <= crate::constants::ACTOR_KV_KEY_SIZE_BYTES_MAX,
            "key exceeds maximum size"
        );
        self.kv.exists(key).await
    }

    /// List keys with a given prefix in the actor's KV store
    pub async fn kv_list_keys(&self, prefix: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.kv.list_keys(prefix).await
    }

    /// Swap the KV implementation
    ///
    /// Used by the runtime to inject a buffering KV for transactional operations.
    /// Returns the previous KV implementation.
    pub fn swap_kv(&mut self, new_kv: Box<dyn ContextKV>) -> Box<dyn ContextKV> {
        std::mem::replace(&mut self.kv, new_kv)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_actor_id_valid() {
        let id = ActorId::new("agents", "agent-123").unwrap();
        assert_eq!(id.namespace(), "agents");
        assert_eq!(id.id(), "agent-123");
        assert_eq!(id.qualified_name(), "agents:agent-123");
    }

    #[test]
    fn test_actor_id_invalid_chars() {
        let result = ActorId::new("agents", "agent/123");
        assert!(result.is_err());
    }

    #[test]
    fn test_actor_id_too_long() {
        let long_id = "a".repeat(ACTOR_ID_LENGTH_BYTES_MAX + 1);
        let result = ActorId::new("agents", long_id);
        assert!(matches!(result, Err(Error::ActorIdTooLong { .. })));
    }

    #[test]
    fn test_actor_ref_from_parts() {
        let actor_ref = ActorRef::from_parts("agents", "agent-456").unwrap();
        assert_eq!(actor_ref.id.qualified_name(), "agents:agent-456");
    }

    #[test]
    fn test_actor_id_display() {
        let id = ActorId::new("ns", "id").unwrap();
        assert_eq!(format!("{}", id), "ns:id");
    }
}
