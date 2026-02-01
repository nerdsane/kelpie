//! Write-Ahead Log (WAL) for operation durability
//!
//! TigerStyle: Explicit types, clear contracts, atomicity guarantees.
//!
//! # Overview
//!
//! The WAL ensures operations are durable before returning success to clients.
//! On crash recovery, pending entries are replayed to restore consistency.
//!
//! # Flow
//!
//! ```text
//! 1. WAL.append(operation) → entry_id (durable)
//! 2. Execute operation
//! 3. WAL.complete(entry_id) OR WAL.fail(entry_id)
//! 4. Return to client
//!
//! On recovery: replay all pending entries
//! ```

use async_trait::async_trait;
use bytes::Bytes;
use kelpie_core::actor::ActorId;
use kelpie_core::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

// =============================================================================
// Constants (TigerStyle: explicit limits with units)
// =============================================================================

/// Maximum age of completed entries before cleanup (24 hours)
pub const WAL_ENTRY_RETENTION_MS: u64 = 24 * 60 * 60 * 1000;

/// Maximum number of pending entries before warning
pub const WAL_PENDING_ENTRIES_WARN_THRESHOLD: usize = 1000;

/// Maximum retries for WAL counter increment (handles transaction conflicts)
const WAL_COUNTER_MAX_RETRIES: usize = 5;

// =============================================================================
// Types
// =============================================================================

/// WAL entry ID (monotonically increasing)
pub type WalEntryId = u64;

/// Operation type for WAL entry
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WalOperation {
    /// Create a new agent
    CreateAgent,
    /// Update an existing agent
    UpdateAgent,
    /// Send a message to an agent
    SendMessage,
    /// Delete an agent
    DeleteAgent,
    /// Update a memory block
    UpdateBlock,
    /// Generic operation (for extensibility)
    Custom(String),
}

impl std::fmt::Display for WalOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WalOperation::CreateAgent => write!(f, "CreateAgent"),
            WalOperation::UpdateAgent => write!(f, "UpdateAgent"),
            WalOperation::SendMessage => write!(f, "SendMessage"),
            WalOperation::DeleteAgent => write!(f, "DeleteAgent"),
            WalOperation::UpdateBlock => write!(f, "UpdateBlock"),
            WalOperation::Custom(name) => write!(f, "Custom({})", name),
        }
    }
}

/// Status of a WAL entry
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WalStatus {
    /// Operation is pending (not yet completed or failed)
    Pending,
    /// Operation completed successfully
    Complete,
    /// Operation failed (won't be replayed)
    Failed { error: String },
}

/// A single WAL entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalEntry {
    /// Unique, monotonically increasing ID
    pub id: WalEntryId,
    /// Type of operation
    pub operation: WalOperation,
    /// Target actor ID
    pub actor_id: String,
    /// Serialized request payload
    pub payload: Vec<u8>,
    /// Current status
    pub status: WalStatus,
    /// Creation timestamp (milliseconds since epoch)
    pub created_at_ms: u64,
    /// Completion timestamp (if completed/failed)
    pub completed_at_ms: Option<u64>,
    /// Optional idempotency key for deduplication (e.g., message UUID)
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

impl WalEntry {
    /// Create a new pending WAL entry
    pub fn new(
        id: WalEntryId,
        operation: WalOperation,
        actor_id: &ActorId,
        payload: Bytes,
        now_ms: u64,
    ) -> Self {
        Self {
            id,
            operation,
            actor_id: actor_id.to_string(),
            payload: payload.to_vec(),
            status: WalStatus::Pending,
            created_at_ms: now_ms,
            completed_at_ms: None,
            idempotency_key: None,
        }
    }

