//! SimStorage - In-memory AgentStorage for testing with FDB-like transaction semantics
//!
//! TigerStyle: Deterministic in-memory storage for DST compatibility.
//!
//! This implementation provides:
//! - Full AgentStorage trait implementation
//! - FDB-like transaction semantics (atomicity, conflict detection)
//! - Thread-safe concurrent access via RwLock
//! - Optional fault injection for DST testing
//! - No external dependencies (no FDB required)
//!
//! Transaction Semantics (matching FDB):
//! - Atomic commits: multi-key operations succeed or fail together
//! - Conflict detection: concurrent writes to same keys detected via versioning
//! - Atomic checkpoint: session + message saved together
//! - Atomic cascade delete: agent and all related data deleted atomically
//!
//! Use Cases:
//! - Unit tests
//! - DST (Deterministic Simulation Testing)
//! - Local development without FDB
//! - CI pipelines

use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use crate::models::{AgentGroup, ArchivalEntry, Block, Identity, Job, MCPServer, Message, Project};

use super::traits::{AgentStorage, StorageError};
use super::types::{AgentMetadata, CustomToolRecord, SessionState};

#[cfg(feature = "dst")]
use kelpie_dst::fault::FaultInjector;

// =============================================================================
// Constants (TigerStyle)
// =============================================================================

/// Maximum transaction retry attempts for conflict resolution
const TRANSACTION_RETRY_COUNT_MAX: u64 = 5;

// =============================================================================
// Version Types for MVCC
// =============================================================================

/// Version number for MVCC conflict detection
type Version = u64;

/// Key identifier for conflict detection
///
/// TigerStyle: Explicit key types for type-safe conflict tracking
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub enum StorageKey {
    Agent(String),
    Blocks(String),
    Session {
        agent_id: String,
        session_id: String,
    },
    Messages(String),
    CustomTool(String),
    McpServer(String),
    AgentGroup(String),
    Identity(String),
    Project(String),
    Job(String),
    Archival {
        agent_id: String,
        entry_id: String,
    },
    ArchivalAll(String),
}

/// In-memory storage implementation for testing and development
///
/// TigerStyle: All fields use RwLock for thread-safe concurrent access.
/// Data is stored in HashMaps, providing O(1) lookups.
/// Transaction semantics match FDB for realistic simulation.
pub struct SimStorage {
    /// Inner storage (Arc for cloning support)
    inner: Arc<SimStorageInner>,
}

/// Inner storage state with version tracking for MVCC
///
/// TigerStyle: Separate inner struct for Arc sharing and version management
struct SimStorageInner {
    /// Global version counter (incremented on each write)
    version: AtomicU64,
    /// Version when each key was last modified
    key_versions: RwLock<HashMap<StorageKey, Version>>,
    /// Agent metadata by ID
    agents: RwLock<HashMap<String, AgentMetadata>>,
    /// Blocks by agent_id -> label -> Block
    blocks: RwLock<HashMap<String, HashMap<String, Block>>>,
    /// Sessions by agent_id -> session_id -> SessionState
    sessions: RwLock<HashMap<String, HashMap<String, SessionState>>>,
    /// Custom tools by name
    custom_tools: RwLock<HashMap<String, CustomToolRecord>>,
    /// Messages by agent_id (ordered by insertion)
    messages: RwLock<HashMap<String, Vec<Message>>>,
    /// MCP servers by ID
    mcp_servers: RwLock<HashMap<String, MCPServer>>,
    /// Agent groups by ID
    agent_groups: RwLock<HashMap<String, AgentGroup>>,
    /// Identities by ID
    identities: RwLock<HashMap<String, Identity>>,
    /// Projects by ID
    projects: RwLock<HashMap<String, Project>>,
    /// Jobs by ID
    jobs: RwLock<HashMap<String, Job>>,
    /// Archival entries by agent_id -> entry_id -> ArchivalEntry
    archival: RwLock<HashMap<String, HashMap<String, ArchivalEntry>>>,
    /// Optional fault injector for DST
    #[cfg(feature = "dst")]
    fault_injector: Option<Arc<FaultInjector>>,
}

impl SimStorageInner {
    fn new() -> Self {
        Self {
            version: AtomicU64::new(0),
            key_versions: RwLock::new(HashMap::new()),
            agents: RwLock::new(HashMap::new()),
            blocks: RwLock::new(HashMap::new()),
            sessions: RwLock::new(HashMap::new()),
            custom_tools: RwLock::new(HashMap::new()),
            messages: RwLock::new(HashMap::new()),
            mcp_servers: RwLock::new(HashMap::new()),
            agent_groups: RwLock::new(HashMap::new()),
            identities: RwLock::new(HashMap::new()),
            projects: RwLock::new(HashMap::new()),
            jobs: RwLock::new(HashMap::new()),
            archival: RwLock::new(HashMap::new()),
            #[cfg(feature = "dst")]
            fault_injector: None,
        }
    }

    #[cfg(feature = "dst")]
    fn with_fault_injector(fault_injector: Arc<FaultInjector>) -> Self {
        Self {
            version: AtomicU64::new(0),
            key_versions: RwLock::new(HashMap::new()),
            agents: RwLock::new(HashMap::new()),
            blocks: RwLock::new(HashMap::new()),
            sessions: RwLock::new(HashMap::new()),
            custom_tools: RwLock::new(HashMap::new()),
            messages: RwLock::new(HashMap::new()),
            mcp_servers: RwLock::new(HashMap::new()),
            agent_groups: RwLock::new(HashMap::new()),
            identities: RwLock::new(HashMap::new()),
            projects: RwLock::new(HashMap::new()),
            jobs: RwLock::new(HashMap::new()),
            archival: RwLock::new(HashMap::new()),
            fault_injector: Some(fault_injector),
        }
    }

