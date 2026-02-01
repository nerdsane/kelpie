//! FoundationDB KV storage backend
//!
//! Production-ready, linearizable storage for Kelpie actors.
//!
//! TigerStyle: Explicit key encoding, bounded sizes, 2+ assertions per method.
//!
//! # Key Space Design
//!
//! Keys are encoded using FDB tuple layer:
//! ```text
//! ("kelpie", "actors", namespace, actor_id, "data", user_key)
//! ```
//!
//! # Example
//!
//! ```ignore
//! use kelpie_storage::FdbKV;
//!
//! let kv = FdbKV::connect(None).await?;
//! kv.set(&actor_id, b"my-key", b"my-value").await?;
//! ```

use async_trait::async_trait;
use bytes::Bytes;
use foundationdb::api::{FdbApiBuilder, NetworkAutoStop};
use foundationdb::options::StreamingMode;
use foundationdb::tuple::Subspace;
use foundationdb::{Database, RangeOption, Transaction as FdbTransaction};
use kelpie_core::constants::{
    ACTOR_KV_KEY_SIZE_BYTES_MAX, ACTOR_KV_VALUE_SIZE_BYTES_MAX, TRANSACTION_TIMEOUT_MS_DEFAULT,
};
use kelpie_core::{ActorId, Error, Result};
use std::sync::{Arc, OnceLock};
use tracing::{debug, instrument, warn};

use crate::kv::{ActorKV, ActorTransaction};

/// Global FDB network guard - must live for the entire process
/// Using OnceLock to ensure single initialization
static FDB_NETWORK: OnceLock<NetworkAutoStop> = OnceLock::new();

/// Key prefix for actor data in FDB
const KEY_PREFIX_KELPIE: &str = "kelpie";
const KEY_PREFIX_ACTORS: &str = "actors";
const KEY_PREFIX_DATA: &str = "data";

/// Maximum retry attempts for retriable errors
const TRANSACTION_RETRY_COUNT_MAX: usize = 5;

/// FoundationDB KV store
///
/// Implements `ActorKV` trait with linearizable guarantees.
#[derive(Clone)]
pub struct FdbKV {
    /// FDB database handle (connection pooled internally)
    db: Arc<Database>,
    /// Subspace for all Kelpie actor data
    subspace: Subspace,
}

impl FdbKV {
    /// Connect to FoundationDB cluster
    ///
    /// # Arguments
    ///
    /// * `cluster_file` - Path to FDB cluster file. If None, uses default location.
    ///
    /// # Errors
    ///
    /// Returns error if connection fails or FDB network cannot be started.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Use default cluster file
    /// let kv = FdbKV::connect(None).await?;
    ///
    /// // Use specific cluster file
    /// let kv = FdbKV::connect(Some("/etc/foundationdb/fdb.cluster")).await?;
    /// ```
    #[instrument(skip_all, fields(cluster_file))]
    pub async fn connect(cluster_file: Option<&str>) -> Result<Self> {
        // Boot FDB network (must be called once per process)
        // Using OnceLock to ensure single initialization and keep guard alive
        FDB_NETWORK.get_or_init(|| {
            let network_builder = FdbApiBuilder::default()
                .build()
                .expect("FDB API build failed");

            // Start network thread - must keep the guard alive for operations to work
            unsafe { network_builder.boot().expect("FDB network boot failed") }
        });

        // Open database
        let db = Database::new(cluster_file).map_err(|e| Error::Internal {
            message: format!("FDB database open failed: {}", e),
        })?;

        // Create subspace for Kelpie data
        let subspace = Subspace::from((KEY_PREFIX_KELPIE, KEY_PREFIX_ACTORS));

        debug!("Connected to FoundationDB");

        Ok(Self {
            db: Arc::new(db),
            subspace,
        })
    }

    /// Create FdbKV from existing database handle
    ///
    /// Useful for testing or when sharing a database connection.
    pub fn from_database(db: Arc<Database>) -> Self {
        let subspace = Subspace::from((KEY_PREFIX_KELPIE, KEY_PREFIX_ACTORS));
        Self { db, subspace }
    }