    /// Create a new pending WAL entry with idempotency key
    pub fn new_with_idempotency_key(
        id: WalEntryId,
        operation: WalOperation,
        actor_id: &ActorId,
        payload: Bytes,
        now_ms: u64,
        idempotency_key: String,
    ) -> Self {
        Self {
            id,
            operation,
            actor_id: actor_id.to_string(),
            payload: payload.to_vec(),
            status: WalStatus::Pending,
            created_at_ms: now_ms,
            completed_at_ms: None,
            idempotency_key: Some(idempotency_key),
        }
    }

    /// Check if entry is pending
    pub fn is_pending(&self) -> bool {
        matches!(self.status, WalStatus::Pending)
    }

    /// Get payload as Bytes
    pub fn payload_bytes(&self) -> Bytes {
        Bytes::from(self.payload.clone())
    }
}

// =============================================================================
// Trait
// =============================================================================

/// Write-Ahead Log trait for durable operation logging
#[async_trait]
pub trait WriteAheadLog: Send + Sync {
    /// Durably append a new entry to the WAL
    ///
    /// Returns the entry ID on success. The entry is guaranteed to be
    /// durable when this method returns.
    ///
    /// # Arguments
    /// * `operation` - Type of operation
    /// * `actor_id` - Target actor
    /// * `payload` - Serialized request data
    /// * `now_ms` - Current timestamp in milliseconds
    async fn append(
        &self,
        operation: WalOperation,
        actor_id: &ActorId,
        payload: Bytes,
        now_ms: u64,
    ) -> Result<WalEntryId>;

    /// Durably append a new entry with idempotency key
    ///
    /// If an entry with the same idempotency key already exists (completed or pending),
    /// returns the existing entry ID without creating a duplicate.
    ///
    /// # Arguments
    /// * `operation` - Type of operation
    /// * `actor_id` - Target actor
    /// * `payload` - Serialized request data
    /// * `now_ms` - Current timestamp in milliseconds
    /// * `idempotency_key` - Unique key for deduplication
    async fn append_with_idempotency(
        &self,
        operation: WalOperation,
        actor_id: &ActorId,
        payload: Bytes,
        now_ms: u64,
        idempotency_key: String,
    ) -> Result<(WalEntryId, bool)>; // Returns (entry_id, is_new)

    /// Mark an entry as successfully completed
    ///
    /// The entry will not be replayed on recovery.
    async fn complete(&self, entry_id: WalEntryId, now_ms: u64) -> Result<()>;

    /// Mark an entry as failed
    ///
    /// Failed entries are not replayed on recovery.
    async fn fail(&self, entry_id: WalEntryId, error: &str, now_ms: u64) -> Result<()>;

    /// Get all pending entries for replay
    ///
    /// Returns entries in ID order (oldest first).
    async fn pending_entries(&self) -> Result<Vec<WalEntry>>;

    /// Get a specific entry by ID
    async fn get(&self, entry_id: WalEntryId) -> Result<Option<WalEntry>>;

    /// Find entry by idempotency key
    async fn find_by_idempotency_key(&self, key: &str) -> Result<Option<WalEntry>>;

    /// Cleanup old completed/failed entries
    ///
    /// Removes entries older than `older_than_ms` that are not pending.
    /// Returns the number of entries removed.
    async fn cleanup(&self, older_than_ms: u64) -> Result<u64>;

    /// Get count of pending entries
    async fn pending_count(&self) -> Result<usize>;
}

// =============================================================================
// In-Memory Implementation (for testing and DST)
// =============================================================================

/// In-memory WAL implementation for testing
///
/// TigerStyle: This implementation is for testing only.
/// Production should use KvWal or a file-based implementation.
#[derive(Debug)]
pub struct MemoryWal {
    entries: RwLock<HashMap<WalEntryId, WalEntry>>,
    next_id: AtomicU64,
}

impl MemoryWal {
    /// Create a new in-memory WAL
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Create a new in-memory WAL wrapped in Arc
    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

impl Default for MemoryWal {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WriteAheadLog for MemoryWal {
    async fn append(
        &self,
        operation: WalOperation,
        actor_id: &ActorId,
        payload: Bytes,
        now_ms: u64,
    ) -> Result<WalEntryId> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let entry = WalEntry::new(id, operation, actor_id, payload, now_ms);

