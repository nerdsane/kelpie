//! KV Adapter for AgentStorage
//!
//! TigerStyle: Structural adapter mapping AgentStorage trait onto byte-level ActorKV.
//!
//! This adapter enables kelpie-server to use the proper kelpie-dst infrastructure
//! for deterministic simulation testing, replacing the ad-hoc SimStorage implementation.
//!
//! ## Key Mapping Strategy
//!
//! All keys are scoped under a single ActorId("kelpie", "server") namespace:
//! - `agents/{id}` -> JSON-serialized AgentMetadata
//! - `sessions/{agent_id}/{session_id}` -> JSON-serialized SessionState
//! - `messages/{agent_id}/{message_id}` -> JSON-serialized Message
//! - `blocks/{agent_id}` -> JSON-serialized `Vec<Block>`
//! - `tools/{name}` -> JSON-serialized CustomToolRecord

use async_trait::async_trait;
use kelpie_core::ActorId;
use kelpie_storage::ActorKV;
use std::sync::Arc;

use crate::models::{ArchivalEntry, Block, Message};

use super::traits::{AgentStorage, StorageError};
use super::types::{AgentMetadata, CustomToolRecord, SessionState};

/// Maximum key length in bytes
const KEY_LENGTH_BYTES_MAX: usize = 256;

/// Maximum value size in bytes (10 MB)
const VALUE_SIZE_BYTES_MAX: usize = 10 * 1024 * 1024;

/// Adapter that wraps ActorKV and implements AgentStorage
///
/// TigerStyle: Explicit scoping, bounded keys, JSON serialization for debuggability.
pub struct KvAdapter {
    /// Underlying key-value store (SimStorage, MemoryKV, or FdbKV)
    kv: Arc<dyn ActorKV>,
    /// Actor ID used as namespace for all server storage
    actor_id: ActorId,
}

impl KvAdapter {
    /// Create a new KvAdapter wrapping the given ActorKV
    ///
    /// All storage operations will be scoped under ActorId("kelpie", "server").
    pub fn new(kv: Arc<dyn ActorKV>) -> Self {
        let actor_id = ActorId::new("kelpie", "server")
            .expect("failed to create server actor id - this is a bug");

        Self { kv, actor_id }
    }

    /// Create a KvAdapter backed by MemoryKV (for testing)
    ///
    /// This is a convenience method for creating in-memory storage for unit tests.
    pub fn with_memory() -> Self {
        use kelpie_storage::memory::MemoryKV;
        let kv: Arc<dyn ActorKV> = Arc::new(MemoryKV::new());
        Self::new(kv)
    }

    /// Create a KvAdapter backed by SimStorage (for DST testing)
    ///
    /// This connects the server to the proper kelpie-dst infrastructure with
    /// fault injection and deterministic behavior.
    ///
    /// # Arguments
    /// * `rng` - Deterministic RNG from kelpie-dst
    /// * `fault_injector` - FaultInjector for simulating failures
    #[cfg(feature = "dst")]
    pub fn with_dst_storage(
        rng: kelpie_dst::DeterministicRng,
        fault_injector: std::sync::Arc<kelpie_dst::FaultInjector>,
    ) -> Self {
        use kelpie_dst::SimStorage;
        let storage = SimStorage::new(rng, fault_injector);
        let kv: Arc<dyn ActorKV> = Arc::new(storage);
        Self::new(kv)
    }

    /// Get the underlying ActorKV (for testing)
    #[cfg(test)]
    pub fn underlying_kv(&self) -> Arc<dyn ActorKV> {
        self.kv.clone()
    }

    // =========================================================================
    // Key Mapping Functions
    // =========================================================================

    /// Generate key for agent metadata: `agents/{id}`
    fn agent_key(id: &str) -> Vec<u8> {
        assert!(!id.is_empty(), "agent id cannot be empty");
        let key = format!("agents/{}", id);
        assert!(
            key.len() <= KEY_LENGTH_BYTES_MAX,
            "agent key too long: {} bytes",
            key.len()
        );
        key.into_bytes()
    }

    /// Generate key for session state: `sessions/{agent_id}/{session_id}`
    fn session_key(agent_id: &str, session_id: &str) -> Vec<u8> {
        assert!(!agent_id.is_empty(), "agent id cannot be empty");
        assert!(!session_id.is_empty(), "session id cannot be empty");
        // TigerStyle: Sessions are stored in the agent's namespace
        format!("session:{}", session_id).into_bytes()
    }

    /// Generate prefix for listing sessions: `sessions/{agent_id}/`
    fn session_prefix(_agent_id: &str) -> Vec<u8> {
        // TigerStyle: Sessions are stored in the agent's namespace with prefix "session:"
        b"session:".to_vec()
    }

    /// Generate key for message: `message:{message_id}`
    fn message_key(_agent_id: &str, message_id: &str) -> Vec<u8> {
        // TigerStyle: Messages are stored in the agent's namespace
        format!("message:{}", message_id).into_bytes()
    }

    /// Generate prefix for listing messages: `message:`
    fn message_prefix(_agent_id: &str) -> Vec<u8> {
        // TigerStyle: Messages are stored in the agent's namespace with prefix "message:"
        b"message:".to_vec()
    }

    /// Generate key for blocks: `blocks/{agent_id}`
    fn blocks_key(_agent_id: &str) -> Vec<u8> {
        // TigerStyle: Blocks are stored in the agent's namespace
        b"blocks".to_vec()
    }

    /// Generate key for custom tool: `tools/{name}`
    fn tool_key(name: &str) -> Vec<u8> {
        assert!(!name.is_empty(), "tool name cannot be empty");
        let key = format!("tools/{}", name);
        assert!(
            key.len() <= KEY_LENGTH_BYTES_MAX,
            "tool key too long: {} bytes",
            key.len()
        );
        key.into_bytes()
    }

    /// Generate key for MCP server: `mcp_servers/{id}`
    fn mcp_server_key(id: &str) -> Vec<u8> {
        assert!(!id.is_empty(), "mcp server id cannot be empty");
        let key = format!("mcp_servers/{}", id);
        assert!(
            key.len() <= KEY_LENGTH_BYTES_MAX,
            "mcp server key too long: {} bytes",
            key.len()
        );
        key.into_bytes()
    }

    /// Generate key for agent group: `agent_groups/{id}`
    fn agent_group_key(id: &str) -> Vec<u8> {
        assert!(!id.is_empty(), "agent group id cannot be empty");
        let key = format!("agent_groups/{}", id);
        assert!(
            key.len() <= KEY_LENGTH_BYTES_MAX,
            "agent group key too long: {} bytes",
            key.len()
        );
        key.into_bytes()
    }