    /// Encode a full key for FDB storage
    ///
    /// Format: (kelpie, actors, namespace, actor_id, data, user_key)
    fn encode_key(&self, actor_id: &ActorId, key: &[u8]) -> Vec<u8> {
        // Preconditions
        assert!(
            key.len() <= ACTOR_KV_KEY_SIZE_BYTES_MAX,
            "key size {} exceeds max {}",
            key.len(),
            ACTOR_KV_KEY_SIZE_BYTES_MAX
        );
        assert!(
            !actor_id.namespace().is_empty(),
            "actor namespace cannot be empty"
        );
        assert!(!actor_id.id().is_empty(), "actor id cannot be empty");

        // Build key using subspace
        let actor_subspace =
            self.subspace
                .subspace(&(actor_id.namespace(), actor_id.id(), KEY_PREFIX_DATA));

        let encoded = actor_subspace.pack(&key);

        // Postcondition
        assert!(!encoded.is_empty(), "encoded key cannot be empty");

        encoded
    }

    /// Encode the prefix for listing keys
    ///
    /// NOTE: For prefix matching to work correctly, we need to use the subspace
    /// bytes directly instead of tuple packing. The FDB tuple layer encoding
    /// adds type markers and length encoding that prevent simple prefix matching.
    fn encode_prefix(&self, actor_id: &ActorId, _prefix: &[u8]) -> Vec<u8> {
        // Preconditions
        assert!(
            !actor_id.namespace().is_empty(),
            "actor namespace cannot be empty"
        );
        assert!(!actor_id.id().is_empty(), "actor id cannot be empty");

        let actor_subspace =
            self.subspace
                .subspace(&(actor_id.namespace(), actor_id.id(), KEY_PREFIX_DATA));

        // Always return just the subspace bytes - the range scan will find all keys
        // in this subspace, and we filter by user prefix in the list_keys/scan_prefix code
        actor_subspace.bytes().to_vec()
    }

    /// Decode a user key from FDB key
    fn decode_user_key(&self, actor_id: &ActorId, fdb_key: &[u8]) -> Option<Vec<u8>> {
        let actor_subspace =
            self.subspace
                .subspace(&(actor_id.namespace(), actor_id.id(), KEY_PREFIX_DATA));

        actor_subspace.unpack::<Vec<u8>>(fdb_key).ok()
    }

    /// Execute a transaction with automatic retry on conflicts
    async fn run_transaction<F, T>(&self, f: F) -> Result<T>
    where
        F: Fn(&FdbTransaction) -> std::result::Result<T, foundationdb::FdbError>,
    {
        let mut attempts = 0;

        loop {
            attempts += 1;
            assert!(
                attempts <= TRANSACTION_RETRY_COUNT_MAX,
                "exceeded max transaction retries"
            );

            let txn = self.db.create_trx().map_err(|e| Error::TransactionFailed {
                reason: format!("create transaction failed: {}", e),
            })?;

            // Set transaction timeout
            txn.set_option(foundationdb::options::TransactionOption::Timeout(
                TRANSACTION_TIMEOUT_MS_DEFAULT as i32,
            ))
            .map_err(|e| Error::TransactionFailed {
                reason: format!("set timeout failed: {}", e),
            })?;

            match f(&txn) {
                Ok(result) => {
                    // Commit transaction
                    txn.commit().await.map_err(|e| {
                        if e.is_retryable() {
                            Error::TransactionConflict {
                                reason: format!("commit conflict: {}", e),
                            }
                        } else {
                            Error::TransactionFailed {
                                reason: format!("commit failed: {}", e),
                            }
                        }
                    })?;
                    return Ok(result);
                }
                Err(e) if e.is_retryable() && attempts < TRANSACTION_RETRY_COUNT_MAX => {
                    warn!(
                        "Transaction conflict (attempt {}/{}), retrying",
                        attempts, TRANSACTION_RETRY_COUNT_MAX
                    );
                    // Wait before retry using on_error
                    txn.on_error(e)
                        .await
                        .map_err(|e| Error::TransactionFailed {
                            reason: format!("retry wait failed: {}", e),
                        })?;
                    continue;
                }
                Err(e) => {
                    return Err(Error::TransactionFailed {
                        reason: format!("transaction failed: {}", e),
                    });
                }
            }
        }
    }
}

impl std::fmt::Debug for FdbKV {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FdbKV")
            .field("subspace", &self.subspace)
            .finish()
    }
}

// =============================================================================
// FdbActorTransaction
// =============================================================================

use std::collections::HashMap;

/// FoundationDB implementation of ActorTransaction
///
/// Buffers writes locally, then applies them atomically on commit using
/// a single FDB transaction.
///
/// TigerStyle: Explicit buffer management, atomic commit, 2+ assertions.
pub struct FdbActorTransaction {
    /// Actor ID this transaction is scoped to
    actor_id: ActorId,
    /// Reference to the FDB KV store (cloned, shares `Arc<Database>` internally)
    kv: FdbKV,
    /// Buffered writes: key -> Some(value) for set, None for delete
    write_buffer: HashMap<Vec<u8>, Option<Vec<u8>>>,
    /// Whether the transaction has been finalized (committed or aborted)
    finalized: bool,
}