        let mut entries = self.entries.write().await;
        entries.insert(id, entry);

        Ok(id)
    }

    async fn append_with_idempotency(
        &self,
        operation: WalOperation,
        actor_id: &ActorId,
        payload: Bytes,
        now_ms: u64,
        idempotency_key: String,
    ) -> Result<(WalEntryId, bool)> {
        let mut entries = self.entries.write().await;

        // Check for existing entry with same idempotency key
        for entry in entries.values() {
            if entry.idempotency_key.as_deref() == Some(&idempotency_key) {
                return Ok((entry.id, false)); // Already exists
            }
        }

        // Create new entry
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let entry = WalEntry::new_with_idempotency_key(
            id,
            operation,
            actor_id,
            payload,
            now_ms,
            idempotency_key,
        );
        entries.insert(id, entry);

        Ok((id, true)) // New entry created
    }

    async fn complete(&self, entry_id: WalEntryId, now_ms: u64) -> Result<()> {
        let mut entries = self.entries.write().await;

        if let Some(entry) = entries.get_mut(&entry_id) {
            assert!(
                entry.is_pending(),
                "Cannot complete non-pending entry {}",
                entry_id
            );
            entry.status = WalStatus::Complete;
            entry.completed_at_ms = Some(now_ms);
            Ok(())
        } else {
            Err(kelpie_core::Error::Internal {
                message: format!("WAL entry {} not found", entry_id),
            })
        }
    }

    async fn fail(&self, entry_id: WalEntryId, error: &str, now_ms: u64) -> Result<()> {
        let mut entries = self.entries.write().await;

        if let Some(entry) = entries.get_mut(&entry_id) {
            assert!(
                entry.is_pending(),
                "Cannot fail non-pending entry {}",
                entry_id
            );
            entry.status = WalStatus::Failed {
                error: error.to_string(),
            };
            entry.completed_at_ms = Some(now_ms);
            Ok(())
        } else {
            Err(kelpie_core::Error::Internal {
                message: format!("WAL entry {} not found", entry_id),
            })
        }
    }

    async fn pending_entries(&self) -> Result<Vec<WalEntry>> {
        let entries = self.entries.read().await;

        let mut pending: Vec<_> = entries
            .values()
            .filter(|e| e.is_pending())
            .cloned()
            .collect();

        // Sort by ID (oldest first)
        pending.sort_by_key(|e| e.id);

        Ok(pending)
    }

    async fn get(&self, entry_id: WalEntryId) -> Result<Option<WalEntry>> {
        let entries = self.entries.read().await;
        Ok(entries.get(&entry_id).cloned())
    }

    async fn find_by_idempotency_key(&self, key: &str) -> Result<Option<WalEntry>> {
        let entries = self.entries.read().await;
        for entry in entries.values() {
            if entry.idempotency_key.as_deref() == Some(key) {
                return Ok(Some(entry.clone()));
            }
        }
        Ok(None)
    }

    async fn cleanup(&self, older_than_ms: u64) -> Result<u64> {
        let mut entries = self.entries.write().await;

        let to_remove: Vec<_> = entries
            .iter()
            .filter(|(_, e)| {
                !e.is_pending()
                    && e.completed_at_ms
                        .map(|t| t < older_than_ms)
                        .unwrap_or(false)
            })
            .map(|(id, _)| *id)
            .collect();

        let count = to_remove.len() as u64;
        for id in to_remove {
            entries.remove(&id);
        }

        Ok(count)
    }