    /// Get the current global version
    fn current_version(&self) -> Version {
        self.version.load(Ordering::SeqCst)
    }

    /// Check if any keys have been modified since the given version
    fn has_conflicts(&self, read_set: &HashSet<StorageKey>, since_version: Version) -> bool {
        if let Ok(versions) = self.key_versions.read() {
            for key in read_set {
                if let Some(&key_version) = versions.get(key) {
                    if key_version > since_version {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Update key versions after a successful write
    fn update_key_versions(&self, keys: &[StorageKey]) {
        let new_version = self.version.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(mut versions) = self.key_versions.write() {
            for key in keys {
                versions.insert(key.clone(), new_version);
            }
        }
    }
}

impl SimStorage {
    /// Create a new empty SimStorage
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SimStorageInner::new()),
        }
    }

    /// Create SimStorage with fault injection for DST
    #[cfg(feature = "dst")]
    pub fn with_fault_injector(fault_injector: Arc<FaultInjector>) -> Self {
        Self {
            inner: Arc::new(SimStorageInner::with_fault_injector(fault_injector)),
        }
    }

    /// Begin a new transaction for read-modify-write operations
    ///
    /// Returns a transaction that tracks reads and detects conflicts on commit.
    pub fn begin_transaction(&self) -> SimStorageTransaction {
        SimStorageTransaction::new(self.inner.clone())
    }

    /// Check if fault should be injected for an operation
    #[cfg(feature = "dst")]
    fn should_inject_fault(&self, operation: &str) -> bool {
        if let Some(ref injector) = self.inner.fault_injector {
            injector.should_inject(operation).is_some()
        } else {
            false
        }
    }

    #[cfg(not(feature = "dst"))]
    #[allow(dead_code)]
    fn should_inject_fault(&self, _operation: &str) -> bool {
        false
    }

    /// Helper to create fault-injected error
    #[cfg(feature = "dst")]
    fn fault_error(operation: &str) -> StorageError {
        StorageError::FaultInjected {
            operation: operation.to_string(),
        }
    }

    /// Helper for read lock errors
    fn lock_error(operation: &str) -> StorageError {
        StorageError::Internal {
            message: format!("{}: lock poisoned", operation),
        }
    }
}

impl Clone for SimStorage {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

// =============================================================================
// SimStorageTransaction - FDB-like transaction semantics
// =============================================================================

/// Transaction for SimStorage with FDB-like semantics
///
/// Tracks reads and detects conflicts on commit. Provides:
/// - Read tracking for conflict detection
/// - Version-based conflict detection (optimistic concurrency)
/// - Automatic retry support via is_retriable() on errors
///
/// TigerStyle: Explicit transaction lifecycle, 2+ assertions per method
pub struct SimStorageTransaction {
    /// Reference to storage inner
    storage: Arc<SimStorageInner>,
    /// Snapshot version at transaction start
    snapshot_version: Version,
    /// Keys read during transaction (for conflict detection)
    read_set: HashSet<StorageKey>,
    /// Keys that will be written
    write_keys: Vec<StorageKey>,
    /// Whether transaction is finalized
    finalized: bool,
}

impl SimStorageTransaction {
    fn new(storage: Arc<SimStorageInner>) -> Self {
        let snapshot_version = storage.current_version();
        Self {
            storage,
            snapshot_version,
            read_set: HashSet::new(),
            write_keys: Vec::new(),
            finalized: false,
        }
    }

    /// Record a read for conflict detection
    pub fn record_read(&mut self, key: StorageKey) {
        assert!(!self.finalized, "transaction already finalized");
        self.read_set.insert(key);
    }

    /// Record a write key for version updates
    pub fn record_write(&mut self, key: StorageKey) {
        assert!(!self.finalized, "transaction already finalized");
        self.write_keys.push(key);
    }

    /// Check for conflicts before committing
    ///
    /// Returns error if any read keys have been modified since transaction start
    pub fn check_conflicts(&self) -> Result<(), StorageError> {
        assert!(!self.finalized, "transaction already finalized");

        if self
            .storage
            .has_conflicts(&self.read_set, self.snapshot_version)
        {
            return Err(StorageError::TransactionConflict {
                reason: "concurrent modification detected".to_string(),
            });
        }
        Ok(())
    }

    /// Commit the transaction (update versions for written keys)
    ///
    /// Call this AFTER successfully applying writes to update version tracking
    pub fn commit(&mut self) {
        assert!(!self.finalized, "transaction already finalized");
        self.storage.update_key_versions(&self.write_keys);
        self.finalized = true;
    }

    /// Abort the transaction (discard without updating versions)
    pub fn abort(&mut self) {
        assert!(!self.finalized, "transaction already finalized");
        self.finalized = true;
    }
}

impl Default for SimStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentStorage for SimStorage {
    // =========================================================================
    // Agent Metadata Operations
    // =========================================================================

    async fn save_agent(&self, agent: &AgentMetadata) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("save_agent") {
            return Err(Self::fault_error("save_agent"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::Agent(agent.id.clone()));

        let mut agents = self
            .inner
            .agents
            .write()
            .map_err(|_| Self::lock_error("save_agent"))?;
        agents.insert(agent.id.clone(), agent.clone());
        drop(agents);

        txn.commit();
        Ok(())
    }

    async fn load_agent(&self, id: &str) -> Result<Option<AgentMetadata>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("load_agent") {
            return Err(Self::fault_error("load_agent"));
        }

        let agents = self
            .inner
            .agents
            .read()
            .map_err(|_| Self::lock_error("load_agent"))?;
        Ok(agents.get(id).cloned())
    }

    async fn delete_agent(&self, id: &str) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("delete_agent") {
            return Err(Self::fault_error("delete_agent"));
        }

        // TigerStyle: Atomic cascade delete - acquire ALL locks BEFORE making changes
        // This ensures either ALL data is deleted or NONE is deleted
        // Lock ordering: agents -> blocks -> sessions -> messages -> archival
        // (consistent ordering prevents deadlocks)

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::Agent(id.to_string()));
        txn.record_write(StorageKey::Blocks(id.to_string()));
        txn.record_write(StorageKey::Messages(id.to_string()));
        txn.record_write(StorageKey::ArchivalAll(id.to_string()));

        // Acquire all locks in consistent order
        let mut agents = self
            .inner
            .agents
            .write()
            .map_err(|_| Self::lock_error("delete_agent"))?;
        let mut blocks = self
            .inner
            .blocks
            .write()
            .map_err(|_| Self::lock_error("delete_agent_blocks"))?;
        let mut sessions = self
            .inner
            .sessions
            .write()
            .map_err(|_| Self::lock_error("delete_agent_sessions"))?;
        let mut messages = self
            .inner
            .messages
            .write()
            .map_err(|_| Self::lock_error("delete_agent_messages"))?;
        let mut archival = self
            .inner
            .archival
            .write()
            .map_err(|_| Self::lock_error("delete_agent_archival"))?;

        // Now atomically delete all data (all locks held)
        agents.remove(id);
        blocks.remove(id);
        sessions.remove(id);
        messages.remove(id);
        archival.remove(id);

        // Release locks (done implicitly when guards drop)
        drop(archival);
        drop(messages);
        drop(sessions);
        drop(blocks);
        drop(agents);

        txn.commit();
        Ok(())
    }

    async fn list_agents(&self) -> Result<Vec<AgentMetadata>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("list_agents") {
            return Err(Self::fault_error("list_agents"));
        }

        let agents = self
            .inner
            .agents
            .read()
            .map_err(|_| Self::lock_error("list_agents"))?;
        Ok(agents.values().cloned().collect())
    }