impl FdbActorTransaction {
    /// Create a new transaction for an actor
    fn new(actor_id: ActorId, kv: FdbKV) -> Self {
        // Preconditions
        assert!(
            !actor_id.namespace().is_empty(),
            "actor namespace cannot be empty"
        );
        assert!(!actor_id.id().is_empty(), "actor id cannot be empty");

        Self {
            actor_id,
            kv,
            write_buffer: HashMap::new(),
            finalized: false,
        }
    }
}

#[async_trait]
impl ActorTransaction for FdbActorTransaction {
    async fn get(&self, key: &[u8]) -> Result<Option<Bytes>> {
        // Preconditions
        assert!(!self.finalized, "transaction already finalized");
        assert!(
            key.len() <= ACTOR_KV_KEY_SIZE_BYTES_MAX,
            "key size {} exceeds max {}",
            key.len(),
            ACTOR_KV_KEY_SIZE_BYTES_MAX
        );

        // Check write buffer first (read-your-writes)
        if let Some(buffered) = self.write_buffer.get(key) {
            return Ok(buffered.as_ref().map(|v| Bytes::copy_from_slice(v)));
        }

        // Fall back to underlying store
        self.kv.get(&self.actor_id, key).await
    }

    async fn set(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        // Preconditions
        assert!(!self.finalized, "transaction already finalized");
        assert!(
            key.len() <= ACTOR_KV_KEY_SIZE_BYTES_MAX,
            "key size {} exceeds max {}",
            key.len(),
            ACTOR_KV_KEY_SIZE_BYTES_MAX
        );
        assert!(
            value.len() <= ACTOR_KV_VALUE_SIZE_BYTES_MAX,
            "value size {} exceeds max {}",
            value.len(),
            ACTOR_KV_VALUE_SIZE_BYTES_MAX
        );

        self.write_buffer.insert(key.to_vec(), Some(value.to_vec()));

        // Postcondition
        assert!(
            self.write_buffer.contains_key(key),
            "write should be buffered"
        );

        Ok(())
    }

    async fn delete(&mut self, key: &[u8]) -> Result<()> {
        // Preconditions
        assert!(!self.finalized, "transaction already finalized");
        assert!(
            key.len() <= ACTOR_KV_KEY_SIZE_BYTES_MAX,
            "key size {} exceeds max {}",
            key.len(),
            ACTOR_KV_KEY_SIZE_BYTES_MAX
        );

        self.write_buffer.insert(key.to_vec(), None);

        // Postcondition
        assert!(
            self.write_buffer.contains_key(key),
            "delete should be buffered"
        );

        Ok(())
    }

    async fn commit(mut self: Box<Self>) -> Result<()> {
        // Preconditions
        assert!(!self.finalized, "transaction already finalized");

        self.finalized = true;

        // If no writes, nothing to do
        if self.write_buffer.is_empty() {
            debug!(actor_id = %self.actor_id.qualified_name(), "Empty transaction committed");
            return Ok(());
        }

        // Encode all keys and prepare writes
        let mut encoded_ops: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();

        for (key, value) in self.write_buffer.drain() {
            let fdb_key = self.kv.encode_key(&self.actor_id, &key);
            encoded_ops.push((fdb_key, value));
        }

        let write_count = encoded_ops.len();

        // Apply all writes in a single FDB transaction
        let mut attempts = 0;

        loop {
            attempts += 1;
            assert!(
                attempts <= TRANSACTION_RETRY_COUNT_MAX,
                "exceeded max transaction retries"
            );

            let txn = self
                .kv
                .db
                .create_trx()
                .map_err(|e| Error::TransactionFailed {
                    reason: format!("create transaction failed: {}", e),
                })?;

            // Set transaction timeout
            txn.set_option(foundationdb::options::TransactionOption::Timeout(
                TRANSACTION_TIMEOUT_MS_DEFAULT as i32,
            ))
            .map_err(|e| Error::TransactionFailed {
                reason: format!("set timeout failed: {}", e),
            })?;

            // Apply all operations
            for (fdb_key, value) in &encoded_ops {
                match value {
                    Some(v) => txn.set(fdb_key, v),
                    None => txn.clear(fdb_key),
                }
            }

            // Commit (consumes txn in FDB 0.10)
            match txn.commit().await {
                Ok(_) => {
                    debug!(
                        actor_id = %self.actor_id.qualified_name(),
                        write_count = write_count,
                        attempts = attempts,
                        "FDB transaction committed"
                    );
                    return Ok(());
                }
                Err(e) if e.is_retryable() && attempts < TRANSACTION_RETRY_COUNT_MAX => {
                    warn!(
                        actor_id = %self.actor_id.qualified_name(),
                        "Transaction conflict (attempt {}/{}), retrying",
                        attempts, TRANSACTION_RETRY_COUNT_MAX
                    );
                    // FDB 0.10: TransactionCommitError contains the transaction.
                    // Call on_error() to wait with exponential backoff before retrying.
                    let _txn = e.on_error().await.map_err(|e| Error::TransactionFailed {
                        reason: format!("retry wait failed: {}", e),
                    })?;
                    // Loop back to create a new transaction and retry
                    continue;
                }
                Err(e) if e.is_retryable() => {
                    return Err(Error::TransactionConflict {
                        reason: format!("commit conflict after {} attempts: {}", attempts, e),
                    });
                }
                Err(e) => {
                    return Err(Error::TransactionFailed {
                        reason: format!("commit failed: {}", e),
                    });
                }
            }
        }
    }