    async fn pending_count(&self) -> Result<usize> {
        let entries = self.entries.read().await;
        Ok(entries.values().filter(|e| e.is_pending()).count())
    }
}

// =============================================================================
// KV-Backed Implementation
// =============================================================================

use crate::ActorKV;

/// WAL key prefix
const WAL_KEY_PREFIX: &[u8] = b"wal:";
/// WAL counter key
const WAL_COUNTER_KEY: &[u8] = b"wal:_counter";
/// System namespace for WAL storage
const WAL_SYSTEM_NAMESPACE: &str = "_system";
/// System actor ID for WAL storage
const WAL_SYSTEM_ID: &str = "wal";

/// KV-backed WAL implementation
///
/// Stores WAL entries in the same KV store as actor state.
/// Uses atomic transactions for durability.
/// Uses a special "_system:wal" actor ID to isolate WAL data.
pub struct KvWal {
    kv: Arc<dyn ActorKV>,
    /// System actor ID for WAL storage
    system_actor_id: ActorId,
}

impl KvWal {
    /// Create a new KV-backed WAL
    pub fn new(kv: Arc<dyn ActorKV>) -> Self {
        // Use a system actor ID for WAL storage
        let system_actor_id = ActorId::new(WAL_SYSTEM_NAMESPACE, WAL_SYSTEM_ID)
            .expect("WAL system actor ID must be valid");
        Self {
            kv,
            system_actor_id,
        }
    }

    /// Create a new KV-backed WAL wrapped in Arc
    pub fn new_arc(kv: Arc<dyn ActorKV>) -> Arc<Self> {
        Arc::new(Self::new(kv))
    }

    /// Generate the key for a WAL entry
    fn entry_key(id: WalEntryId) -> Vec<u8> {
        let mut key = WAL_KEY_PREFIX.to_vec();
        key.extend_from_slice(&id.to_be_bytes());
        key
    }

    /// Get the next entry ID atomically with retry on conflict
    async fn next_id(&self) -> Result<WalEntryId> {
        let mut last_error = None;

        for _attempt in 0..WAL_COUNTER_MAX_RETRIES {
            let mut txn = self.kv.begin_transaction(&self.system_actor_id).await?;

            // Read current counter
            let current =
                match txn.get(WAL_COUNTER_KEY).await? {
                    Some(bytes) => {
                        let arr: [u8; 8] = bytes.as_ref().try_into().map_err(|_| {
                            kelpie_core::Error::Internal {
                                message: "Invalid WAL counter".to_string(),
                            }
                        })?;
                        u64::from_be_bytes(arr)
                    }
                    None => 0,
                };

            let next = current + 1;

            // Write incremented counter
            txn.set(WAL_COUNTER_KEY, &next.to_be_bytes()).await?;

            match txn.commit().await {
                Ok(_) => return Ok(next),
                Err(e) => {
                    // Transaction conflict, retry
                    last_error = Some(e);
                    continue;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| kelpie_core::Error::Internal {
            message: "WAL counter increment failed after max retries".to_string(),
        }))
    }
}

#[async_trait]
impl WriteAheadLog for KvWal {
    async fn append(
        &self,
        operation: WalOperation,
        actor_id: &ActorId,
        payload: Bytes,
        now_ms: u64,
    ) -> Result<WalEntryId> {
        let id = self.next_id().await?;
        let entry = WalEntry::new(id, operation, actor_id, payload, now_ms);

        // Serialize and store
        let key = Self::entry_key(id);
        let value =
            serde_json::to_vec(&entry).map_err(|e| kelpie_core::Error::SerializationFailed {
                reason: e.to_string(),
            })?;

        let mut txn = self.kv.begin_transaction(&self.system_actor_id).await?;
        txn.set(&key, &value).await?;
        txn.commit().await?;

        Ok(id)
    }