    // =========================================================================
    // Core Memory Block Operations
    // =========================================================================

    async fn save_blocks(&self, agent_id: &str, blocks: &[Block]) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("save_blocks") {
            return Err(Self::fault_error("save_blocks"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::Blocks(agent_id.to_string()));

        let mut all_blocks = self
            .inner
            .blocks
            .write()
            .map_err(|_| Self::lock_error("save_blocks"))?;

        let agent_blocks = all_blocks.entry(agent_id.to_string()).or_default();
        for block in blocks {
            agent_blocks.insert(block.label.clone(), block.clone());
        }
        drop(all_blocks);

        txn.commit();
        Ok(())
    }

    async fn load_blocks(&self, agent_id: &str) -> Result<Vec<Block>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("load_blocks") {
            return Err(Self::fault_error("load_blocks"));
        }

        let all_blocks = self
            .inner
            .blocks
            .read()
            .map_err(|_| Self::lock_error("load_blocks"))?;

        Ok(all_blocks
            .get(agent_id)
            .map(|blocks| blocks.values().cloned().collect())
            .unwrap_or_default())
    }

    async fn update_block(
        &self,
        agent_id: &str,
        label: &str,
        value: &str,
    ) -> Result<Block, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("update_block") {
            return Err(Self::fault_error("update_block"));
        }