    async fn abort(mut self: Box<Self>) -> Result<()> {
        // Preconditions
        assert!(!self.finalized, "transaction already finalized");

        self.finalized = true;
        self.write_buffer.clear();

        debug!(actor_id = %self.actor_id.qualified_name(), "FDB transaction aborted");

        // Postcondition
        assert!(
            self.write_buffer.is_empty(),
            "write buffer should be empty after abort"
        );

        Ok(())
    }
}

#[async_trait]
impl ActorKV for FdbKV {
    #[instrument(skip(self, key), fields(actor_id = %actor_id.qualified_name(), key_len = key.len()))]
    async fn get(&self, actor_id: &ActorId, key: &[u8]) -> Result<Option<Bytes>> {
        // Preconditions
        assert!(
            key.len() <= ACTOR_KV_KEY_SIZE_BYTES_MAX,
            "key size {} exceeds max {}",
            key.len(),
            ACTOR_KV_KEY_SIZE_BYTES_MAX
        );
        assert!(
            !actor_id.namespace().is_empty(),
            "actor namespace cannot be empty"
        );

        let fdb_key = self.encode_key(actor_id, key);
        let key_display = String::from_utf8_lossy(key).to_string();

        let txn = self.db.create_trx().map_err(|e| Error::StorageReadFailed {
            key: key_display.clone(),
            reason: format!("create transaction failed: {}", e),
        })?;

        let value = txn
            .get(&fdb_key, false)
            .await
            .map_err(|e| Error::StorageReadFailed {
                key: key_display.clone(),
                reason: format!("get failed: {}", e),
            })?;

        let result = value.map(|v| Bytes::copy_from_slice(v.as_ref()));

        debug!(
            found = result.is_some(),
            value_len = result.as_ref().map(|v| v.len()),
            "FDB get"
        );

        Ok(result)
    }

    #[instrument(skip(self, key, value), fields(actor_id = %actor_id.qualified_name(), key_len = key.len(), value_len = value.len()))]
    async fn set(&self, actor_id: &ActorId, key: &[u8], value: &[u8]) -> Result<()> {
        // Preconditions
        assert!(
            key.len() <= ACTOR_KV_KEY_SIZE_BYTES_MAX,
            "key size {} exceeds max {}",
            key.len(),
            ACTOR_KV_KEY_SIZE_BYTES_MAX
        );
        assert!(
            value.len() <= ACTOR_KV_VALUE_SIZE_BYTES_MAX,
            "value size {} exceeds max {}",
            value.len(),
            ACTOR_KV_VALUE_SIZE_BYTES_MAX
        );
        assert!(
            !actor_id.namespace().is_empty(),
            "actor namespace cannot be empty"
        );

        let fdb_key = self.encode_key(actor_id, key);
        let key_display = String::from_utf8_lossy(key).to_string();
        let value_owned = value.to_vec();

        self.run_transaction(|txn| {
            txn.set(&fdb_key, &value_owned);
            Ok(())
        })
        .await
        .map_err(|e| Error::StorageWriteFailed {
            key: key_display,
            reason: e.to_string(),
        })?;

        debug!("FDB set complete");

        Ok(())
    }