    async fn append_with_idempotency(
        &self,
        operation: WalOperation,
        actor_id: &ActorId,
        payload: Bytes,
        now_ms: u64,
        idempotency_key: String,
    ) -> Result<(WalEntryId, bool)> {
        // First, check if entry with this idempotency key already exists
        if let Some(existing) = self.find_by_idempotency_key(&idempotency_key).await? {
            return Ok((existing.id, false)); // Already exists
        }

        // Create new entry with idempotency key
        let id = self.next_id().await?;
        let entry = WalEntry::new_with_idempotency_key(
            id,
            operation,
            actor_id,
            payload,
            now_ms,
            idempotency_key,
        );

        // Serialize and store
        let key = Self::entry_key(id);
        let value =
            serde_json::to_vec(&entry).map_err(|e| kelpie_core::Error::SerializationFailed {
                reason: e.to_string(),
            })?;

        let mut txn = self.kv.begin_transaction(&self.system_actor_id).await?;
        txn.set(&key, &value).await?;
        txn.commit().await?;

        Ok((id, true)) // New entry created
    }

    async fn find_by_idempotency_key(&self, key: &str) -> Result<Option<WalEntry>> {
        // Scan all WAL entries to find one with matching idempotency key
        let entries = self
            .kv
            .list_keys(&self.system_actor_id, WAL_KEY_PREFIX)
            .await?;

        for entry_key in entries {
            // Skip the counter key
            if entry_key == WAL_COUNTER_KEY {
                continue;
            }

            if let Some(bytes) = self.kv.get(&self.system_actor_id, &entry_key).await? {
                if let Ok(entry) = serde_json::from_slice::<WalEntry>(&bytes) {
                    if entry.idempotency_key.as_deref() == Some(key) {
                        return Ok(Some(entry));
                    }
                }
            }
        }

        Ok(None)
    }

    async fn complete(&self, entry_id: WalEntryId, now_ms: u64) -> Result<()> {
        let key = Self::entry_key(entry_id);

        let mut txn = self.kv.begin_transaction(&self.system_actor_id).await?;

        // Read current entry
        let bytes = txn
            .get(&key)
            .await?
            .ok_or_else(|| kelpie_core::Error::Internal {
                message: format!("WAL entry {} not found", entry_id),
            })?;

        let mut entry: WalEntry = serde_json::from_slice(&bytes).map_err(|e| {
            kelpie_core::Error::DeserializationFailed {
                reason: e.to_string(),
            }
        })?;

        assert!(
            entry.is_pending(),
            "Cannot complete non-pending entry {}",
            entry_id
        );

        entry.status = WalStatus::Complete;
        entry.completed_at_ms = Some(now_ms);

        // Write updated entry
        let value =
            serde_json::to_vec(&entry).map_err(|e| kelpie_core::Error::SerializationFailed {
                reason: e.to_string(),
            })?;

        txn.set(&key, &value).await?;
        txn.commit().await?;

        Ok(())
    }

    async fn fail(&self, entry_id: WalEntryId, error: &str, now_ms: u64) -> Result<()> {
        let key = Self::entry_key(entry_id);

        let mut txn = self.kv.begin_transaction(&self.system_actor_id).await?;

        // Read current entry
        let bytes = txn
            .get(&key)
            .await?
            .ok_or_else(|| kelpie_core::Error::Internal {
                message: format!("WAL entry {} not found", entry_id),
            })?;

        let mut entry: WalEntry = serde_json::from_slice(&bytes).map_err(|e| {
            kelpie_core::Error::DeserializationFailed {
                reason: e.to_string(),
            }
        })?;

        assert!(
            entry.is_pending(),
            "Cannot fail non-pending entry {}",
            entry_id
        );

        entry.status = WalStatus::Failed {
            error: error.to_string(),
        };
        entry.completed_at_ms = Some(now_ms);

        // Write updated entry
        let value =
            serde_json::to_vec(&entry).map_err(|e| kelpie_core::Error::SerializationFailed {
                reason: e.to_string(),
            })?;

        txn.set(&key, &value).await?;
        txn.commit().await?;

        Ok(())
    }