    /// Generate key for identity: `identities/{id}`
    fn identity_key(id: &str) -> Vec<u8> {
        assert!(!id.is_empty(), "identity id cannot be empty");
        let key = format!("identities/{}", id);
        assert!(
            key.len() <= KEY_LENGTH_BYTES_MAX,
            "identity key too long: {} bytes",
            key.len()
        );
        key.into_bytes()
    }

    /// Generate key for project: `projects/{id}`
    fn project_key(id: &str) -> Vec<u8> {
        assert!(!id.is_empty(), "project id cannot be empty");
        let key = format!("projects/{}", id);
        assert!(
            key.len() <= KEY_LENGTH_BYTES_MAX,
            "project key too long: {} bytes",
            key.len()
        );
        key.into_bytes()
    }

    /// Generate key for job: `jobs/{id}`
    fn job_key(id: &str) -> Vec<u8> {
        assert!(!id.is_empty(), "job id cannot be empty");
        let key = format!("jobs/{}", id);
        assert!(
            key.len() <= KEY_LENGTH_BYTES_MAX,
            "job key too long: {} bytes",
            key.len()
        );
        key.into_bytes()
    }

    /// Generate key for archival entry: `archival:{entry_id}`
    fn archival_key(_agent_id: &str, entry_id: &str) -> Vec<u8> {
        assert!(!entry_id.is_empty(), "entry id cannot be empty");
        // TigerStyle: Archival entries are stored in the agent's namespace
        format!("archival:{}", entry_id).into_bytes()
    }

    /// Generate prefix for listing archival entries: `archival:`
    fn archival_prefix(_agent_id: &str) -> Vec<u8> {
        // TigerStyle: Archival entries are stored in the agent's namespace with prefix "archival:"
        b"archival:".to_vec()
    }

    // =========================================================================
    // Serialization Helpers
    // =========================================================================

    /// Serialize a value to JSON bytes
    fn serialize<T: serde::Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, StorageError> {
        let json = serde_json::to_vec(value).map_err(|e| StorageError::SerializationFailed {
            reason: e.to_string(),
        })?;

        assert!(
            json.len() <= VALUE_SIZE_BYTES_MAX,
            "value too large: {} bytes (max {})",
            json.len(),
            VALUE_SIZE_BYTES_MAX
        );

        Ok(json)
    }

    /// Deserialize JSON bytes to a value
    fn deserialize<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, StorageError> {
        serde_json::from_slice(bytes).map_err(|e| StorageError::DeserializationFailed {
            reason: e.to_string(),
        })
    }

    /// Map kelpie_core::Error to StorageError
    fn map_kv_error(operation: &str, err: kelpie_core::Error) -> StorageError {
        match err {
            kelpie_core::Error::StorageReadFailed { key, reason } => {
                // Check if this is a fault-injected error
                #[cfg(feature = "dst")]
                if reason.contains("(injected)") {
                    return StorageError::FaultInjected {
                        operation: format!("{}: read {}", operation, key),
                    };
                }
                StorageError::ReadFailed {
                    operation: format!("{}: {}", operation, key),
                    reason,
                }
            }
            kelpie_core::Error::StorageWriteFailed { key, reason } => {
                // Check if this is a fault-injected error
                #[cfg(feature = "dst")]
                if reason.contains("(injected)") {
                    return StorageError::FaultInjected {
                        operation: format!("{}: write {}", operation, key),
                    };
                }
                StorageError::WriteFailed {
                    operation: format!("{}: {}", operation, key),
                    reason,
                }
            }
            kelpie_core::Error::Internal { message } => {
                // Check if this is a fault-injected error
                #[cfg(feature = "dst")]
                if message.contains("(injected)") {
                    return StorageError::FaultInjected {
                        operation: operation.to_string(),
                    };
                }
                StorageError::Internal { message }
            }
            _ => StorageError::Internal {
                message: format!("{}: {}", operation, err),
            },
        }
    }
}

#[async_trait]
impl AgentStorage for KvAdapter {
    // =========================================================================
    // Agent Metadata Operations
    // =========================================================================

    async fn save_agent(&self, agent: &AgentMetadata) -> Result<(), StorageError> {
        // Preconditions
        assert!(!agent.id.is_empty(), "agent id cannot be empty");

        let key = Self::agent_key(&agent.id);
        let value = Self::serialize(agent)?;

        self.kv
            .set(&self.actor_id, &key, &value)
            .await
            .map_err(|e| Self::map_kv_error("save_agent", e))?;

        Ok(())
    }

    async fn load_agent(&self, id: &str) -> Result<Option<AgentMetadata>, StorageError> {
        // Preconditions
        assert!(!id.is_empty(), "agent id cannot be empty");

        let key = Self::agent_key(id);

        let bytes = self
            .kv
            .get(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("load_agent", e))?;

        match bytes {
            Some(b) => {
                let agent = Self::deserialize(&b)?;
                Ok(Some(agent))
            }
            None => Ok(None),
        }
    }

    async fn delete_agent(&self, id: &str) -> Result<(), StorageError> {
        // Preconditions
        assert!(!id.is_empty(), "agent id cannot be empty");

        // Check if agent exists first
        if !self.agent_exists(id).await? {
            return Err(StorageError::NotFound {
                resource: "agent",
                id: id.to_string(),
            });
        }

        // IMPORTANT: Delete children FIRST, then agent metadata LAST.
        // This ensures atomicity - if any child delete fails, the agent still exists
        // and the operation can be retried. If we deleted the agent first and a child
        // delete failed, we'd have orphaned data with no parent agent.

        // 1. Delete associated messages first
        // Note: Message keys are `message:{message_id}` (not scoped by agent_id in key),
        // so we need to scan all messages and filter by agent_id from the value.
        let message_prefix = Self::message_prefix(id);
        let message_pairs = self
            .kv
            .scan_prefix(&self.actor_id, &message_prefix)
            .await
            .map_err(|e| Self::map_kv_error("delete_agent_messages", e))?;

        for (key, value) in message_pairs {
            // Deserialize to check if this message belongs to this agent
            if let Ok(message) = Self::deserialize::<Message>(&value) {
                if message.agent_id == id {
                    self.kv
                        .delete(&self.actor_id, &key)
                        .await
                        .map_err(|e| Self::map_kv_error("delete_message", e))?;
                }
            }
        }

        // 2. Delete associated sessions
        // Note: Session keys are `session:{session_id}` (not scoped by agent_id in key),
        // so we need to scan all sessions and filter by agent_id from the value.
        let session_prefix = Self::session_prefix(id);
        let session_pairs = self
            .kv
            .scan_prefix(&self.actor_id, &session_prefix)
            .await
            .map_err(|e| Self::map_kv_error("delete_agent_sessions", e))?;

        for (key, value) in session_pairs {
            // Deserialize to check if this session belongs to this agent
            if let Ok(session) = Self::deserialize::<SessionState>(&value) {
                if session.agent_id == id {
                    self.kv
                        .delete(&self.actor_id, &key)
                        .await
                        .map_err(|e| Self::map_kv_error("delete_session", e))?;
                }
            }
        }

        // 3. Delete associated blocks
        let blocks_key = Self::blocks_key(id);
        // Note: blocks may not exist, so we handle NotFound gracefully
        match self.kv.delete(&self.actor_id, &blocks_key).await {
            Ok(()) => {}
            Err(e) => {
                // Only propagate if it's not a "key not found" type error
                // Most KV stores return Ok(()) for deleting non-existent keys,
                // but we handle both cases to be safe
                let err_str = format!("{:?}", e);
                if !err_str.contains("NotFound") && !err_str.contains("not found") {
                    return Err(Self::map_kv_error("delete_blocks", e));
                }
            }
        }

        // 4. Delete agent metadata LAST (after all children are deleted)
        let agent_key = Self::agent_key(id);
        self.kv
            .delete(&self.actor_id, &agent_key)
            .await
            .map_err(|e| Self::map_kv_error("delete_agent_metadata", e))?;

        Ok(())
    }