    #[instrument(skip(self, key), fields(actor_id = %actor_id.qualified_name(), key_len = key.len()))]
    async fn delete(&self, actor_id: &ActorId, key: &[u8]) -> Result<()> {
        // Preconditions
        assert!(
            key.len() <= ACTOR_KV_KEY_SIZE_BYTES_MAX,
            "key size {} exceeds max {}",
            key.len(),
            ACTOR_KV_KEY_SIZE_BYTES_MAX
        );
        assert!(
            !actor_id.namespace().is_empty(),
            "actor namespace cannot be empty"
        );

        let fdb_key = self.encode_key(actor_id, key);
        let key_display = String::from_utf8_lossy(key).to_string();

        self.run_transaction(|txn| {
            txn.clear(&fdb_key);
            Ok(())
        })
        .await
        .map_err(|e| Error::StorageWriteFailed {
            key: key_display,
            reason: e.to_string(),
        })?;

        debug!("FDB delete complete");

        Ok(())
    }

    #[instrument(skip(self, prefix), fields(actor_id = %actor_id.qualified_name(), prefix_len = prefix.len()))]
    async fn list_keys(&self, actor_id: &ActorId, prefix: &[u8]) -> Result<Vec<Vec<u8>>> {
        // Preconditions
        assert!(
            prefix.len() <= ACTOR_KV_KEY_SIZE_BYTES_MAX,
            "prefix size {} exceeds max {}",
            prefix.len(),
            ACTOR_KV_KEY_SIZE_BYTES_MAX
        );
        assert!(
            !actor_id.namespace().is_empty(),
            "actor namespace cannot be empty"
        );

        let start_key = self.encode_prefix(actor_id, prefix);

        // Calculate end key (prefix + 0xFF for range scan)
        let mut end_key = start_key.clone();
        end_key.push(0xFF);

        let txn = self.db.create_trx().map_err(|e| Error::StorageReadFailed {
            key: format!("prefix:{}", String::from_utf8_lossy(prefix)),
            reason: format!("create transaction failed: {}", e),
        })?;

        let mut range_option = RangeOption::from((start_key.as_slice(), end_key.as_slice()));
        // TigerStyle: Use WantAll streaming mode to fetch all results in one batch
        // Default mode (Iterator) returns one chunk at a time, causing partial results
        range_option.mode = StreamingMode::WantAll;

        let range =
            txn.get_range(&range_option, 1, false)
                .await
                .map_err(|e| Error::StorageReadFailed {
                    key: format!("prefix:{}", String::from_utf8_lossy(prefix)),
                    reason: format!("range scan failed: {}", e),
                })?;

        let mut keys = Vec::new();
        for kv in range.iter() {
            if let Some(user_key) = self.decode_user_key(actor_id, kv.key()) {
                // Only include keys that match the prefix
                if prefix.is_empty() || user_key.starts_with(prefix) {
                    keys.push(user_key);
                }
            }
        }

        debug!(count = keys.len(), "FDB list_keys complete");

        // Postcondition
        assert!(
            keys.iter().all(|k| k.len() <= ACTOR_KV_KEY_SIZE_BYTES_MAX),
            "returned keys must be within size limit"
        );

        Ok(keys)
    }

    #[instrument(skip(self, prefix), fields(actor_id = %actor_id.qualified_name(), prefix_len = prefix.len()))]
    async fn scan_prefix(
        &self,
        actor_id: &ActorId,
        prefix: &[u8],
    ) -> Result<Vec<(Vec<u8>, Bytes)>> {
        // Preconditions
        assert!(
            prefix.len() <= ACTOR_KV_KEY_SIZE_BYTES_MAX,
            "prefix size {} exceeds max {}",
            prefix.len(),
            ACTOR_KV_KEY_SIZE_BYTES_MAX
        );
        assert!(
            !actor_id.namespace().is_empty(),
            "actor namespace cannot be empty"
        );

        let start_key = self.encode_prefix(actor_id, prefix);

        // Calculate end key (prefix + 0xFF for range scan)
        let mut end_key = start_key.clone();
        end_key.push(0xFF);

        let txn = self.db.create_trx().map_err(|e| Error::StorageReadFailed {
            key: format!("prefix:{}", String::from_utf8_lossy(prefix)),
            reason: format!("create transaction failed: {}", e),
        })?;

        let mut range_option = RangeOption::from((start_key.as_slice(), end_key.as_slice()));
        // TigerStyle: Use WantAll streaming mode to fetch all results in one batch
        // Default mode (Iterator) returns one chunk at a time, causing partial results
        range_option.mode = StreamingMode::WantAll;

        let range =
            txn.get_range(&range_option, 1, false)
                .await
                .map_err(|e| Error::StorageReadFailed {
                    key: format!("prefix:{}", String::from_utf8_lossy(prefix)),
                    reason: format!("range scan failed: {}", e),
                })?;

        let mut results = Vec::new();
        for kv in range.iter() {
            if let Some(user_key) = self.decode_user_key(actor_id, kv.key()) {
                // Only include keys that match the prefix
                if prefix.is_empty() || user_key.starts_with(prefix) {
                    let value = Bytes::copy_from_slice(kv.value());
                    results.push((user_key, value));
                }
            }
        }

        debug!(count = results.len(), "FDB scan_prefix complete");

        // Postcondition
        assert!(
            results
                .iter()
                .all(|(k, _)| k.len() <= ACTOR_KV_KEY_SIZE_BYTES_MAX),
            "returned keys must be within size limit"
        );

        Ok(results)
    }