    async fn pending_entries(&self) -> Result<Vec<WalEntry>> {
        // Scan all WAL entries using system actor ID
        let entries = self
            .kv
            .list_keys(&self.system_actor_id, WAL_KEY_PREFIX)
            .await?;

        let mut pending = Vec::new();

        for key in entries {
            // Skip the counter key
            if key == WAL_COUNTER_KEY {
                continue;
            }

            if let Some(bytes) = self.kv.get(&self.system_actor_id, &key).await? {
                if let Ok(entry) = serde_json::from_slice::<WalEntry>(&bytes) {
                    if entry.is_pending() {
                        pending.push(entry);
                    }
                }
            }
        }

        // Sort by ID (oldest first)
        pending.sort_by_key(|e| e.id);

        Ok(pending)
    }

    async fn get(&self, entry_id: WalEntryId) -> Result<Option<WalEntry>> {
        let key = Self::entry_key(entry_id);

        match self.kv.get(&self.system_actor_id, &key).await? {
            Some(bytes) => {
                let entry = serde_json::from_slice(&bytes).map_err(|e| {
                    kelpie_core::Error::DeserializationFailed {
                        reason: e.to_string(),
                    }
                })?;
                Ok(Some(entry))
            }
            None => Ok(None),
        }
    }

    async fn cleanup(&self, older_than_ms: u64) -> Result<u64> {
        let entries = self
            .kv
            .list_keys(&self.system_actor_id, WAL_KEY_PREFIX)
            .await?;

        let mut count = 0u64;

        for key in entries {
            // Skip the counter key
            if key == WAL_COUNTER_KEY {
                continue;
            }

            if let Some(bytes) = self.kv.get(&self.system_actor_id, &key).await? {
                if let Ok(entry) = serde_json::from_slice::<WalEntry>(&bytes) {
                    if !entry.is_pending() {
                        if let Some(completed_at) = entry.completed_at_ms {
                            if completed_at < older_than_ms {
                                self.kv.delete(&self.system_actor_id, &key).await?;
                                count += 1;
                            }
                        }
                    }
                }
            }
        }

        Ok(count)
    }

    async fn pending_count(&self) -> Result<usize> {
        let pending = self.pending_entries().await?;
        Ok(pending.len())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryKV;

    fn test_actor_id() -> ActorId {
        ActorId::new("test", "agent-1").unwrap()
    }

    #[tokio::test]
    async fn test_memory_wal_append_and_complete() {
        let wal = MemoryWal::new();
        let actor_id = test_actor_id();
        let payload = Bytes::from("test payload");

        // Append
        let entry_id = wal
            .append(WalOperation::CreateAgent, &actor_id, payload.clone(), 1000)
            .await
            .unwrap();

        assert_eq!(entry_id, 1);

        // Verify pending
        let pending = wal.pending_entries().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, entry_id);
        assert!(pending[0].is_pending());

        // Complete
        wal.complete(entry_id, 2000).await.unwrap();

        // Verify no longer pending
        let pending = wal.pending_entries().await.unwrap();
        assert_eq!(pending.len(), 0);

        // Verify entry still exists but completed
        let entry = wal.get(entry_id).await.unwrap().unwrap();
        assert_eq!(entry.status, WalStatus::Complete);
        assert_eq!(entry.completed_at_ms, Some(2000));
    }

    #[tokio::test]
    async fn test_memory_wal_append_and_fail() {
        let wal = MemoryWal::new();
        let actor_id = test_actor_id();

        let entry_id = wal
            .append(
                WalOperation::SendMessage,
                &actor_id,
                Bytes::from("msg"),
                1000,
            )
            .await
            .unwrap();

        // Fail
        wal.fail(entry_id, "test error", 2000).await.unwrap();

        // Verify no longer pending
        let pending = wal.pending_entries().await.unwrap();
        assert_eq!(pending.len(), 0);

        // Verify entry is failed
        let entry = wal.get(entry_id).await.unwrap().unwrap();
        assert!(matches!(entry.status, WalStatus::Failed { .. }));
    }