    async fn list_agents(&self) -> Result<Vec<AgentMetadata>, StorageError> {
        let prefix = b"agents/";
        let pairs = self
            .kv
            .scan_prefix(&self.actor_id, prefix)
            .await
            .map_err(|e| Self::map_kv_error("list_agents", e))?;

        let mut agents = Vec::with_capacity(pairs.len());
        for (_key, value) in pairs {
            let agent = Self::deserialize(&value)?;
            agents.push(agent);
        }

        Ok(agents)
    }

    // =========================================================================
    // Core Memory Block Operations
    // =========================================================================

    async fn save_blocks(&self, agent_id: &str, blocks: &[Block]) -> Result<(), StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");

        // Verify agent exists
        if !self.agent_exists(agent_id).await? {
            return Err(StorageError::NotFound {
                resource: "agent",
                id: agent_id.to_string(),
            });
        }

        let key = Self::blocks_key(agent_id);
        let value = Self::serialize(blocks)?;

        self.kv
            .set(&self.actor_id, &key, &value)
            .await
            .map_err(|e| Self::map_kv_error("save_blocks", e))?;

        Ok(())
    }

    async fn load_blocks(&self, agent_id: &str) -> Result<Vec<Block>, StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");

        let key = Self::blocks_key(agent_id);

        let bytes = self
            .kv
            .get(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("load_blocks", e))?;

        match bytes {
            Some(b) => {
                let blocks = Self::deserialize(&b)?;
                Ok(blocks)
            }
            None => Ok(Vec::new()), // No blocks = empty vec
        }
    }

    async fn update_block(
        &self,
        agent_id: &str,
        label: &str,
        value: &str,
    ) -> Result<Block, StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");
        assert!(!label.is_empty(), "label cannot be empty");

        let mut blocks = self.load_blocks(agent_id).await?;

        // Find and update block
        for block in blocks.iter_mut() {
            if block.label == label {
                block.value = value.to_string();
                block.updated_at = chrono::Utc::now();
                let result = block.clone();

                // Save updated blocks (after cloning)
                self.save_blocks(agent_id, &blocks).await?;

                return Ok(result);
            }
        }

        Err(StorageError::NotFound {
            resource: "block",
            id: label.to_string(),
        })
    }

    async fn append_block(
        &self,
        agent_id: &str,
        label: &str,
        content: &str,
    ) -> Result<Block, StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");
        assert!(!label.is_empty(), "label cannot be empty");

        let mut blocks = self.load_blocks(agent_id).await?;

        // Find existing block or create new
        for block in blocks.iter_mut() {
            if block.label == label {
                block.value.push_str(content);
                block.updated_at = chrono::Utc::now();
                let result = block.clone();

                // Save updated blocks (after cloning)
                self.save_blocks(agent_id, &blocks).await?;

                return Ok(result);
            }
        }

        // Create new block
        let block = Block::new(label, content);
        blocks.push(block.clone());
        self.save_blocks(agent_id, &blocks).await?;

        Ok(block)
    }

    // =========================================================================
    // Session State Operations
    // =========================================================================

    async fn save_session(&self, state: &SessionState) -> Result<(), StorageError> {
        // Preconditions
        assert!(!state.agent_id.is_empty(), "agent id cannot be empty");
        assert!(!state.session_id.is_empty(), "session id cannot be empty");

        let key = Self::session_key(&state.agent_id, &state.session_id);
        let value = Self::serialize(state)?;

        self.kv
            .set(&self.actor_id, &key, &value)
            .await
            .map_err(|e| Self::map_kv_error("save_session", e))?;

        Ok(())
    }

    async fn load_session(
        &self,
        agent_id: &str,
        session_id: &str,
    ) -> Result<Option<SessionState>, StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");
        assert!(!session_id.is_empty(), "session id cannot be empty");

        let key = Self::session_key(agent_id, session_id);

        let bytes = self
            .kv
            .get(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("load_session", e))?;

        match bytes {
            Some(b) => {
                let session = Self::deserialize(&b)?;
                Ok(Some(session))
            }
            None => Ok(None),
        }
    }

    async fn delete_session(&self, agent_id: &str, session_id: &str) -> Result<(), StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");
        assert!(!session_id.is_empty(), "session id cannot be empty");

        let key = Self::session_key(agent_id, session_id);

        // Check if exists
        if !self
            .kv
            .exists(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("delete_session_exists_check", e))?
        {
            return Err(StorageError::NotFound {
                resource: "session",
                id: session_id.to_string(),
            });
        }

        self.kv
            .delete(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("delete_session", e))?;

        Ok(())
    }

    async fn list_sessions(&self, agent_id: &str) -> Result<Vec<SessionState>, StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");

        // Note: Session keys are `session:{session_id}` (not scoped by agent_id in key),
        // so we need to scan all sessions and filter by agent_id from the value.
        let prefix = Self::session_prefix(agent_id);
        let pairs = self
            .kv
            .scan_prefix(&self.actor_id, &prefix)
            .await
            .map_err(|e| Self::map_kv_error("list_sessions", e))?;

        let mut sessions = Vec::new();
        for (_key, value) in pairs {
            let session: SessionState = Self::deserialize(&value)?;
            // Filter by agent_id since keys don't include agent scope
            if session.agent_id == agent_id {
                sessions.push(session);
            }
        }

        Ok(sessions)
    }

    // =========================================================================
    // Message Operations
    // =========================================================================

    async fn append_message(&self, agent_id: &str, message: &Message) -> Result<(), StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");
        assert!(!message.id.is_empty(), "message id cannot be empty");
        assert_eq!(
            message.agent_id, agent_id,
            "message agent_id must match parameter"
        );

        let key = Self::message_key(agent_id, &message.id);
        let value = Self::serialize(message)?;

        self.kv
            .set(&self.actor_id, &key, &value)
            .await
            .map_err(|e| Self::map_kv_error("append_message", e))?;

        Ok(())
    }

    async fn load_messages(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<Message>, StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");
        assert!(limit > 0, "limit must be positive");
        assert!(limit <= 10000, "limit too large: {}", limit);

        // Note: Message keys are `message:{message_id}` (not scoped by agent_id in key),
        // so we need to scan all messages and filter by agent_id from the value.
        let prefix = Self::message_prefix(agent_id);

        let pairs = self
            .kv
            .scan_prefix(&self.actor_id, &prefix)
            .await
            .map_err(|e| Self::map_kv_error("load_messages", e))?;

        let mut messages = Vec::new();
        for (_key, value) in pairs {
            let message: Message = Self::deserialize(&value)?;
            // Filter by agent_id since keys don't include agent scope
            if message.agent_id == agent_id {
                messages.push(message);
            }
        }

        // Sort by created_at (oldest first)
        messages.sort_by_key(|m| m.created_at);

        // Return most recent messages (last `limit` items)
        let start = messages.len().saturating_sub(limit);
        Ok(messages[start..].to_vec())
    }

    async fn load_messages_since(
        &self,
        agent_id: &str,
        since_ms: u64,
    ) -> Result<Vec<Message>, StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");

        // Note: Message keys are `message:{message_id}` (not scoped by agent_id in key),
        // so we need to scan all messages and filter by agent_id from the value.
        let prefix = Self::message_prefix(agent_id);
        let pairs = self
            .kv
            .scan_prefix(&self.actor_id, &prefix)
            .await
            .map_err(|e| Self::map_kv_error("load_messages_since", e))?;

        let mut messages = Vec::new();
        for (_key, value) in pairs {
            let message: Message = Self::deserialize(&value)?;
            // Filter by agent_id since keys don't include agent scope
            if message.agent_id == agent_id
                && message.created_at.timestamp_millis() as u64 > since_ms
            {
                messages.push(message);
            }
        }

        // Sort by created_at (oldest first)
        messages.sort_by_key(|m| m.created_at);

        Ok(messages)
    }

    async fn count_messages(&self, agent_id: &str) -> Result<usize, StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");

        // Note: Message keys are `message:{message_id}` (not scoped by agent_id in key),
        // so we need to scan all messages and count by agent_id from the value.
        let prefix = Self::message_prefix(agent_id);

        let pairs = self
            .kv
            .scan_prefix(&self.actor_id, &prefix)
            .await
            .map_err(|e| Self::map_kv_error("count_messages", e))?;

        let mut count = 0;
        for (_key, value) in pairs {
            if let Ok(message) = Self::deserialize::<Message>(&value) {
                if message.agent_id == agent_id {
                    count += 1;
                }
            }
        }

        Ok(count)
    }

    async fn delete_messages(&self, agent_id: &str) -> Result<(), StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");

        // Note: Message keys are `message:{message_id}` (not scoped by agent_id in key),
        // so we need to scan all messages and filter by agent_id from the value.
        let prefix = Self::message_prefix(agent_id);

        let pairs = self
            .kv
            .scan_prefix(&self.actor_id, &prefix)
            .await
            .map_err(|e| Self::map_kv_error("delete_messages", e))?;

        for (key, value) in pairs {
            // Deserialize to check if this message belongs to this agent
            if let Ok(message) = Self::deserialize::<Message>(&value) {
                if message.agent_id == agent_id {
                    let _ = self.kv.delete(&self.actor_id, &key).await; // Continue on error
                }
            }
        }

        Ok(())
    }

    // =========================================================================
    // Custom Tool Operations
    // =========================================================================

    async fn save_custom_tool(&self, tool: &CustomToolRecord) -> Result<(), StorageError> {
        // Preconditions
        assert!(!tool.name.is_empty(), "tool name cannot be empty");

        let key = Self::tool_key(&tool.name);
        let value = Self::serialize(tool)?;

        self.kv
            .set(&self.actor_id, &key, &value)
            .await
            .map_err(|e| Self::map_kv_error("save_custom_tool", e))?;

        Ok(())
    }

    async fn load_custom_tool(&self, name: &str) -> Result<Option<CustomToolRecord>, StorageError> {
        // Preconditions
        assert!(!name.is_empty(), "tool name cannot be empty");

        let key = Self::tool_key(name);

        let bytes = self
            .kv
            .get(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("load_custom_tool", e))?;

        match bytes {
            Some(b) => {
                let tool = Self::deserialize(&b)?;
                Ok(Some(tool))
            }
            None => Ok(None),
        }
    }

    async fn delete_custom_tool(&self, name: &str) -> Result<(), StorageError> {
        // Preconditions
        assert!(!name.is_empty(), "tool name cannot be empty");

        let key = Self::tool_key(name);

        // Check if exists
        if !self
            .kv
            .exists(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("delete_custom_tool_exists_check", e))?
        {
            return Err(StorageError::NotFound {
                resource: "tool",
                id: name.to_string(),
            });
        }

        self.kv
            .delete(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("delete_custom_tool", e))?;

        Ok(())
    }

    async fn list_custom_tools(&self) -> Result<Vec<CustomToolRecord>, StorageError> {
        let prefix = b"tools/";
        let pairs = self
            .kv
            .scan_prefix(&self.actor_id, prefix)
            .await
            .map_err(|e| Self::map_kv_error("list_custom_tools", e))?;

        let mut tools = Vec::with_capacity(pairs.len());
        for (_key, value) in pairs {
            let tool = Self::deserialize(&value)?;
            tools.push(tool);
        }

        Ok(tools)
    }

    // =========================================================================
    // Transactional Operations
    // =========================================================================

    async fn checkpoint(
        &self,
        session: &SessionState,
        message: Option<&Message>,
    ) -> Result<(), StorageError> {
        // Preconditions
        assert!(
            !session.agent_id.is_empty(),
            "session agent_id cannot be empty"
        );
        assert!(
            !session.session_id.is_empty(),
            "session session_id cannot be empty"
        );

        if let Some(msg) = message {
            assert!(!msg.id.is_empty(), "message id cannot be empty");
            assert_eq!(
                msg.agent_id, session.agent_id,
                "message agent_id must match session agent_id"
            );
        }

        // Use ActorKV transaction for atomicity
        let mut txn = self
            .kv
            .begin_transaction(&self.actor_id)
            .await
            .map_err(|e| Self::map_kv_error("checkpoint_begin_txn", e))?;

        // 1. Save session state
        let session_key = Self::session_key(&session.agent_id, &session.session_id);
        let session_value = Self::serialize(session)?;
        txn.set(&session_key, &session_value)
            .await
            .map_err(|e| Self::map_kv_error("checkpoint_save_session", e))?;

        // 2. Append message if present
        if let Some(msg) = message {
            let message_key = Self::message_key(&session.agent_id, &msg.id);
            let message_value = Self::serialize(msg)?;
            txn.set(&message_key, &message_value)
                .await
                .map_err(|e| Self::map_kv_error("checkpoint_save_message", e))?;
        }

        // 3. Commit transaction atomically
        txn.commit()
            .await
            .map_err(|e| Self::map_kv_error("checkpoint_commit", e))?;

        Ok(())
    }

    // =========================================================================
    // MCP Server Operations
    // =========================================================================

    async fn save_mcp_server(&self, server: &crate::models::MCPServer) -> Result<(), StorageError> {
        // Preconditions
        assert!(!server.id.is_empty(), "mcp server id cannot be empty");

        let key = Self::mcp_server_key(&server.id);
        let value = Self::serialize(server)?;

        self.kv
            .set(&self.actor_id, &key, &value)
            .await
            .map_err(|e| Self::map_kv_error("save_mcp_server", e))?;

        Ok(())
    }

    async fn load_mcp_server(
        &self,
        id: &str,
    ) -> Result<Option<crate::models::MCPServer>, StorageError> {
        // Preconditions
        assert!(!id.is_empty(), "mcp server id cannot be empty");

        let key = Self::mcp_server_key(id);

        let bytes = self
            .kv
            .get(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("load_mcp_server", e))?;

        match bytes {
            Some(b) => {
                let server = Self::deserialize(&b)?;
                Ok(Some(server))
            }
            None => Ok(None),
        }
    }

    async fn delete_mcp_server(&self, id: &str) -> Result<(), StorageError> {
        // Preconditions
        assert!(!id.is_empty(), "mcp server id cannot be empty");

        let key = Self::mcp_server_key(id);

        // Check if exists
        if !self
            .kv
            .exists(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("delete_mcp_server_exists_check", e))?
        {
            return Err(StorageError::NotFound {
                resource: "mcp_server",
                id: id.to_string(),
            });
        }

        self.kv
            .delete(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("delete_mcp_server", e))?;

        Ok(())
    }

    async fn list_mcp_servers(&self) -> Result<Vec<crate::models::MCPServer>, StorageError> {
        let prefix = b"mcp_servers/";
        let pairs = self
            .kv
            .scan_prefix(&self.actor_id, prefix)
            .await
            .map_err(|e| Self::map_kv_error("list_mcp_servers", e))?;

        let mut servers = Vec::with_capacity(pairs.len());
        for (_key, value) in pairs {
            let server = Self::deserialize(&value)?;
            servers.push(server);
        }

        Ok(servers)
    }

    // =========================================================================
    // Agent Group Operations
    // =========================================================================

    async fn save_agent_group(
        &self,
        group: &crate::models::AgentGroup,
    ) -> Result<(), StorageError> {
        // Preconditions
        assert!(!group.id.is_empty(), "agent group id cannot be empty");

        let key = Self::agent_group_key(&group.id);
        let value = Self::serialize(group)?;

        self.kv
            .set(&self.actor_id, &key, &value)
            .await
            .map_err(|e| Self::map_kv_error("save_agent_group", e))?;

        Ok(())
    }

    async fn load_agent_group(
        &self,
        id: &str,
    ) -> Result<Option<crate::models::AgentGroup>, StorageError> {
        // Preconditions
        assert!(!id.is_empty(), "agent group id cannot be empty");

        let key = Self::agent_group_key(id);

        let bytes = self
            .kv
            .get(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("load_agent_group", e))?;

        match bytes {
            Some(b) => {
                let group = Self::deserialize(&b)?;
                Ok(Some(group))
            }
            None => Ok(None),
        }
    }

    async fn delete_agent_group(&self, id: &str) -> Result<(), StorageError> {
        // Preconditions
        assert!(!id.is_empty(), "agent group id cannot be empty");

        let key = Self::agent_group_key(id);

        // Check if exists
        if !self
            .kv
            .exists(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("delete_agent_group_exists_check", e))?
        {
            return Err(StorageError::NotFound {
                resource: "agent_group",
                id: id.to_string(),
            });
        }

        self.kv
            .delete(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("delete_agent_group", e))?;

        Ok(())
    }

    async fn list_agent_groups(&self) -> Result<Vec<crate::models::AgentGroup>, StorageError> {
        let prefix = b"agent_groups/";
        let pairs = self
            .kv
            .scan_prefix(&self.actor_id, prefix)
            .await
            .map_err(|e| Self::map_kv_error("list_agent_groups", e))?;

        let mut groups = Vec::with_capacity(pairs.len());
        for (_key, value) in pairs {
            let group = Self::deserialize(&value)?;
            groups.push(group);
        }

        Ok(groups)
    }

    // =========================================================================
    // Identity Operations
    // =========================================================================

    async fn save_identity(&self, identity: &crate::models::Identity) -> Result<(), StorageError> {
        // Preconditions
        assert!(!identity.id.is_empty(), "identity id cannot be empty");

        let key = Self::identity_key(&identity.id);
        let value = Self::serialize(identity)?;

        self.kv
            .set(&self.actor_id, &key, &value)
            .await
            .map_err(|e| Self::map_kv_error("save_identity", e))?;

        Ok(())
    }

    async fn load_identity(
        &self,
        id: &str,
    ) -> Result<Option<crate::models::Identity>, StorageError> {
        // Preconditions
        assert!(!id.is_empty(), "identity id cannot be empty");

        let key = Self::identity_key(id);

        let bytes = self
            .kv
            .get(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("load_identity", e))?;

        match bytes {
            Some(b) => {
                let identity = Self::deserialize(&b)?;
                Ok(Some(identity))
            }
            None => Ok(None),
        }
    }

    async fn delete_identity(&self, id: &str) -> Result<(), StorageError> {
        // Preconditions
        assert!(!id.is_empty(), "identity id cannot be empty");

        let key = Self::identity_key(id);

        // Check if exists
        if !self
            .kv
            .exists(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("delete_identity_exists_check", e))?
        {
            return Err(StorageError::NotFound {
                resource: "identity",
                id: id.to_string(),
            });
        }

        self.kv
            .delete(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("delete_identity", e))?;

        Ok(())
    }

    async fn list_identities(&self) -> Result<Vec<crate::models::Identity>, StorageError> {
        let prefix = b"identities/";
        let pairs = self
            .kv
            .scan_prefix(&self.actor_id, prefix)
            .await
            .map_err(|e| Self::map_kv_error("list_identities", e))?;

        let mut identities = Vec::with_capacity(pairs.len());
        for (_key, value) in pairs {
            let identity = Self::deserialize(&value)?;
            identities.push(identity);
        }

        Ok(identities)
    }

    // =========================================================================
    // Project Operations
    // =========================================================================

    async fn save_project(&self, project: &crate::models::Project) -> Result<(), StorageError> {
        // Preconditions
        assert!(!project.id.is_empty(), "project id cannot be empty");

        let key = Self::project_key(&project.id);
        let value = Self::serialize(project)?;

        self.kv
            .set(&self.actor_id, &key, &value)
            .await
            .map_err(|e| Self::map_kv_error("save_project", e))?;

        Ok(())
    }

    async fn load_project(&self, id: &str) -> Result<Option<crate::models::Project>, StorageError> {
        // Preconditions
        assert!(!id.is_empty(), "project id cannot be empty");

        let key = Self::project_key(id);

        let bytes = self
            .kv
            .get(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("load_project", e))?;

        match bytes {
            Some(b) => {
                let project = Self::deserialize(&b)?;
                Ok(Some(project))
            }
            None => Ok(None),
        }
    }

    async fn delete_project(&self, id: &str) -> Result<(), StorageError> {
        // Preconditions
        assert!(!id.is_empty(), "project id cannot be empty");

        let key = Self::project_key(id);

        // Check if exists
        if !self
            .kv
            .exists(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("delete_project_exists_check", e))?
        {
            return Err(StorageError::NotFound {
                resource: "project",
                id: id.to_string(),
            });
        }

        self.kv
            .delete(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("delete_project", e))?;

        Ok(())
    }

    async fn list_projects(&self) -> Result<Vec<crate::models::Project>, StorageError> {
        let prefix = b"projects/";
        let pairs = self
            .kv
            .scan_prefix(&self.actor_id, prefix)
            .await
            .map_err(|e| Self::map_kv_error("list_projects", e))?;

        let mut projects = Vec::with_capacity(pairs.len());
        for (_key, value) in pairs {
            let project = Self::deserialize(&value)?;
            projects.push(project);
        }

        Ok(projects)
    }

    // =========================================================================
    // Job Operations
    // =========================================================================

    async fn save_job(&self, job: &crate::models::Job) -> Result<(), StorageError> {
        // Preconditions
        assert!(!job.id.is_empty(), "job id cannot be empty");

        let key = Self::job_key(&job.id);
        let value = Self::serialize(job)?;

        self.kv
            .set(&self.actor_id, &key, &value)
            .await
            .map_err(|e| Self::map_kv_error("save_job", e))?;

        Ok(())
    }

    async fn load_job(&self, id: &str) -> Result<Option<crate::models::Job>, StorageError> {
        // Preconditions
        assert!(!id.is_empty(), "job id cannot be empty");

        let key = Self::job_key(id);

        let bytes = self
            .kv
            .get(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("load_job", e))?;

        match bytes {
            Some(b) => {
                let job = Self::deserialize(&b)?;
                Ok(Some(job))
            }
            None => Ok(None),
        }
    }

    async fn delete_job(&self, id: &str) -> Result<(), StorageError> {
        // Preconditions
        assert!(!id.is_empty(), "job id cannot be empty");

        let key = Self::job_key(id);

        // Check if exists
        if !self
            .kv
            .exists(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("delete_job_exists_check", e))?
        {
            return Err(StorageError::NotFound {
                resource: "job",
                id: id.to_string(),
            });
        }

        self.kv
            .delete(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("delete_job", e))?;

        Ok(())
    }

    async fn list_jobs(&self) -> Result<Vec<crate::models::Job>, StorageError> {
        let prefix = b"jobs/";
        let pairs = self
            .kv
            .scan_prefix(&self.actor_id, prefix)
            .await
            .map_err(|e| Self::map_kv_error("list_jobs", e))?;

        let mut jobs = Vec::with_capacity(pairs.len());
        for (_key, value) in pairs {
            let job = Self::deserialize(&value)?;
            jobs.push(job);
        }

        Ok(jobs)
    }

    // =========================================================================
    // Archival Memory Operations
    // =========================================================================

    async fn save_archival_entry(
        &self,
        agent_id: &str,
        entry: &ArchivalEntry,
    ) -> Result<(), StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");
        assert!(!entry.id.is_empty(), "entry id cannot be empty");

        let key = Self::archival_key(agent_id, &entry.id);
        let value = Self::serialize(entry)?;

        self.kv
            .set(&self.actor_id, &key, &value)
            .await
            .map_err(|e| Self::map_kv_error("save_archival_entry", e))?;

        Ok(())
    }

    async fn load_archival_entries(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<ArchivalEntry>, StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");
        assert!(limit > 0, "limit must be positive");

        let prefix = Self::archival_prefix(agent_id);
        let pairs = self
            .kv
            .scan_prefix(&self.actor_id, &prefix)
            .await
            .map_err(|e| Self::map_kv_error("load_archival_entries", e))?;

        let mut entries = Vec::with_capacity(pairs.len().min(limit));
        for (_key, value) in pairs {
            if entries.len() >= limit {
                break;
            }
            let entry: ArchivalEntry = Self::deserialize(&value)?;
            entries.push(entry);
        }

        // Sort by creation time (most recent first)
        entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(entries)
    }

    async fn get_archival_entry(
        &self,
        agent_id: &str,
        entry_id: &str,
    ) -> Result<Option<ArchivalEntry>, StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");
        assert!(!entry_id.is_empty(), "entry id cannot be empty");

        let key = Self::archival_key(agent_id, entry_id);

        let bytes = self
            .kv
            .get(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("get_archival_entry", e))?;

        match bytes {
            Some(b) => {
                let entry = Self::deserialize(&b)?;
                Ok(Some(entry))
            }
            None => Ok(None),
        }
    }

    async fn delete_archival_entry(
        &self,
        agent_id: &str,
        entry_id: &str,
    ) -> Result<(), StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");
        assert!(!entry_id.is_empty(), "entry id cannot be empty");

        let key = Self::archival_key(agent_id, entry_id);

        self.kv
            .delete(&self.actor_id, &key)
            .await
            .map_err(|e| Self::map_kv_error("delete_archival_entry", e))?;

        Ok(())
    }

    async fn delete_archival_entries(&self, agent_id: &str) -> Result<(), StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");

        let prefix = Self::archival_prefix(agent_id);
        let keys = self
            .kv
            .list_keys(&self.actor_id, &prefix)
            .await
            .map_err(|e| Self::map_kv_error("delete_archival_entries", e))?;

        for key in keys {
            let _ = self.kv.delete(&self.actor_id, &key).await; // Continue on error
        }

        Ok(())
    }

    async fn search_archival_entries(
        &self,
        agent_id: &str,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ArchivalEntry>, StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");
        assert!(limit > 0, "limit must be positive");

        let prefix = Self::archival_prefix(agent_id);
        let pairs = self
            .kv
            .scan_prefix(&self.actor_id, &prefix)
            .await
            .map_err(|e| Self::map_kv_error("search_archival_entries", e))?;

        let mut entries = Vec::new();
        for (_key, value) in pairs {
            let entry: ArchivalEntry = Self::deserialize(&value)?;

            // Filter by query if provided (case-insensitive substring match)
            let matches = match query {
                Some(q) => entry.content.to_lowercase().contains(&q.to_lowercase()),
                None => true,
            };

            if matches {
                entries.push(entry);
                if entries.len() >= limit {
                    break;
                }
            }
        }

        // Sort by creation time (most recent first)
        entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AgentType, MessageRole};
    use kelpie_storage::memory::MemoryKV;

    fn test_adapter() -> KvAdapter {
        let kv: Arc<dyn ActorKV> = Arc::new(MemoryKV::new());
        KvAdapter::new(kv)
    }

    #[tokio::test]
    async fn test_adapter_agent_crud() {
        let adapter = test_adapter();

        // Create agent
        let agent = AgentMetadata::new(
            "agent-1".to_string(),
            "Test Agent".to_string(),
            AgentType::MemgptAgent,
        );
        adapter.save_agent(&agent).await.unwrap();

        // Load agent
        let loaded = adapter.load_agent("agent-1").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().name, "Test Agent");

        // List agents
        let agents = adapter.list_agents().await.unwrap();
        assert_eq!(agents.len(), 1);

        // Delete agent
        adapter.delete_agent("agent-1").await.unwrap();

        // Verify deleted
        let loaded = adapter.load_agent("agent-1").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_adapter_session_crud() {
        let adapter = test_adapter();

        // Create agent first
        let agent = AgentMetadata::new(
            "agent-1".to_string(),
            "Test Agent".to_string(),
            AgentType::MemgptAgent,
        );
        adapter.save_agent(&agent).await.unwrap();

        // Create session
        let session = SessionState::new("session-1".to_string(), "agent-1".to_string());
        adapter.save_session(&session).await.unwrap();

        // Load session
        let loaded = adapter.load_session("agent-1", "session-1").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().iteration, 0);

        // Update session
        let mut updated = session.clone();
        updated.advance_iteration();
        adapter.save_session(&updated).await.unwrap();

        // Verify update
        let loaded = adapter.load_session("agent-1", "session-1").await.unwrap();
        assert_eq!(loaded.unwrap().iteration, 1);

        // Delete session
        adapter
            .delete_session("agent-1", "session-1")
            .await
            .unwrap();

        // Verify deleted
        let loaded = adapter.load_session("agent-1", "session-1").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_adapter_messages() {
        let adapter = test_adapter();

        // Create agent first
        let agent = AgentMetadata::new(
            "agent-1".to_string(),
            "Test Agent".to_string(),
            AgentType::MemgptAgent,
        );
        adapter.save_agent(&agent).await.unwrap();

        // Add messages
        let msg1 = Message {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: "agent-1".to_string(),
            message_type: "user_message".to_string(),
            role: MessageRole::User,
            content: "Hello".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
            tool_call: None,
            tool_return: None,
            status: None,
            created_at: chrono::Utc::now(),
        };
        adapter.append_message("agent-1", &msg1).await.unwrap();

        let msg2 = Message {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: "agent-1".to_string(),
            message_type: "assistant_message".to_string(),
            role: MessageRole::Assistant,
            content: "Hi there!".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
            tool_call: None,
            tool_return: None,
            status: None,
            created_at: chrono::Utc::now(),
        };
        adapter.append_message("agent-1", &msg2).await.unwrap();

        // Load messages
        let messages = adapter.load_messages("agent-1", 10).await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "Hello");
        assert_eq!(messages[1].content, "Hi there!");

        // Count messages
        let count = adapter.count_messages("agent-1").await.unwrap();
        assert_eq!(count, 2);

        // Delete messages
        adapter.delete_messages("agent-1").await.unwrap();

        // Verify deleted
        let count = adapter.count_messages("agent-1").await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_adapter_blocks() {
        let adapter = test_adapter();

        // Create agent first
        let agent = AgentMetadata::new(
            "agent-1".to_string(),
            "Test Agent".to_string(),
            AgentType::MemgptAgent,
        );
        adapter.save_agent(&agent).await.unwrap();

        // Append to block (creates new)
        let block = adapter
            .append_block("agent-1", "persona", "I am helpful")
            .await
            .unwrap();
        assert_eq!(block.label, "persona");
        assert_eq!(block.value, "I am helpful");

        // Append more
        let block = adapter
            .append_block("agent-1", "persona", " and kind")
            .await
            .unwrap();
        assert_eq!(block.value, "I am helpful and kind");

        // Load blocks
        let blocks = adapter.load_blocks("agent-1").await.unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].value, "I am helpful and kind");

        // Update block
        let updated = adapter
            .update_block("agent-1", "persona", "I am very helpful")
            .await
            .unwrap();
        assert_eq!(updated.value, "I am very helpful");

        // Verify update
        let blocks = adapter.load_blocks("agent-1").await.unwrap();
        assert_eq!(blocks[0].value, "I am very helpful");
    }

    #[tokio::test]
    async fn test_adapter_custom_tools() {
        let adapter = test_adapter();

        // Create tool
        let now = chrono::Utc::now();
        let tool = CustomToolRecord {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            source_code: "def test(): pass".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            runtime: "python".to_string(),
            requirements: vec![],
            created_at: now,
            updated_at: now,
        };
        adapter.save_custom_tool(&tool).await.unwrap();

        // Load tool
        let loaded = adapter.load_custom_tool("test_tool").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().description, "A test tool");

        // List tools
        let tools = adapter.list_custom_tools().await.unwrap();
        assert_eq!(tools.len(), 1);

        // Delete tool
        adapter.delete_custom_tool("test_tool").await.unwrap();

        // Verify deleted
        let loaded = adapter.load_custom_tool("test_tool").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_adapter_checkpoint_atomic() {
        let adapter = test_adapter();

        // Create agent first
        let agent = AgentMetadata::new(
            "agent-1".to_string(),
            "Test Agent".to_string(),
            AgentType::MemgptAgent,
        );
        adapter.save_agent(&agent).await.unwrap();

        // Create session and message
        let session = SessionState::new("session-1".to_string(), "agent-1".to_string());
        let message = Message {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: "agent-1".to_string(),
            message_type: "user_message".to_string(),
            role: MessageRole::User,
            content: "Test message".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
            tool_call: None,
            tool_return: None,
            status: None,
            created_at: chrono::Utc::now(),
        };

        // Checkpoint atomically
        adapter.checkpoint(&session, Some(&message)).await.unwrap();

        // Verify both saved
        let loaded_session = adapter.load_session("agent-1", "session-1").await.unwrap();
        assert!(loaded_session.is_some());

        let messages = adapter.load_messages("agent-1", 10).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Test message");
    }

    #[tokio::test]
    async fn test_adapter_key_assertions() {
        // Test key length assertions
        let long_id = "a".repeat(300);
        let result = std::panic::catch_unwind(|| KvAdapter::agent_key(&long_id));
        assert!(result.is_err(), "should panic on key too long");

        // Test empty id assertions
        let result = std::panic::catch_unwind(|| KvAdapter::agent_key(""));
        assert!(result.is_err(), "should panic on empty agent id");
    }

    #[tokio::test]
    async fn test_adapter_mcp_server_crud() {
        let adapter = test_adapter();

        // Create MCP server
        use crate::models::{MCPServer, MCPServerConfig};
        let server = MCPServer::new(
            "test-server",
            MCPServerConfig::Stdio {
                command: "python".to_string(),
                args: vec!["-m".to_string(), "mcp_server".to_string()],
                env: None,
            },
        );
        adapter.save_mcp_server(&server).await.unwrap();

        // Load server
        let loaded = adapter.load_mcp_server(&server.id).await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().server_name, "test-server");

        // List servers
        let servers = adapter.list_mcp_servers().await.unwrap();
        assert_eq!(servers.len(), 1);

        // Delete server
        adapter.delete_mcp_server(&server.id).await.unwrap();

        // Verify deleted
        let loaded = adapter.load_mcp_server(&server.id).await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_adapter_agent_group_crud() {
        let adapter = test_adapter();

        // Create agent group
        use crate::models::{AgentGroup, CreateAgentGroupRequest, RoutingPolicy};
        let request = CreateAgentGroupRequest {
            name: Some("test-group".to_string()),
            description: Some("Test group".to_string()),
            agent_ids: vec!["agent-1".to_string(), "agent-2".to_string()],
            routing_policy: RoutingPolicy::RoundRobin,
            metadata: serde_json::json!({}),
        };
        let group = AgentGroup::from_request(request);
        adapter.save_agent_group(&group).await.unwrap();

        // Load group
        let loaded = adapter.load_agent_group(&group.id).await.unwrap();
        assert!(loaded.is_some());
        let loaded_group = loaded.unwrap();
        assert_eq!(loaded_group.name, "test-group");
        assert_eq!(loaded_group.agent_ids.len(), 2);

        // List groups
        let groups = adapter.list_agent_groups().await.unwrap();
        assert_eq!(groups.len(), 1);

        // Delete group
        adapter.delete_agent_group(&group.id).await.unwrap();

        // Verify deleted
        let loaded = adapter.load_agent_group(&group.id).await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_adapter_identity_crud() {
        let adapter = test_adapter();

        // Create identity
        use crate::models::{CreateIdentityRequest, Identity, IdentityType};
        let request = CreateIdentityRequest {
            name: "Test User".to_string(),
            identifier_key: Some("user-123".to_string()),
            identity_type: IdentityType::User,
            agent_ids: vec!["agent-1".to_string()],
            block_ids: vec![],
            project_id: None,
            properties: serde_json::json!({"email": "test@example.com"}),
        };
        let identity = Identity::from_request(request);
        adapter.save_identity(&identity).await.unwrap();

        // Load identity
        let loaded = adapter.load_identity(&identity.id).await.unwrap();
        assert!(loaded.is_some());
        let loaded_identity = loaded.unwrap();
        assert_eq!(loaded_identity.name, "Test User");
        assert_eq!(loaded_identity.identifier_key, "user-123");

        // List identities
        let identities = adapter.list_identities().await.unwrap();
        assert_eq!(identities.len(), 1);

        // Delete identity
        adapter.delete_identity(&identity.id).await.unwrap();

        // Verify deleted
        let loaded = adapter.load_identity(&identity.id).await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_adapter_project_crud() {
        let adapter = test_adapter();

        // Create project
        use crate::models::{CreateProjectRequest, Project};
        let request = CreateProjectRequest {
            name: "Test Project".to_string(),
            description: Some("A test project".to_string()),
            tags: vec!["test".to_string()],
            metadata: serde_json::json!({}),
        };
        let project = Project::from_request(request);
        adapter.save_project(&project).await.unwrap();

        // Load project
        let loaded = adapter.load_project(&project.id).await.unwrap();
        assert!(loaded.is_some());
        let loaded_project = loaded.unwrap();
        assert_eq!(loaded_project.name, "Test Project");
        assert_eq!(loaded_project.tags.len(), 1);

        // List projects
        let projects = adapter.list_projects().await.unwrap();
        assert_eq!(projects.len(), 1);

        // Delete project
        adapter.delete_project(&project.id).await.unwrap();

        // Verify deleted
        let loaded = adapter.load_project(&project.id).await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_adapter_job_crud() {
        let adapter = test_adapter();

        // Create job
        use crate::models::{CreateJobRequest, Job, JobAction, ScheduleType};
        let request = CreateJobRequest {
            agent_id: "agent-1".to_string(),
            schedule_type: ScheduleType::Interval,
            schedule: "3600".to_string(), // Every hour
            action: JobAction::SendMessage,
            action_params: serde_json::json!({"message": "Hello"}),
            description: Some("Test job".to_string()),
        };
        let job = Job::from_request(request);
        adapter.save_job(&job).await.unwrap();

        // Load job
        let loaded = adapter.load_job(&job.id).await.unwrap();
        assert!(loaded.is_some());
        let loaded_job = loaded.unwrap();
        assert_eq!(loaded_job.agent_id, "agent-1");
        assert_eq!(loaded_job.schedule, "3600");

        // List jobs
        let jobs = adapter.list_jobs().await.unwrap();
        assert_eq!(jobs.len(), 1);

        // Delete job
        adapter.delete_job(&job.id).await.unwrap();

        // Verify deleted
        let loaded = adapter.load_job(&job.id).await.unwrap();
        assert!(loaded.is_none());
    }
}