    #[instrument(skip(self), fields(actor_id = %actor_id.qualified_name()))]
    async fn begin_transaction(&self, actor_id: &ActorId) -> Result<Box<dyn ActorTransaction>> {
        // Preconditions
        assert!(
            !actor_id.namespace().is_empty(),
            "actor namespace cannot be empty"
        );
        assert!(!actor_id.id().is_empty(), "actor id cannot be empty");

        debug!("Beginning FDB transaction");

        Ok(Box::new(FdbActorTransaction::new(
            actor_id.clone(),
            self.clone(),
        )))
    }
}

// Implement ActorKV for Arc<FdbKV> to allow using it through Arc
//
// This is a common pattern in Rust - implement traits for smart pointers
// by delegating to the inner type.
#[async_trait]
impl ActorKV for Arc<FdbKV> {
    async fn get(&self, actor_id: &ActorId, key: &[u8]) -> Result<Option<Bytes>> {
        (**self).get(actor_id, key).await
    }

    async fn set(&self, actor_id: &ActorId, key: &[u8], value: &[u8]) -> Result<()> {
        (**self).set(actor_id, key, value).await
    }

    async fn delete(&self, actor_id: &ActorId, key: &[u8]) -> Result<()> {
        (**self).delete(actor_id, key).await
    }

    async fn list_keys(&self, actor_id: &ActorId, prefix: &[u8]) -> Result<Vec<Vec<u8>>> {
        (**self).list_keys(actor_id, prefix).await
    }

    async fn scan_prefix(
        &self,
        actor_id: &ActorId,
        prefix: &[u8],
    ) -> Result<Vec<(Vec<u8>, Bytes)>> {
        (**self).scan_prefix(actor_id, prefix).await
    }