    #[tokio::test]
    async fn test_memory_wal_pending_entries_ordered() {
        let wal = MemoryWal::new();
        let actor_id = test_actor_id();

        // Append multiple entries
        let id1 = wal
            .append(WalOperation::CreateAgent, &actor_id, Bytes::new(), 1000)
            .await
            .unwrap();
        let id2 = wal
            .append(WalOperation::UpdateAgent, &actor_id, Bytes::new(), 2000)
            .await
            .unwrap();
        let id3 = wal
            .append(WalOperation::SendMessage, &actor_id, Bytes::new(), 3000)
            .await
            .unwrap();

        // Complete middle one
        wal.complete(id2, 2500).await.unwrap();

        // Verify pending entries are ordered
        let pending = wal.pending_entries().await.unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].id, id1); // Oldest first
        assert_eq!(pending[1].id, id3);
    }

    #[tokio::test]
    async fn test_memory_wal_cleanup() {
        let wal = MemoryWal::new();
        let actor_id = test_actor_id();

        // Append and complete entries
        let id1 = wal
            .append(WalOperation::CreateAgent, &actor_id, Bytes::new(), 1000)
            .await
            .unwrap();
        let id2 = wal
            .append(WalOperation::UpdateAgent, &actor_id, Bytes::new(), 2000)
            .await
            .unwrap();
        let _id3 = wal
            .append(WalOperation::SendMessage, &actor_id, Bytes::new(), 3000)
            .await
            .unwrap();

        wal.complete(id1, 1500).await.unwrap();
        wal.complete(id2, 2500).await.unwrap();
        // id3 stays pending

        // Cleanup entries completed before 2000
        let removed = wal.cleanup(2000).await.unwrap();
        assert_eq!(removed, 1); // Only id1

        // Verify id1 is gone, id2 and id3 remain
        assert!(wal.get(id1).await.unwrap().is_none());
        assert!(wal.get(id2).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_kv_wal_basic() {
        let kv = Arc::new(MemoryKV::new());
        let wal = KvWal::new(kv);
        let actor_id = test_actor_id();

        // Append
        let entry_id = wal
            .append(
                WalOperation::CreateAgent,
                &actor_id,
                Bytes::from("payload"),
                1000,
            )
            .await
            .unwrap();

        assert_eq!(entry_id, 1);

        // Verify pending
        let pending = wal.pending_entries().await.unwrap();
        assert_eq!(pending.len(), 1);

        // Complete
        wal.complete(entry_id, 2000).await.unwrap();

        // Verify not pending
        let pending = wal.pending_entries().await.unwrap();
        assert_eq!(pending.len(), 0);
    }

    #[tokio::test]
    async fn test_pending_count() {
        let wal = MemoryWal::new();
        let actor_id = test_actor_id();

        assert_eq!(wal.pending_count().await.unwrap(), 0);

        let id1 = wal
            .append(WalOperation::CreateAgent, &actor_id, Bytes::new(), 1000)
            .await
            .unwrap();

        assert_eq!(wal.pending_count().await.unwrap(), 1);

        let _id2 = wal
            .append(WalOperation::UpdateAgent, &actor_id, Bytes::new(), 2000)
            .await
            .unwrap();

        assert_eq!(wal.pending_count().await.unwrap(), 2);

        wal.complete(id1, 1500).await.unwrap();

        assert_eq!(wal.pending_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_memory_wal_idempotency() {
        let wal = MemoryWal::new();
        let actor_id = test_actor_id();
        let idempotency_key = "msg-uuid-12345".to_string();

        // First append with idempotency key
        let (id1, is_new1) = wal
            .append_with_idempotency(
                WalOperation::SendMessage,
                &actor_id,
                Bytes::from("message payload"),
                1000,
                idempotency_key.clone(),
            )
            .await
            .unwrap();

        assert!(is_new1, "First append should create new entry");
        assert_eq!(id1, 1);

        // Second append with same idempotency key - should return existing
        let (id2, is_new2) = wal
            .append_with_idempotency(
                WalOperation::SendMessage,
                &actor_id,
                Bytes::from("different payload"),
                2000,
                idempotency_key.clone(),
            )
            .await
            .unwrap();

        assert!(!is_new2, "Second append should return existing entry");
        assert_eq!(id2, id1, "Should return same entry ID");

        // Verify only one entry exists
        assert_eq!(wal.pending_count().await.unwrap(), 1);

        // Find by idempotency key
        let found = wal
            .find_by_idempotency_key(&idempotency_key)
            .await
            .unwrap()
            .expect("Entry should be found");
        assert_eq!(found.id, id1);
        assert_eq!(found.idempotency_key, Some(idempotency_key));

        // Different idempotency key creates new entry
        let (id3, is_new3) = wal
            .append_with_idempotency(
                WalOperation::SendMessage,
                &actor_id,
                Bytes::from("another message"),
                3000,
                "msg-uuid-67890".to_string(),
            )
            .await
            .unwrap();

        assert!(is_new3, "Different key should create new entry");
        assert_ne!(id3, id1, "Should be different entry ID");
        assert_eq!(wal.pending_count().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn test_memory_wal_idempotency_after_complete() {
        let wal = MemoryWal::new();
        let actor_id = test_actor_id();
        let idempotency_key = "msg-uuid-complete".to_string();

        // Create and complete entry
        let (id1, _) = wal
            .append_with_idempotency(
                WalOperation::SendMessage,
                &actor_id,
                Bytes::from("payload"),
                1000,
                idempotency_key.clone(),
            )
            .await
            .unwrap();

        wal.complete(id1, 1500).await.unwrap();

        // Try to append again with same key - should still find completed entry
        let (id2, is_new) = wal
            .append_with_idempotency(
                WalOperation::SendMessage,
                &actor_id,
                Bytes::from("retry payload"),
                2000,
                idempotency_key.clone(),
            )
            .await
            .unwrap();

        assert!(!is_new, "Should find completed entry, not create new");
        assert_eq!(id2, id1, "Should return same entry ID");
    }

    #[tokio::test]
    async fn test_kv_wal_idempotency() {
        let kv = Arc::new(MemoryKV::new());
        let wal = KvWal::new(kv);
        let actor_id = test_actor_id();
        let idempotency_key = "kv-msg-uuid-12345".to_string();

        // First append with idempotency key
        let (id1, is_new1) = wal
            .append_with_idempotency(
                WalOperation::SendMessage,
                &actor_id,
                Bytes::from("message payload"),
                1000,
                idempotency_key.clone(),
            )
            .await
            .unwrap();

        assert!(is_new1, "First append should create new entry");

        // Second append with same idempotency key - should return existing
        let (id2, is_new2) = wal
            .append_with_idempotency(
                WalOperation::SendMessage,
                &actor_id,
                Bytes::from("different payload"),
                2000,
                idempotency_key.clone(),
            )
            .await
            .unwrap();

        assert!(!is_new2, "Second append should return existing entry");
        assert_eq!(id2, id1, "Should return same entry ID");

        // Verify only one entry
        let pending = wal.pending_entries().await.unwrap();
        assert_eq!(pending.len(), 1);

        // Find by idempotency key
        let found = wal
            .find_by_idempotency_key(&idempotency_key)
            .await
            .unwrap()
            .expect("Entry should be found");
        assert_eq!(found.id, id1);
    }

    #[tokio::test]
    async fn test_find_by_idempotency_key_not_found() {
        let wal = MemoryWal::new();

        // Non-existent key
        let result = wal.find_by_idempotency_key("nonexistent").await.unwrap();
        assert!(result.is_none());

        // Entry without idempotency key
        let actor_id = test_actor_id();
        wal.append(WalOperation::CreateAgent, &actor_id, Bytes::new(), 1000)
            .await
            .unwrap();

        // Still not found
        let result = wal.find_by_idempotency_key("nonexistent").await.unwrap();
        assert!(result.is_none());
    }
}