        // Use transaction for atomic read-modify-write with conflict detection
        let mut attempts = 0u64;
        loop {
            attempts += 1;
            assert!(
                attempts <= TRANSACTION_RETRY_COUNT_MAX,
                "exceeded max transaction retries"
            );

            let mut txn = self.begin_transaction();
            txn.record_read(StorageKey::Blocks(agent_id.to_string()));
            txn.record_write(StorageKey::Blocks(agent_id.to_string()));

            let mut all_blocks = self
                .inner
                .blocks
                .write()
                .map_err(|_| Self::lock_error("update_block"))?;

            // Check for conflicts before proceeding
            if let Err(e) = txn.check_conflicts() {
                drop(all_blocks);
                txn.abort();
                if attempts < TRANSACTION_RETRY_COUNT_MAX {
                    continue;
                }
                return Err(e);
            }

            let agent_blocks = all_blocks.entry(agent_id.to_string()).or_default();

            let block = agent_blocks
                .get_mut(label)
                .ok_or_else(|| StorageError::NotFound {
                    resource: "block",
                    id: format!("{}:{}", agent_id, label),
                })?;

            block.value = value.to_string();
            let result = block.clone();
            drop(all_blocks);

            txn.commit();
            return Ok(result);
        }
    }

    async fn append_block(
        &self,
        agent_id: &str,
        label: &str,
        content: &str,
    ) -> Result<Block, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("append_block") {
            return Err(Self::fault_error("append_block"));
        }

        // Use transaction for atomic read-modify-write with conflict detection
        let mut attempts = 0u64;
        loop {
            attempts += 1;
            assert!(
                attempts <= TRANSACTION_RETRY_COUNT_MAX,
                "exceeded max transaction retries"
            );

            let mut txn = self.begin_transaction();
            txn.record_read(StorageKey::Blocks(agent_id.to_string()));
            txn.record_write(StorageKey::Blocks(agent_id.to_string()));

            let mut all_blocks = self
                .inner
                .blocks
                .write()
                .map_err(|_| Self::lock_error("append_block"))?;

            // Check for conflicts before proceeding
            if let Err(e) = txn.check_conflicts() {
                drop(all_blocks);
                txn.abort();
                if attempts < TRANSACTION_RETRY_COUNT_MAX {
                    continue;
                }
                return Err(e);
            }

            let agent_blocks = all_blocks.entry(agent_id.to_string()).or_default();

            let block = agent_blocks
                .entry(label.to_string())
                .or_insert_with(|| Block {
                    id: uuid::Uuid::new_v4().to_string(),
                    label: label.to_string(),
                    value: String::new(),
                    description: None,
                    limit: None,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                });

            block.value.push_str(content);
            block.updated_at = chrono::Utc::now();
            let result = block.clone();
            drop(all_blocks);

            txn.commit();
            return Ok(result);
        }
    }

    // =========================================================================
    // Session State Operations
    // =========================================================================

    async fn save_session(&self, state: &SessionState) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("save_session") {
            return Err(Self::fault_error("save_session"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::Session {
            agent_id: state.agent_id.clone(),
            session_id: state.session_id.clone(),
        });

        let mut all_sessions = self
            .inner
            .sessions
            .write()
            .map_err(|_| Self::lock_error("save_session"))?;

        let agent_sessions = all_sessions.entry(state.agent_id.clone()).or_default();
        agent_sessions.insert(state.session_id.clone(), state.clone());
        drop(all_sessions);

        txn.commit();
        Ok(())
    }

    async fn load_session(
        &self,
        agent_id: &str,
        session_id: &str,
    ) -> Result<Option<SessionState>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("load_session") {
            return Err(Self::fault_error("load_session"));
        }

        let all_sessions = self
            .inner
            .sessions
            .read()
            .map_err(|_| Self::lock_error("load_session"))?;

        Ok(all_sessions
            .get(agent_id)
            .and_then(|sessions| sessions.get(session_id))
            .cloned())
    }

    async fn delete_session(&self, agent_id: &str, session_id: &str) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("delete_session") {
            return Err(Self::fault_error("delete_session"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::Session {
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
        });

        let mut all_sessions = self
            .inner
            .sessions
            .write()
            .map_err(|_| Self::lock_error("delete_session"))?;

        if let Some(agent_sessions) = all_sessions.get_mut(agent_id) {
            agent_sessions.remove(session_id);
        }
        drop(all_sessions);

        txn.commit();
        Ok(())
    }

    async fn list_sessions(&self, agent_id: &str) -> Result<Vec<SessionState>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("list_sessions") {
            return Err(Self::fault_error("list_sessions"));
        }

        let all_sessions = self
            .inner
            .sessions
            .read()
            .map_err(|_| Self::lock_error("list_sessions"))?;

        Ok(all_sessions
            .get(agent_id)
            .map(|sessions| sessions.values().cloned().collect())
            .unwrap_or_default())
    }

    // =========================================================================
    // Custom Tool Operations
    // =========================================================================

    async fn save_custom_tool(&self, tool: &CustomToolRecord) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("save_custom_tool") {
            return Err(Self::fault_error("save_custom_tool"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::CustomTool(tool.name.clone()));

        let mut tools = self
            .inner
            .custom_tools
            .write()
            .map_err(|_| Self::lock_error("save_custom_tool"))?;
        tools.insert(tool.name.clone(), tool.clone());
        drop(tools);

        txn.commit();
        Ok(())
    }

    async fn load_custom_tool(&self, name: &str) -> Result<Option<CustomToolRecord>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("load_custom_tool") {
            return Err(Self::fault_error("load_custom_tool"));
        }

        let tools = self
            .inner
            .custom_tools
            .read()
            .map_err(|_| Self::lock_error("load_custom_tool"))?;
        Ok(tools.get(name).cloned())
    }

    async fn delete_custom_tool(&self, name: &str) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("delete_custom_tool") {
            return Err(Self::fault_error("delete_custom_tool"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::CustomTool(name.to_string()));

        let mut tools = self
            .inner
            .custom_tools
            .write()
            .map_err(|_| Self::lock_error("delete_custom_tool"))?;
        tools.remove(name);
        drop(tools);

        txn.commit();
        Ok(())
    }

    async fn list_custom_tools(&self) -> Result<Vec<CustomToolRecord>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("list_custom_tools") {
            return Err(Self::fault_error("list_custom_tools"));
        }

        let tools = self
            .inner
            .custom_tools
            .read()
            .map_err(|_| Self::lock_error("list_custom_tools"))?;
        Ok(tools.values().cloned().collect())
    }

    // =========================================================================
    // Message Operations
    // =========================================================================

    async fn append_message(&self, agent_id: &str, message: &Message) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("append_message") {
            return Err(Self::fault_error("append_message"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::Messages(agent_id.to_string()));

        let mut all_messages = self
            .inner
            .messages
            .write()
            .map_err(|_| Self::lock_error("append_message"))?;

        let agent_messages = all_messages.entry(agent_id.to_string()).or_default();
        agent_messages.push(message.clone());
        drop(all_messages);

        txn.commit();
        Ok(())
    }

    async fn load_messages(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<Message>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("load_messages") {
            return Err(Self::fault_error("load_messages"));
        }

        let all_messages = self
            .inner
            .messages
            .read()
            .map_err(|_| Self::lock_error("load_messages"))?;

        let messages = all_messages
            .get(agent_id)
            .map(|msgs| {
                let start = msgs.len().saturating_sub(limit);
                msgs[start..].to_vec()
            })
            .unwrap_or_default();

        Ok(messages)
    }

    async fn load_messages_since(
        &self,
        agent_id: &str,
        since_ms: u64,
    ) -> Result<Vec<Message>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("load_messages_since") {
            return Err(Self::fault_error("load_messages_since"));
        }

        let all_messages = self
            .inner
            .messages
            .read()
            .map_err(|_| Self::lock_error("load_messages_since"))?;

        let messages = all_messages
            .get(agent_id)
            .map(|msgs| {
                msgs.iter()
                    .filter(|m| m.created_at.timestamp_millis() as u64 > since_ms)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        Ok(messages)
    }

    async fn count_messages(&self, agent_id: &str) -> Result<usize, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("count_messages") {
            return Err(Self::fault_error("count_messages"));
        }

        let all_messages = self
            .inner
            .messages
            .read()
            .map_err(|_| Self::lock_error("count_messages"))?;

        Ok(all_messages.get(agent_id).map(|m| m.len()).unwrap_or(0))
    }

    async fn delete_messages(&self, agent_id: &str) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("delete_messages") {
            return Err(Self::fault_error("delete_messages"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::Messages(agent_id.to_string()));

        let mut all_messages = self
            .inner
            .messages
            .write()
            .map_err(|_| Self::lock_error("delete_messages"))?;
        all_messages.remove(agent_id);
        drop(all_messages);

        txn.commit();
        Ok(())
    }

    // =========================================================================
    // MCP Server Operations
    // =========================================================================

    async fn save_mcp_server(&self, server: &MCPServer) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("save_mcp_server") {
            return Err(Self::fault_error("save_mcp_server"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::McpServer(server.id.clone()));

        let mut servers = self
            .inner
            .mcp_servers
            .write()
            .map_err(|_| Self::lock_error("save_mcp_server"))?;
        servers.insert(server.id.clone(), server.clone());
        drop(servers);

        txn.commit();
        Ok(())
    }

    async fn load_mcp_server(&self, id: &str) -> Result<Option<MCPServer>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("load_mcp_server") {
            return Err(Self::fault_error("load_mcp_server"));
        }

        let servers = self
            .inner
            .mcp_servers
            .read()
            .map_err(|_| Self::lock_error("load_mcp_server"))?;
        Ok(servers.get(id).cloned())
    }

    async fn delete_mcp_server(&self, id: &str) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("delete_mcp_server") {
            return Err(Self::fault_error("delete_mcp_server"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::McpServer(id.to_string()));

        let mut servers = self
            .inner
            .mcp_servers
            .write()
            .map_err(|_| Self::lock_error("delete_mcp_server"))?;
        servers.remove(id);
        drop(servers);

        txn.commit();
        Ok(())
    }

    async fn list_mcp_servers(&self) -> Result<Vec<MCPServer>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("list_mcp_servers") {
            return Err(Self::fault_error("list_mcp_servers"));
        }

        let servers = self
            .inner
            .mcp_servers
            .read()
            .map_err(|_| Self::lock_error("list_mcp_servers"))?;
        Ok(servers.values().cloned().collect())
    }

    // =========================================================================
    // Agent Group Operations
    // =========================================================================

    async fn save_agent_group(&self, group: &AgentGroup) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("save_agent_group") {
            return Err(Self::fault_error("save_agent_group"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::AgentGroup(group.id.clone()));

        let mut groups = self
            .inner
            .agent_groups
            .write()
            .map_err(|_| Self::lock_error("save_agent_group"))?;
        groups.insert(group.id.clone(), group.clone());
        drop(groups);

        txn.commit();
        Ok(())
    }

    async fn load_agent_group(&self, id: &str) -> Result<Option<AgentGroup>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("load_agent_group") {
            return Err(Self::fault_error("load_agent_group"));
        }

        let groups = self
            .inner
            .agent_groups
            .read()
            .map_err(|_| Self::lock_error("load_agent_group"))?;
        Ok(groups.get(id).cloned())
    }

    async fn delete_agent_group(&self, id: &str) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("delete_agent_group") {
            return Err(Self::fault_error("delete_agent_group"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::AgentGroup(id.to_string()));

        let mut groups = self
            .inner
            .agent_groups
            .write()
            .map_err(|_| Self::lock_error("delete_agent_group"))?;
        groups.remove(id);
        drop(groups);

        txn.commit();
        Ok(())
    }

    async fn list_agent_groups(&self) -> Result<Vec<AgentGroup>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("list_agent_groups") {
            return Err(Self::fault_error("list_agent_groups"));
        }

        let groups = self
            .inner
            .agent_groups
            .read()
            .map_err(|_| Self::lock_error("list_agent_groups"))?;
        Ok(groups.values().cloned().collect())
    }

    // =========================================================================
    // Identity Operations
    // =========================================================================

    async fn save_identity(&self, identity: &Identity) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("save_identity") {
            return Err(Self::fault_error("save_identity"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::Identity(identity.id.clone()));

        let mut identities = self
            .inner
            .identities
            .write()
            .map_err(|_| Self::lock_error("save_identity"))?;
        identities.insert(identity.id.clone(), identity.clone());
        drop(identities);

        txn.commit();
        Ok(())
    }

    async fn load_identity(&self, id: &str) -> Result<Option<Identity>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("load_identity") {
            return Err(Self::fault_error("load_identity"));
        }

        let identities = self
            .inner
            .identities
            .read()
            .map_err(|_| Self::lock_error("load_identity"))?;
        Ok(identities.get(id).cloned())
    }

    async fn delete_identity(&self, id: &str) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("delete_identity") {
            return Err(Self::fault_error("delete_identity"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::Identity(id.to_string()));

        let mut identities = self
            .inner
            .identities
            .write()
            .map_err(|_| Self::lock_error("delete_identity"))?;
        identities.remove(id);
        drop(identities);

        txn.commit();
        Ok(())
    }

    async fn list_identities(&self) -> Result<Vec<Identity>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("list_identities") {
            return Err(Self::fault_error("list_identities"));
        }

        let identities = self
            .inner
            .identities
            .read()
            .map_err(|_| Self::lock_error("list_identities"))?;
        Ok(identities.values().cloned().collect())
    }

    // =========================================================================
    // Project Operations
    // =========================================================================

    async fn save_project(&self, project: &Project) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("save_project") {
            return Err(Self::fault_error("save_project"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::Project(project.id.clone()));

        let mut projects = self
            .inner
            .projects
            .write()
            .map_err(|_| Self::lock_error("save_project"))?;
        projects.insert(project.id.clone(), project.clone());
        drop(projects);

        txn.commit();
        Ok(())
    }

    async fn load_project(&self, id: &str) -> Result<Option<Project>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("load_project") {
            return Err(Self::fault_error("load_project"));
        }

        let projects = self
            .inner
            .projects
            .read()
            .map_err(|_| Self::lock_error("load_project"))?;
        Ok(projects.get(id).cloned())
    }

    async fn delete_project(&self, id: &str) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("delete_project") {
            return Err(Self::fault_error("delete_project"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::Project(id.to_string()));

        let mut projects = self
            .inner
            .projects
            .write()
            .map_err(|_| Self::lock_error("delete_project"))?;
        projects.remove(id);
        drop(projects);

        txn.commit();
        Ok(())
    }

    async fn list_projects(&self) -> Result<Vec<Project>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("list_projects") {
            return Err(Self::fault_error("list_projects"));
        }

        let projects = self
            .inner
            .projects
            .read()
            .map_err(|_| Self::lock_error("list_projects"))?;
        Ok(projects.values().cloned().collect())
    }

    // =========================================================================
    // Job Operations
    // =========================================================================

    async fn save_job(&self, job: &Job) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("save_job") {
            return Err(Self::fault_error("save_job"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::Job(job.id.clone()));

        let mut jobs = self
            .inner
            .jobs
            .write()
            .map_err(|_| Self::lock_error("save_job"))?;
        jobs.insert(job.id.clone(), job.clone());
        drop(jobs);

        txn.commit();
        Ok(())
    }

    async fn load_job(&self, id: &str) -> Result<Option<Job>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("load_job") {
            return Err(Self::fault_error("load_job"));
        }

        let jobs = self
            .inner
            .jobs
            .read()
            .map_err(|_| Self::lock_error("load_job"))?;
        Ok(jobs.get(id).cloned())
    }

    async fn delete_job(&self, id: &str) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("delete_job") {
            return Err(Self::fault_error("delete_job"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::Job(id.to_string()));

        let mut jobs = self
            .inner
            .jobs
            .write()
            .map_err(|_| Self::lock_error("delete_job"))?;
        jobs.remove(id);
        drop(jobs);

        txn.commit();
        Ok(())
    }

    async fn list_jobs(&self) -> Result<Vec<Job>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("list_jobs") {
            return Err(Self::fault_error("list_jobs"));
        }

        let jobs = self
            .inner
            .jobs
            .read()
            .map_err(|_| Self::lock_error("list_jobs"))?;
        Ok(jobs.values().cloned().collect())
    }

    // =========================================================================
    // Archival Memory Operations
    // =========================================================================

    async fn save_archival_entry(
        &self,
        agent_id: &str,
        entry: &ArchivalEntry,
    ) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("save_archival_entry") {
            return Err(Self::fault_error("save_archival_entry"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::Archival {
            agent_id: agent_id.to_string(),
            entry_id: entry.id.clone(),
        });

        let mut archival = self
            .inner
            .archival
            .write()
            .map_err(|_| Self::lock_error("save_archival_entry"))?;
        let agent_entries = archival.entry(agent_id.to_string()).or_default();
        agent_entries.insert(entry.id.clone(), entry.clone());
        drop(archival);

        txn.commit();
        Ok(())
    }

    async fn load_archival_entries(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<ArchivalEntry>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("load_archival_entries") {
            return Err(Self::fault_error("load_archival_entries"));
        }

        let archival = self
            .inner
            .archival
            .read()
            .map_err(|_| Self::lock_error("load_archival_entries"))?;
        let entries: Vec<ArchivalEntry> = archival
            .get(agent_id)
            .map(|m| m.values().take(limit).cloned().collect())
            .unwrap_or_default();
        Ok(entries)
    }

    async fn get_archival_entry(
        &self,
        agent_id: &str,
        entry_id: &str,
    ) -> Result<Option<ArchivalEntry>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("get_archival_entry") {
            return Err(Self::fault_error("get_archival_entry"));
        }

        let archival = self
            .inner
            .archival
            .read()
            .map_err(|_| Self::lock_error("get_archival_entry"))?;
        let entry = archival
            .get(agent_id)
            .and_then(|m| m.get(entry_id).cloned());
        Ok(entry)
    }

    async fn delete_archival_entry(
        &self,
        agent_id: &str,
        entry_id: &str,
    ) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("delete_archival_entry") {
            return Err(Self::fault_error("delete_archival_entry"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::Archival {
            agent_id: agent_id.to_string(),
            entry_id: entry_id.to_string(),
        });

        let mut archival = self
            .inner
            .archival
            .write()
            .map_err(|_| Self::lock_error("delete_archival_entry"))?;
        if let Some(agent_entries) = archival.get_mut(agent_id) {
            agent_entries.remove(entry_id);
        }
        drop(archival);

        txn.commit();
        Ok(())
    }

    async fn delete_archival_entries(&self, agent_id: &str) -> Result<(), StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("delete_archival_entries") {
            return Err(Self::fault_error("delete_archival_entries"));
        }

        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::ArchivalAll(agent_id.to_string()));

        let mut archival = self
            .inner
            .archival
            .write()
            .map_err(|_| Self::lock_error("delete_archival_entries"))?;
        archival.remove(agent_id);
        drop(archival);

        txn.commit();
        Ok(())
    }

    async fn search_archival_entries(
        &self,
        agent_id: &str,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ArchivalEntry>, StorageError> {
        #[cfg(feature = "dst")]
        if self.should_inject_fault("search_archival_entries") {
            return Err(Self::fault_error("search_archival_entries"));
        }

        let archival = self
            .inner
            .archival
            .read()
            .map_err(|_| Self::lock_error("search_archival_entries"))?;

        let entries: Vec<ArchivalEntry> = archival
            .get(agent_id)
            .map(|m| {
                m.values()
                    .filter(|e| {
                        // If no query, return all entries
                        // If query, filter by content containing query (case-insensitive)
                        query.map_or(true, |q| {
                            e.content.to_lowercase().contains(&q.to_lowercase())
                        })
                    })
                    .take(limit)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        Ok(entries)
    }

    // =========================================================================
    // Transactional Operations
    // =========================================================================

    /// Atomic checkpoint: save session state + append message
    ///
    /// TigerStyle: This overrides the default non-atomic implementation to ensure
    /// session and message are saved together atomically. This matches FDB semantics
    /// where checkpoint operations are transactional.
    ///
    /// Implementation acquires both locks before making changes to ensure:
    /// - Either both session AND message are saved, or neither
    /// - No partial reads can see inconsistent state
    /// - Fault injection at any point causes complete rollback
    async fn checkpoint(
        &self,
        session: &SessionState,
        message: Option<&Message>,
    ) -> Result<(), StorageError> {
        // Preconditions
        assert!(!session.agent_id.is_empty(), "agent_id cannot be empty");
        assert!(!session.session_id.is_empty(), "session_id cannot be empty");

        #[cfg(feature = "dst")]
        if self.should_inject_fault("checkpoint") {
            return Err(Self::fault_error("checkpoint"));
        }

        // Start transaction for atomic checkpoint
        let mut txn = self.begin_transaction();
        txn.record_write(StorageKey::Session {
            agent_id: session.agent_id.clone(),
            session_id: session.session_id.clone(),
        });
        if message.is_some() {
            txn.record_write(StorageKey::Messages(session.agent_id.clone()));
        }

        // Acquire BOTH locks BEFORE making any changes to ensure atomicity
        // This prevents a partial checkpoint where session is saved but message is not
        // TigerStyle: Lock ordering is important - always acquire sessions before messages
        // to prevent deadlocks
        let mut all_sessions = self
            .inner
            .sessions
            .write()
            .map_err(|_| Self::lock_error("checkpoint_sessions"))?;

        let mut all_messages = self
            .inner
            .messages
            .write()
            .map_err(|_| Self::lock_error("checkpoint_messages"))?;

        // Now that we have both locks, apply changes atomically
        // If we fail after this point, both changes would be visible (correct)
        // If we had failed before acquiring both locks, no changes would be visible (correct)

        // 1. Save session
        let agent_sessions = all_sessions.entry(session.agent_id.clone()).or_default();
        agent_sessions.insert(session.session_id.clone(), session.clone());

        // 2. Append message if provided
        if let Some(msg) = message {
            // Allow empty agent_id (storage will use session's agent_id) or matching agent_id
            assert!(
                msg.agent_id.is_empty() || msg.agent_id == session.agent_id,
                "message agent_id must be empty or match session agent_id"
            );
            let agent_messages = all_messages.entry(session.agent_id.clone()).or_default();
            agent_messages.push(msg.clone());
        }

        // Release locks
        drop(all_messages);
        drop(all_sessions);

        // Commit transaction to update versions
        txn.commit();

        // Both operations succeeded atomically
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sim_storage_agent_crud() {
        use crate::models::AgentType;
        let storage = SimStorage::new();

        // Create agent
        let agent = AgentMetadata {
            id: "test-agent".to_string(),
            name: "Test Agent".to_string(),
            agent_type: AgentType::MemgptAgent,
            model: Some("claude-3-opus".to_string()),
            embedding: None,
            system: Some("You are a test agent".to_string()),
            description: None,
            tool_ids: vec![],
            tags: vec![],
            metadata: serde_json::Value::Null,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        storage.save_agent(&agent).await.unwrap();

        // Read agent
        let loaded = storage.load_agent("test-agent").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().name, "Test Agent");

        // List agents
        let agents = storage.list_agents().await.unwrap();
        assert_eq!(agents.len(), 1);

        // Delete agent
        storage.delete_agent("test-agent").await.unwrap();
        let loaded = storage.load_agent("test-agent").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_sim_storage_messages() {
        use crate::models::MessageRole;
        let storage = SimStorage::new();

        // Append messages
        let msg1 = Message {
            id: "msg-1".to_string(),
            agent_id: "agent-1".to_string(),
            message_type: "user_message".to_string(),
            role: MessageRole::User,
            content: "Hello".to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            tool_call: None,
            tool_return: None,
            status: None,
            created_at: chrono::Utc::now(),
        };

        let msg2 = Message {
            id: "msg-2".to_string(),
            agent_id: "agent-1".to_string(),
            message_type: "assistant_message".to_string(),
            role: MessageRole::Assistant,
            content: "Hi there!".to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            tool_call: None,
            tool_return: None,
            status: None,
            created_at: chrono::Utc::now(),
        };

        storage.append_message("agent-1", &msg1).await.unwrap();
        storage.append_message("agent-1", &msg2).await.unwrap();

        // Load messages
        let messages = storage.load_messages("agent-1", 10).await.unwrap();
        assert_eq!(messages.len(), 2);

        // Count messages
        let count = storage.count_messages("agent-1").await.unwrap();
        assert_eq!(count, 2);

        // Delete messages
        storage.delete_messages("agent-1").await.unwrap();
        let count = storage.count_messages("agent-1").await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_sim_storage_cascading_delete() {
        use crate::models::AgentType;
        let storage = SimStorage::new();

        // Create agent with blocks and messages
        let agent = AgentMetadata {
            id: "agent-cascade".to_string(),
            name: "Cascade Test".to_string(),
            agent_type: AgentType::MemgptAgent,
            model: None,
            embedding: None,
            system: None,
            description: None,
            tool_ids: vec![],
            tags: vec![],
            metadata: serde_json::Value::Null,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        storage.save_agent(&agent).await.unwrap();

        // Add blocks
        let block = Block {
            id: "block-1".to_string(),
            label: "persona".to_string(),
            value: "I am a test".to_string(),
            description: None,
            limit: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        storage
            .save_blocks("agent-cascade", &[block])
            .await
            .unwrap();

        // Add messages
        let msg = Message {
            id: "msg-cascade".to_string(),
            agent_id: "agent-cascade".to_string(),
            message_type: "user_message".to_string(),
            role: crate::models::MessageRole::User,
            content: "Test".to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            tool_call: None,
            tool_return: None,
            status: None,
            created_at: chrono::Utc::now(),
        };
        storage.append_message("agent-cascade", &msg).await.unwrap();

        // Verify data exists
        assert_eq!(storage.load_blocks("agent-cascade").await.unwrap().len(), 1);
        assert_eq!(storage.count_messages("agent-cascade").await.unwrap(), 1);

        // Delete agent - should cascade
        storage.delete_agent("agent-cascade").await.unwrap();

        // Verify all data is gone
        assert!(storage.load_agent("agent-cascade").await.unwrap().is_none());
        assert_eq!(storage.load_blocks("agent-cascade").await.unwrap().len(), 0);
        assert_eq!(storage.count_messages("agent-cascade").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_sim_storage_atomic_checkpoint() {
        use crate::models::MessageRole;
        let storage = SimStorage::new();

        // Create session and message for atomic checkpoint
        let session = SessionState::new("session-atomic".to_string(), "agent-atomic".to_string());
        let message = Message {
            id: "msg-atomic".to_string(),
            agent_id: "agent-atomic".to_string(),
            message_type: "user_message".to_string(),
            role: MessageRole::User,
            content: "Atomic checkpoint test".to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            tool_call: None,
            tool_return: None,
            status: None,
            created_at: chrono::Utc::now(),
        };

        // Perform atomic checkpoint
        storage.checkpoint(&session, Some(&message)).await.unwrap();

        // Verify BOTH session and message were saved
        let loaded_session = storage
            .load_session("agent-atomic", "session-atomic")
            .await
            .unwrap();
        assert!(
            loaded_session.is_some(),
            "Session should be saved in checkpoint"
        );

        let messages = storage.load_messages("agent-atomic", 10).await.unwrap();
        assert_eq!(messages.len(), 1, "Message should be saved in checkpoint");
        assert_eq!(messages[0].content, "Atomic checkpoint test");
    }

    #[tokio::test]
    async fn test_sim_storage_checkpoint_without_message() {
        let storage = SimStorage::new();

        // Create session without message
        let session = SessionState::new("session-no-msg".to_string(), "agent-no-msg".to_string());

        // Checkpoint with no message
        storage.checkpoint(&session, None).await.unwrap();

        // Verify session was saved
        let loaded_session = storage
            .load_session("agent-no-msg", "session-no-msg")
            .await
            .unwrap();
        assert!(loaded_session.is_some(), "Session should be saved");

        // Verify no messages
        let messages = storage.load_messages("agent-no-msg", 10).await.unwrap();
        assert_eq!(messages.len(), 0, "No messages should exist");
    }

    #[tokio::test]
    async fn test_sim_storage_checkpoint_updates_existing_session() {
        use crate::models::MessageRole;
        let storage = SimStorage::new();

        // Create initial session
        let mut session =
            SessionState::new("session-update".to_string(), "agent-update".to_string());

        // First checkpoint
        storage.checkpoint(&session, None).await.unwrap();
        let initial = storage
            .load_session("agent-update", "session-update")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(initial.iteration, 0);

        // Advance iteration
        session.advance_iteration();

        // Second checkpoint with message
        let message = Message {
            id: "msg-update".to_string(),
            agent_id: "agent-update".to_string(),
            message_type: "assistant_message".to_string(),
            role: MessageRole::Assistant,
            content: "Updated checkpoint".to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            tool_call: None,
            tool_return: None,
            status: None,
            created_at: chrono::Utc::now(),
        };
        storage.checkpoint(&session, Some(&message)).await.unwrap();

        // Verify session was updated
        let updated = storage
            .load_session("agent-update", "session-update")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.iteration, 1, "Session iteration should be updated");

        // Verify message was appended
        let messages = storage.load_messages("agent-update", 10).await.unwrap();
        assert_eq!(messages.len(), 1, "Message should be appended");
    }
}