    async fn begin_transaction(&self, actor_id: &ActorId) -> Result<Box<dyn ActorTransaction>> {
        (**self).begin_transaction(actor_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_encoding_format() {
        // This test verifies the key encoding structure without FDB
        let subspace = Subspace::from((KEY_PREFIX_KELPIE, KEY_PREFIX_ACTORS));
        let actor_subspace = subspace.subspace(&("test-ns", "actor-1", KEY_PREFIX_DATA));
        let encoded = actor_subspace.pack(&b"my-key".to_vec());

        // Key should be non-empty and start with tuple encoding
        assert!(!encoded.is_empty());

        // Verify we can decode it back
        let decoded: Vec<u8> = actor_subspace.unpack(&encoded).unwrap();
        assert_eq!(decoded, b"my-key");
    }

    #[test]
    fn test_key_encoding_ordering() {
        // Verify lexicographic ordering is preserved
        let subspace = Subspace::from((KEY_PREFIX_KELPIE, KEY_PREFIX_ACTORS));
        let actor_subspace = subspace.subspace(&("test-ns", "actor-1", KEY_PREFIX_DATA));

        let key_a = actor_subspace.pack(&b"aaa".to_vec());
        let key_b = actor_subspace.pack(&b"bbb".to_vec());
        let key_c = actor_subspace.pack(&b"ccc".to_vec());

        // Keys should be ordered
        assert!(key_a < key_b);
        assert!(key_b < key_c);
    }

    #[test]
    fn test_subspace_isolation() {
        // Different actors should have different key spaces
        let subspace = Subspace::from((KEY_PREFIX_KELPIE, KEY_PREFIX_ACTORS));

        let actor1_subspace = subspace.subspace(&("ns1", "actor-1", KEY_PREFIX_DATA));
        let actor2_subspace = subspace.subspace(&("ns1", "actor-2", KEY_PREFIX_DATA));

        let key1 = actor1_subspace.pack(&b"key".to_vec());
        let key2 = actor2_subspace.pack(&b"key".to_vec());

        // Same user key, different actors = different FDB keys
        assert_ne!(key1, key2);
    }

    // Integration tests require running FDB - mark as ignored
    #[tokio::test]
    #[ignore = "requires running FDB cluster"]
    async fn test_fdb_integration_crud() {
        let kv = FdbKV::connect(None).await.expect("FDB connection failed");
        let actor_id = ActorId::new("integration-test", "actor-1").unwrap();

        // Clean up any existing data
        let _ = kv.delete(&actor_id, b"test-key").await;

        // Set
        kv.set(&actor_id, b"test-key", b"test-value")
            .await
            .expect("set failed");

        // Get
        let value = kv
            .get(&actor_id, b"test-key")
            .await
            .expect("get failed")
            .expect("value should exist");
        assert_eq!(value.as_ref(), b"test-value");

        // Exists
        assert!(kv
            .exists(&actor_id, b"test-key")
            .await
            .expect("exists failed"));
        assert!(!kv
            .exists(&actor_id, b"nonexistent")
            .await
            .expect("exists failed"));

        // Delete
        kv.delete(&actor_id, b"test-key")
            .await
            .expect("delete failed");
        assert!(kv
            .get(&actor_id, b"test-key")
            .await
            .expect("get failed")
            .is_none());
    }

    #[tokio::test]
    #[ignore = "requires running FDB cluster"]
    async fn test_fdb_integration_list_keys() {
        let kv = FdbKV::connect(None).await.expect("FDB connection failed");
        let actor_id = ActorId::new("integration-test", "actor-list").unwrap();

        // Set up test data
        kv.set(&actor_id, b"prefix:key1", b"value1")
            .await
            .expect("set failed");
        kv.set(&actor_id, b"prefix:key2", b"value2")
            .await
            .expect("set failed");
        kv.set(&actor_id, b"other:key3", b"value3")
            .await
            .expect("set failed");

        // List with prefix
        let keys = kv
            .list_keys(&actor_id, b"prefix:")
            .await
            .expect("list_keys failed");
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&b"prefix:key1".to_vec()));
        assert!(keys.contains(&b"prefix:key2".to_vec()));

        // List all
        let all_keys = kv
            .list_keys(&actor_id, b"")
            .await
            .expect("list_keys failed");
        assert!(all_keys.len() >= 3);

        // Cleanup
        kv.delete(&actor_id, b"prefix:key1").await.unwrap();
        kv.delete(&actor_id, b"prefix:key2").await.unwrap();
        kv.delete(&actor_id, b"other:key3").await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running FDB cluster"]
    async fn test_fdb_integration_actor_isolation() {
        let kv = FdbKV::connect(None).await.expect("FDB connection failed");
        let actor1 = ActorId::new("integration-test", "isolation-1").unwrap();
        let actor2 = ActorId::new("integration-test", "isolation-2").unwrap();

        // Set same key for different actors
        kv.set(&actor1, b"shared-key", b"value-1")
            .await
            .expect("set failed");
        kv.set(&actor2, b"shared-key", b"value-2")
            .await
            .expect("set failed");

        // Values should be isolated
        let value1 = kv
            .get(&actor1, b"shared-key")
            .await
            .expect("get failed")
            .expect("value should exist");
        let value2 = kv
            .get(&actor2, b"shared-key")
            .await
            .expect("get failed")
            .expect("value should exist");

        assert_eq!(value1.as_ref(), b"value-1");
        assert_eq!(value2.as_ref(), b"value-2");

        // Cleanup
        kv.delete(&actor1, b"shared-key").await.unwrap();
        kv.delete(&actor2, b"shared-key").await.unwrap();
    }

    // =========================================================================
    // Transaction Integration Tests
    // =========================================================================

    #[tokio::test]
    #[ignore = "requires running FDB cluster"]
    async fn test_fdb_transaction_commit() {
        let kv = FdbKV::connect(None).await.expect("FDB connection failed");
        let actor_id = ActorId::new("integration-test", "txn-commit").unwrap();

        // Clean up
        let _ = kv.delete(&actor_id, b"txn-key1").await;
        let _ = kv.delete(&actor_id, b"txn-key2").await;

        // Begin transaction and write multiple keys
        let mut txn = kv.begin_transaction(&actor_id).await.expect("begin failed");
        txn.set(b"txn-key1", b"value1").await.expect("set failed");
        txn.set(b"txn-key2", b"value2").await.expect("set failed");

        // Before commit, values not visible via direct KV access
        assert!(kv.get(&actor_id, b"txn-key1").await.unwrap().is_none());
        assert!(kv.get(&actor_id, b"txn-key2").await.unwrap().is_none());

        // Commit
        txn.commit().await.expect("commit failed");

        // After commit, values visible
        assert_eq!(
            kv.get(&actor_id, b"txn-key1")
                .await
                .unwrap()
                .unwrap()
                .as_ref(),
            b"value1"
        );
        assert_eq!(
            kv.get(&actor_id, b"txn-key2")
                .await
                .unwrap()
                .unwrap()
                .as_ref(),
            b"value2"
        );

        // Cleanup
        kv.delete(&actor_id, b"txn-key1").await.unwrap();
        kv.delete(&actor_id, b"txn-key2").await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running FDB cluster"]
    async fn test_fdb_transaction_abort() {
        let kv = FdbKV::connect(None).await.expect("FDB connection failed");
        let actor_id = ActorId::new("integration-test", "txn-abort").unwrap();

        // Clean up
        let _ = kv.delete(&actor_id, b"abort-key").await;

        // Begin transaction and write
        let mut txn = kv.begin_transaction(&actor_id).await.expect("begin failed");
        txn.set(b"abort-key", b"value").await.expect("set failed");

        // Abort
        txn.abort().await.expect("abort failed");

        // After abort, value not visible
        assert!(kv.get(&actor_id, b"abort-key").await.unwrap().is_none());
    }

    #[tokio::test]
    #[ignore = "requires running FDB cluster"]
    async fn test_fdb_transaction_read_your_writes() {
        let kv = FdbKV::connect(None).await.expect("FDB connection failed");
        let actor_id = ActorId::new("integration-test", "txn-ryw").unwrap();

        // Set initial value
        kv.set(&actor_id, b"ryw-key", b"initial")
            .await
            .expect("set failed");

        // Begin transaction
        let mut txn = kv.begin_transaction(&actor_id).await.expect("begin failed");

        // Read initial value
        assert_eq!(
            txn.get(b"ryw-key").await.unwrap().unwrap().as_ref(),
            b"initial"
        );

        // Write new value within transaction
        txn.set(b"ryw-key", b"updated").await.expect("set failed");

        // Read updated value (read-your-writes)
        assert_eq!(
            txn.get(b"ryw-key").await.unwrap().unwrap().as_ref(),
            b"updated"
        );

        // Commit
        txn.commit().await.expect("commit failed");

        // Value persisted
        assert_eq!(
            kv.get(&actor_id, b"ryw-key")
                .await
                .unwrap()
                .unwrap()
                .as_ref(),
            b"updated"
        );

        // Cleanup
        kv.delete(&actor_id, b"ryw-key").await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running FDB cluster"]
    async fn test_fdb_transaction_delete() {
        let kv = FdbKV::connect(None).await.expect("FDB connection failed");
        let actor_id = ActorId::new("integration-test", "txn-delete").unwrap();

        // Set initial value
        kv.set(&actor_id, b"del-key", b"value")
            .await
            .expect("set failed");

        // Begin transaction and delete
        let mut txn = kv.begin_transaction(&actor_id).await.expect("begin failed");
        txn.delete(b"del-key").await.expect("delete failed");

        // Read-your-writes: see the delete
        assert!(txn.get(b"del-key").await.unwrap().is_none());

        // Commit
        txn.commit().await.expect("commit failed");

        // Value deleted
        assert!(kv.get(&actor_id, b"del-key").await.unwrap().is_none());
    }

    #[tokio::test]
    #[ignore = "requires running FDB cluster"]
    async fn test_fdb_transaction_atomicity() {
        let kv = FdbKV::connect(None).await.expect("FDB connection failed");
        let actor_id = ActorId::new("integration-test", "txn-atomic").unwrap();

        // Clean up
        let _ = kv.delete(&actor_id, b"atomic-key1").await;
        let _ = kv.delete(&actor_id, b"atomic-key2").await;
        let _ = kv.delete(&actor_id, b"atomic-key3").await;

        // Write multiple keys in a transaction
        let mut txn = kv.begin_transaction(&actor_id).await.expect("begin failed");
        txn.set(b"atomic-key1", b"value1").await.unwrap();
        txn.set(b"atomic-key2", b"value2").await.unwrap();
        txn.set(b"atomic-key3", b"value3").await.unwrap();
        txn.commit().await.expect("commit failed");

        // All three keys should be visible
        assert!(kv.get(&actor_id, b"atomic-key1").await.unwrap().is_some());
        assert!(kv.get(&actor_id, b"atomic-key2").await.unwrap().is_some());
        assert!(kv.get(&actor_id, b"atomic-key3").await.unwrap().is_some());

        // Cleanup
        kv.delete(&actor_id, b"atomic-key1").await.unwrap();
        kv.delete(&actor_id, b"atomic-key2").await.unwrap();
        kv.delete(&actor_id, b"atomic-key3").await.unwrap();
    }
}
