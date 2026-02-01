//! FoundationDB Agent Registry
//!
//! TigerStyle: AgentStorage implementation using FdbKV for unified storage.
//!
//! Architecture:
//! - Global agent registry stored as special actor ("system/agent_registry")
//! - Per-agent data stored in actor's own KV space
//! - Single FDB connection shared across all storage operations
//!
//! Key Space (via FdbKV/ActorKV):
//! ```text
//! Registry: ("kelpie", "actors", "system", "agent_registry", "data", "agent-123")
//! Per-agent: ("kelpie", "actors", "agents", "agent-123", "data", "blocks")
//! Per-agent: ("kelpie", "actors", "agents", "agent-123", "data", "session:xyz")
//! Per-agent: ("kelpie", "actors", "agents", "agent-123", "data", "message:0")
//! ```

use async_trait::async_trait;
use bytes::Bytes;
use kelpie_core::{ActorId, Result as CoreResult};
use kelpie_storage::{ActorKV, FdbKV};
use std::sync::Arc;

use crate::models::{ArchivalEntry, Block, Message};

use super::traits::{AgentStorage, StorageError};
use super::types::{AgentMetadata, CustomToolRecord, SessionState};

// =============================================================================
// Constants (TigerStyle)
// =============================================================================

/// Registry actor ID for global agent metadata
const REGISTRY_NAMESPACE: &str = "system";
const REGISTRY_ID: &str = "agent_registry";
const TOOL_REGISTRY_ID: &str = "tool_registry";
const MCP_REGISTRY_ID: &str = "mcp_registry";
const GROUP_REGISTRY_ID: &str = "group_registry";
const IDENTITY_REGISTRY_ID: &str = "identity_registry";
const PROJECT_REGISTRY_ID: &str = "project_registry";
const JOB_REGISTRY_ID: &str = "job_registry";

/// Key prefixes for per-agent data
const KEY_PREFIX_BLOCKS: &[u8] = b"blocks";
const KEY_PREFIX_SESSION: &[u8] = b"session:";
const KEY_PREFIX_MESSAGE: &[u8] = b"message:";
const KEY_PREFIX_MESSAGE_COUNT: &[u8] = b"message_count";
const KEY_PREFIX_TOOL: &[u8] = b"tool:";
const KEY_PREFIX_MCP: &[u8] = b"mcp:";
const KEY_PREFIX_GROUP: &[u8] = b"group:";
const KEY_PREFIX_IDENTITY: &[u8] = b"identity:";
const KEY_PREFIX_PROJECT: &[u8] = b"project:";
const KEY_PREFIX_JOB: &[u8] = b"job:";
const KEY_PREFIX_ARCHIVAL: &[u8] = b"archival:";

// =============================================================================
// FdbAgentRegistry Implementation
// =============================================================================

/// FDB-backed agent registry using FdbKV
///
/// Uses FdbKV under the hood:
/// - Registry: Special actor ("system/agent_registry") stores agent metadata
/// - Per-agent: Regular actors ("agents/{id}") store blocks/sessions/messages
pub struct FdbAgentRegistry {
    /// Shared FDB KV instance
    fdb: Arc<FdbKV>,
}

impl FdbAgentRegistry {
    /// Create new FDB agent registry
    ///
    /// # Arguments
    /// * `fdb` - Shared FdbKV instance
    pub fn new(fdb: Arc<FdbKV>) -> Self {
        Self { fdb }
    }

    /// Get registry actor ID (for storing agent metadata)
    fn registry_actor_id() -> CoreResult<ActorId> {
        ActorId::new(REGISTRY_NAMESPACE, REGISTRY_ID)
    }

    /// Get registry actor ID for tools
    fn tool_registry_actor_id() -> CoreResult<ActorId> {
        ActorId::new(REGISTRY_NAMESPACE, TOOL_REGISTRY_ID)
    }

    /// Get registry actor ID for MCP servers
    fn mcp_registry_actor_id() -> CoreResult<ActorId> {
        ActorId::new(REGISTRY_NAMESPACE, MCP_REGISTRY_ID)
    }

    /// Get registry actor ID for agent groups
    fn group_registry_actor_id() -> CoreResult<ActorId> {
        ActorId::new(REGISTRY_NAMESPACE, GROUP_REGISTRY_ID)
    }

    /// Get registry actor ID for identities
    fn identity_registry_actor_id() -> CoreResult<ActorId> {
        ActorId::new(REGISTRY_NAMESPACE, IDENTITY_REGISTRY_ID)
    }

    /// Get registry actor ID for projects
    fn project_registry_actor_id() -> CoreResult<ActorId> {
        ActorId::new(REGISTRY_NAMESPACE, PROJECT_REGISTRY_ID)
    }

    /// Get registry actor ID for jobs
    fn job_registry_actor_id() -> CoreResult<ActorId> {
        ActorId::new(REGISTRY_NAMESPACE, JOB_REGISTRY_ID)
    }

    /// Get actor ID for an agent
    fn agent_actor_id(agent_id: &str) -> CoreResult<ActorId> {
        ActorId::new("agents", agent_id)
    }

    /// Serialize metadata to bytes
    fn serialize_metadata(agent: &AgentMetadata) -> Result<Bytes, StorageError> {
        serde_json::to_vec(agent)
            .map(Bytes::from)
            .map_err(|e| StorageError::SerializationFailed {
                reason: e.to_string(),
            })
    }

    /// Deserialize metadata from bytes
    fn deserialize_metadata(bytes: &Bytes) -> Result<AgentMetadata, StorageError> {
        serde_json::from_slice(bytes).map_err(|e| StorageError::DeserializationFailed {
            reason: e.to_string(),
        })
    }

    /// Serialize blocks to bytes
    fn serialize_blocks(blocks: &[Block]) -> Result<Bytes, StorageError> {
        serde_json::to_vec(blocks)
            .map(Bytes::from)
            .map_err(|e| StorageError::SerializationFailed {
                reason: e.to_string(),
            })
    }

    /// Deserialize blocks from bytes
    fn deserialize_blocks(bytes: &Bytes) -> Result<Vec<Block>, StorageError> {
        serde_json::from_slice(bytes).map_err(|e| StorageError::DeserializationFailed {
            reason: e.to_string(),
        })
    }

    /// Serialize session to bytes
    fn serialize_session(session: &SessionState) -> Result<Bytes, StorageError> {
        serde_json::to_vec(session).map(Bytes::from).map_err(|e| {
            StorageError::SerializationFailed {
                reason: e.to_string(),
            }
        })
    }

    /// Deserialize session from bytes
    fn deserialize_session(bytes: &Bytes) -> Result<SessionState, StorageError> {
        serde_json::from_slice(bytes).map_err(|e| StorageError::DeserializationFailed {
            reason: e.to_string(),
        })
    }

    /// Serialize message to bytes
    fn serialize_message(message: &Message) -> Result<Bytes, StorageError> {
        serde_json::to_vec(message).map(Bytes::from).map_err(|e| {
            StorageError::SerializationFailed {
                reason: e.to_string(),
            }
        })
    }

    /// Deserialize message from bytes
    fn deserialize_message(bytes: &Bytes) -> Result<Message, StorageError> {
        serde_json::from_slice(bytes).map_err(|e| StorageError::DeserializationFailed {
            reason: e.to_string(),
        })
    }

    /// Serialize custom tool to bytes
    fn serialize_custom_tool(tool: &CustomToolRecord) -> Result<Bytes, StorageError> {
        serde_json::to_vec(tool)
            .map(Bytes::from)
            .map_err(|e| StorageError::SerializationFailed {
                reason: e.to_string(),
            })
    }

    /// Deserialize custom tool from bytes
    fn deserialize_custom_tool(bytes: &Bytes) -> Result<CustomToolRecord, StorageError> {
        serde_json::from_slice(bytes).map_err(|e| StorageError::DeserializationFailed {
            reason: e.to_string(),
        })
    }

    /// Serialize MCP server to bytes
    fn serialize_mcp_server(server: &crate::models::MCPServer) -> Result<Bytes, StorageError> {
        serde_json::to_vec(server)
            .map(Bytes::from)
            .map_err(|e| StorageError::SerializationFailed {
                reason: e.to_string(),
            })
    }

    /// Deserialize MCP server from bytes
    fn deserialize_mcp_server(bytes: &Bytes) -> Result<crate::models::MCPServer, StorageError> {
        serde_json::from_slice(bytes).map_err(|e| StorageError::DeserializationFailed {
            reason: e.to_string(),
        })
    }

    /// Serialize agent group to bytes
    fn serialize_agent_group(group: &crate::models::AgentGroup) -> Result<Bytes, StorageError> {
        serde_json::to_vec(group)
            .map(Bytes::from)
            .map_err(|e| StorageError::SerializationFailed {
                reason: e.to_string(),
            })
    }

    /// Deserialize agent group from bytes
    fn deserialize_agent_group(bytes: &Bytes) -> Result<crate::models::AgentGroup, StorageError> {
        serde_json::from_slice(bytes).map_err(|e| StorageError::DeserializationFailed {
            reason: e.to_string(),
        })
    }

    /// Serialize identity to bytes
    fn serialize_identity(identity: &crate::models::Identity) -> Result<Bytes, StorageError> {
        serde_json::to_vec(identity).map(Bytes::from).map_err(|e| {
            StorageError::SerializationFailed {
                reason: e.to_string(),
            }
        })
    }

    /// Deserialize identity from bytes
    fn deserialize_identity(bytes: &Bytes) -> Result<crate::models::Identity, StorageError> {
        serde_json::from_slice(bytes).map_err(|e| StorageError::DeserializationFailed {
            reason: e.to_string(),
        })
    }

    /// Serialize project to bytes
    fn serialize_project(project: &crate::models::Project) -> Result<Bytes, StorageError> {
        serde_json::to_vec(project).map(Bytes::from).map_err(|e| {
            StorageError::SerializationFailed {
                reason: e.to_string(),
            }
        })
    }

    /// Deserialize project from bytes
    fn deserialize_project(bytes: &Bytes) -> Result<crate::models::Project, StorageError> {
        serde_json::from_slice(bytes).map_err(|e| StorageError::DeserializationFailed {
            reason: e.to_string(),
        })
    }

    /// Serialize job to bytes
    fn serialize_job(job: &crate::models::Job) -> Result<Bytes, StorageError> {
        serde_json::to_vec(job)
            .map(Bytes::from)
            .map_err(|e| StorageError::SerializationFailed {
                reason: e.to_string(),
            })
    }

    /// Deserialize job from bytes
    fn deserialize_job(bytes: &Bytes) -> Result<crate::models::Job, StorageError> {
        serde_json::from_slice(bytes).map_err(|e| StorageError::DeserializationFailed {
            reason: e.to_string(),
        })
    }

    /// Serialize archival entry to bytes
    fn serialize_archival_entry(entry: &ArchivalEntry) -> Result<Bytes, StorageError> {
        serde_json::to_vec(entry)
            .map(Bytes::from)
            .map_err(|e| StorageError::SerializationFailed {
                reason: e.to_string(),
            })
    }

    /// Deserialize archival entry from bytes
    fn deserialize_archival_entry(bytes: &Bytes) -> Result<ArchivalEntry, StorageError> {
        serde_json::from_slice(bytes).map_err(|e| StorageError::DeserializationFailed {
            reason: e.to_string(),
        })
    }

    /// Convert kelpie_core::Error to StorageError
    fn map_core_error(err: kelpie_core::Error) -> StorageError {
        StorageError::Internal {
            message: err.to_string(),
        }
    }
}

#[async_trait]
impl AgentStorage for FdbAgentRegistry {
    // =========================================================================
    // Agent Metadata Operations (Global Registry)
    // =========================================================================

    async fn save_agent(&self, agent: &AgentMetadata) -> Result<(), StorageError> {
        // Preconditions
        assert!(!agent.id.is_empty(), "agent id cannot be empty");
        assert!(!agent.name.is_empty(), "agent name cannot be empty");

        let registry_id = Self::registry_actor_id().map_err(Self::map_core_error)?;
        let key = agent.id.as_bytes();
        let value = Self::serialize_metadata(agent)?;

        self.fdb
            .set(&registry_id, key, &value)
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    async fn load_agent(&self, id: &str) -> Result<Option<AgentMetadata>, StorageError> {
        // Preconditions
        assert!(!id.is_empty(), "agent id cannot be empty");

        let registry_id = Self::registry_actor_id().map_err(Self::map_core_error)?;
        let key = id.as_bytes();

        match self.fdb.get(&registry_id, key).await {
            Ok(Some(bytes)) => {
                let metadata = Self::deserialize_metadata(&bytes)?;
                Ok(Some(metadata))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(Self::map_core_error(e)),
        }
    }

    async fn delete_agent(&self, id: &str) -> Result<(), StorageError> {
        // Preconditions
        assert!(!id.is_empty(), "agent id cannot be empty");

        // First, gather all keys that need to be deleted (before starting transaction)
        // This is necessary because scan_prefix can't be done inside a transaction
        let agent_id = Self::agent_actor_id(id).map_err(Self::map_core_error)?;

        // Scan for all session keys
        let session_kvs = self
            .fdb
            .scan_prefix(&agent_id, KEY_PREFIX_SESSION)
            .await
            .map_err(Self::map_core_error)?;

        // Scan for all message keys
        let message_kvs = self
            .fdb
            .scan_prefix(&agent_id, KEY_PREFIX_MESSAGE)
            .await
            .map_err(Self::map_core_error)?;

        // Scan for all archival keys
        let archival_kvs = self
            .fdb
            .scan_prefix(&agent_id, KEY_PREFIX_ARCHIVAL)
            .await
            .map_err(Self::map_core_error)?;

        // Now perform ALL deletes in a single transaction for atomicity
        // Note: FDB transactions have a 10MB limit, but agent data should fit
        let mut txn = self
            .fdb
            .begin_transaction(&agent_id)
            .await
            .map_err(Self::map_core_error)?;

        // Delete blocks
        txn.delete(KEY_PREFIX_BLOCKS)
            .await
            .map_err(Self::map_core_error)?;

        // Delete message count
        txn.delete(KEY_PREFIX_MESSAGE_COUNT)
            .await
            .map_err(Self::map_core_error)?;

        // Delete all sessions
        for (session_key, _) in session_kvs {
            txn.delete(&session_key)
                .await
                .map_err(Self::map_core_error)?;
        }

        // Delete all messages
        for (message_key, _) in message_kvs {
            txn.delete(&message_key)
                .await
                .map_err(Self::map_core_error)?;
        }

        // Delete all archival entries
        for (archival_key, _) in archival_kvs {
            txn.delete(&archival_key)
                .await
                .map_err(Self::map_core_error)?;
        }

        // Commit per-agent data deletion
        txn.commit().await.map_err(Self::map_core_error)?;

        // Delete from registry (separate transaction since different actor)
        let registry_id = Self::registry_actor_id().map_err(Self::map_core_error)?;
        let key = id.as_bytes();

        self.fdb
            .delete(&registry_id, key)
            .await
            .map_err(Self::map_core_error)?;

        tracing::info!(agent_id = %id, "Deleted agent with atomic cascading deletes");

        Ok(())
    }

    async fn list_agents(&self) -> Result<Vec<AgentMetadata>, StorageError> {
        let registry_id = Self::registry_actor_id().map_err(Self::map_core_error)?;

        tracing::debug!("list_agents: scanning registry");

        // Scan all keys in registry (empty prefix = all keys)
        let kvs = self
            .fdb
            .scan_prefix(&registry_id, b"")
            .await
            .map_err(Self::map_core_error)?;

        tracing::debug!(kv_count = kvs.len(), "list_agents: found kvs in registry");

        let mut agents = Vec::new();
        for (key, value) in kvs {
            let metadata = Self::deserialize_metadata(&value)?;
            tracing::debug!(agent_id = %metadata.id, agent_name = %metadata.name, key_len = key.len(), "list_agents: deserialized agent");
            agents.push(metadata);
        }

        // Sort by ID for deterministic ordering
        agents.sort_by(|a, b| a.id.cmp(&b.id));

        tracing::info!(agent_count = agents.len(), "list_agents complete");

        Ok(agents)
    }

    // =========================================================================
    // Core Memory Block Operations (Per-Agent)
    // =========================================================================

    async fn save_blocks(&self, agent_id: &str, blocks: &[Block]) -> Result<(), StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;
        let value = Self::serialize_blocks(blocks)?;

        self.fdb
            .set(&actor_id, KEY_PREFIX_BLOCKS, &value)
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    async fn load_blocks(&self, agent_id: &str) -> Result<Vec<Block>, StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;

        match self.fdb.get(&actor_id, KEY_PREFIX_BLOCKS).await {
            Ok(Some(bytes)) => Self::deserialize_blocks(&bytes),
            Ok(None) => Ok(Vec::new()),
            Err(e) => Err(Self::map_core_error(e)),
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

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;

        // Use a transaction for atomic read-modify-write
        // This prevents race conditions when concurrent updates occur
        let mut txn = self
            .fdb
            .begin_transaction(&actor_id)
            .await
            .map_err(Self::map_core_error)?;

        // Load existing blocks within transaction
        let mut blocks = match txn.get(KEY_PREFIX_BLOCKS).await {
            Ok(Some(bytes)) => Self::deserialize_blocks(&bytes)?,
            Ok(None) => Vec::new(),
            Err(e) => return Err(Self::map_core_error(e)),
        };

        // Find and update block
        let mut found = false;
        let mut result_block = None;
        for block in &mut blocks {
            if block.label == label {
                block.value = value.to_string();
                block.updated_at = chrono::Utc::now();
                result_block = Some(block.clone());
                found = true;
                break;
            }
        }

        if !found {
            return Err(StorageError::NotFound {
                resource: "block",
                id: label.to_string(),
            });
        }

        // Save updated blocks within transaction
        let blocks_value = Self::serialize_blocks(&blocks)?;
        txn.set(KEY_PREFIX_BLOCKS, &blocks_value)
            .await
            .map_err(Self::map_core_error)?;

        // Commit transaction
        txn.commit().await.map_err(Self::map_core_error)?;

        Ok(result_block.unwrap())
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

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;

        // Use a transaction for atomic read-modify-write
        // This prevents race conditions when concurrent appends occur
        let mut txn = self
            .fdb
            .begin_transaction(&actor_id)
            .await
            .map_err(Self::map_core_error)?;

        // Load existing blocks within transaction
        let mut blocks = match txn.get(KEY_PREFIX_BLOCKS).await {
            Ok(Some(bytes)) => Self::deserialize_blocks(&bytes)?,
            Ok(None) => Vec::new(),
            Err(e) => return Err(Self::map_core_error(e)),
        };

        // Find existing block or create new
        let mut found = false;
        let mut result_block = None;
        for block in &mut blocks {
            if block.label == label {
                block.value.push_str(content);
                block.updated_at = chrono::Utc::now();
                result_block = Some(block.clone());
                found = true;
                break;
            }
        }

        if !found {
            // Create new block
            let block = Block::new(label, content);
            result_block = Some(block.clone());
            blocks.push(block);
        }

        // Save blocks within transaction
        let blocks_value = Self::serialize_blocks(&blocks)?;
        txn.set(KEY_PREFIX_BLOCKS, &blocks_value)
            .await
            .map_err(Self::map_core_error)?;

        // Commit transaction
        txn.commit().await.map_err(Self::map_core_error)?;

        Ok(result_block.unwrap())
    }

    // =========================================================================
    // Session State Operations (Per-Agent)
    // =========================================================================

    async fn save_session(&self, state: &SessionState) -> Result<(), StorageError> {
        // Preconditions
        assert!(!state.agent_id.is_empty(), "agent id cannot be empty");
        assert!(!state.session_id.is_empty(), "session id cannot be empty");

        let actor_id = Self::agent_actor_id(&state.agent_id).map_err(Self::map_core_error)?;
        let key = format!(
            "{}{}",
            String::from_utf8_lossy(KEY_PREFIX_SESSION),
            state.session_id
        );
        let value = Self::serialize_session(state)?;

        self.fdb
            .set(&actor_id, key.as_bytes(), &value)
            .await
            .map_err(Self::map_core_error)?;

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

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;
        let key = format!(
            "{}{}",
            String::from_utf8_lossy(KEY_PREFIX_SESSION),
            session_id
        );

        match self.fdb.get(&actor_id, key.as_bytes()).await {
            Ok(Some(bytes)) => {
                let session = Self::deserialize_session(&bytes)?;
                Ok(Some(session))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(Self::map_core_error(e)),
        }
    }

    async fn delete_session(&self, agent_id: &str, session_id: &str) -> Result<(), StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");
        assert!(!session_id.is_empty(), "session id cannot be empty");

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;
        let key = format!(
            "{}{}",
            String::from_utf8_lossy(KEY_PREFIX_SESSION),
            session_id
        );

        self.fdb
            .delete(&actor_id, key.as_bytes())
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    async fn list_sessions(&self, agent_id: &str) -> Result<Vec<SessionState>, StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;

        // Scan all keys with session prefix
        let kvs = self
            .fdb
            .scan_prefix(&actor_id, KEY_PREFIX_SESSION)
            .await
            .map_err(Self::map_core_error)?;

        let mut sessions = Vec::new();
        for (_key, value) in kvs {
            let session = Self::deserialize_session(&value)?;
            sessions.push(session);
        }

        Ok(sessions)
    }

    // =========================================================================
    // Message Operations (Per-Agent)
    // =========================================================================

    async fn append_message(&self, agent_id: &str, message: &Message) -> Result<(), StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;

        // Use a transaction for atomic read-modify-write
        // This prevents race conditions when concurrent appends occur
        let mut txn = self
            .fdb
            .begin_transaction(&actor_id)
            .await
            .map_err(Self::map_core_error)?;

        // Get current message count within transaction
        let count = match txn.get(KEY_PREFIX_MESSAGE_COUNT).await {
            Ok(Some(bytes)) => {
                let count_str = String::from_utf8(bytes.to_vec()).map_err(|e| {
                    StorageError::DeserializationFailed {
                        reason: e.to_string(),
                    }
                })?;
                count_str
                    .parse::<u64>()
                    .map_err(|e| StorageError::DeserializationFailed {
                        reason: e.to_string(),
                    })?
            }
            Ok(None) => 0,
            Err(e) => return Err(Self::map_core_error(e)),
        };

        // Serialize message
        let message_key = format!("{}{}", String::from_utf8_lossy(KEY_PREFIX_MESSAGE), count);
        let message_value = Self::serialize_message(message)?;

        // Store message at index (within transaction)
        txn.set(message_key.as_bytes(), &message_value)
            .await
            .map_err(Self::map_core_error)?;

        // Increment count (within transaction)
        let new_count = count + 1;
        txn.set(KEY_PREFIX_MESSAGE_COUNT, new_count.to_string().as_bytes())
            .await
            .map_err(Self::map_core_error)?;

        // Commit transaction - all operations are atomic
        txn.commit().await.map_err(Self::map_core_error)?;

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

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;

        // Get message count
        let count = match self.fdb.get(&actor_id, KEY_PREFIX_MESSAGE_COUNT).await {
            Ok(Some(bytes)) => {
                let count_str = String::from_utf8(bytes.to_vec()).map_err(|e| {
                    StorageError::DeserializationFailed {
                        reason: e.to_string(),
                    }
                })?;
                count_str
                    .parse::<u64>()
                    .map_err(|e| StorageError::DeserializationFailed {
                        reason: e.to_string(),
                    })?
            }
            Ok(None) => return Ok(Vec::new()),
            Err(e) => return Err(Self::map_core_error(e)),
        };

        // Calculate range (most recent messages)
        let start_idx = count.saturating_sub(limit as u64);

        let mut messages = Vec::new();
        for i in start_idx..count {
            let message_key = format!("{}{}", String::from_utf8_lossy(KEY_PREFIX_MESSAGE), i);
            match self.fdb.get(&actor_id, message_key.as_bytes()).await {
                Ok(Some(bytes)) => {
                    let message = Self::deserialize_message(&bytes)?;
                    messages.push(message);
                }
                Ok(None) => {
                    // Message not found - skip
                }
                Err(e) => return Err(Self::map_core_error(e)),
            }
        }

        Ok(messages)
    }

    async fn load_messages_since(
        &self,
        agent_id: &str,
        since_ms: u64,
    ) -> Result<Vec<Message>, StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");

        // Load all messages and filter by timestamp
        // TODO: Optimize with secondary index
        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;

        let kvs = self
            .fdb
            .scan_prefix(&actor_id, KEY_PREFIX_MESSAGE)
            .await
            .map_err(Self::map_core_error)?;

        let mut messages = Vec::new();
        for (_key, value) in kvs {
            let message = Self::deserialize_message(&value)?;
            if message.created_at.timestamp_millis() as u64 > since_ms {
                messages.push(message);
            }
        }

        Ok(messages)
    }

    async fn count_messages(&self, agent_id: &str) -> Result<usize, StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;

        match self.fdb.get(&actor_id, KEY_PREFIX_MESSAGE_COUNT).await {
            Ok(Some(bytes)) => {
                let count_str = String::from_utf8(bytes.to_vec()).map_err(|e| {
                    StorageError::DeserializationFailed {
                        reason: e.to_string(),
                    }
                })?;
                let count = count_str.parse::<usize>().map_err(|e| {
                    StorageError::DeserializationFailed {
                        reason: e.to_string(),
                    }
                })?;
                Ok(count)
            }
            Ok(None) => Ok(0),
            Err(e) => Err(Self::map_core_error(e)),
        }
    }

    async fn delete_messages(&self, agent_id: &str) -> Result<(), StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;

        // Get message count
        let count = self.count_messages(agent_id).await?;

        // Delete all message keys
        for i in 0..count {
            let message_key = format!("{}{}", String::from_utf8_lossy(KEY_PREFIX_MESSAGE), i);
            self.fdb
                .delete(&actor_id, message_key.as_bytes())
                .await
                .map_err(Self::map_core_error)?;
        }

        // Reset count
        self.fdb
            .set(&actor_id, KEY_PREFIX_MESSAGE_COUNT, &Bytes::from("0"))
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    // =========================================================================
    // Custom Tool Operations
    // =========================================================================

    async fn save_custom_tool(&self, tool: &CustomToolRecord) -> Result<(), StorageError> {
        assert!(!tool.name.is_empty(), "tool name cannot be empty");

        let registry_id = Self::tool_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_TOOL, tool.name.as_bytes()].concat();
        let value = Self::serialize_custom_tool(tool)?;

        self.fdb
            .set(&registry_id, &key, &value)
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    async fn load_custom_tool(&self, name: &str) -> Result<Option<CustomToolRecord>, StorageError> {
        assert!(!name.is_empty(), "tool name cannot be empty");

        let registry_id = Self::tool_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_TOOL, name.as_bytes()].concat();

        match self.fdb.get(&registry_id, &key).await {
            Ok(Some(bytes)) => {
                let tool = Self::deserialize_custom_tool(&bytes)?;
                Ok(Some(tool))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(Self::map_core_error(e)),
        }
    }

    async fn delete_custom_tool(&self, name: &str) -> Result<(), StorageError> {
        assert!(!name.is_empty(), "tool name cannot be empty");

        let registry_id = Self::tool_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_TOOL, name.as_bytes()].concat();

        self.fdb
            .delete(&registry_id, &key)
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    async fn list_custom_tools(&self) -> Result<Vec<CustomToolRecord>, StorageError> {
        let registry_id = Self::tool_registry_actor_id().map_err(Self::map_core_error)?;
        let keys = self
            .fdb
            .list_keys(&registry_id, KEY_PREFIX_TOOL)
            .await
            .map_err(Self::map_core_error)?;

        let mut tools = Vec::new();
        for key in keys {
            if let Ok(Some(bytes)) = self.fdb.get(&registry_id, &key).await {
                if let Ok(tool) = Self::deserialize_custom_tool(&bytes) {
                    tools.push(tool);
                }
            }
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
        assert!(!session.agent_id.is_empty(), "agent id cannot be empty");
        assert!(!session.session_id.is_empty(), "session id cannot be empty");

        let actor_id = Self::agent_actor_id(&session.agent_id).map_err(Self::map_core_error)?;

        // Use a transaction for atomic session + message checkpoint
        // Both operations succeed or fail together
        let mut txn = self
            .fdb
            .begin_transaction(&actor_id)
            .await
            .map_err(Self::map_core_error)?;

        // Serialize and store session
        let session_key = format!(
            "{}{}",
            String::from_utf8_lossy(KEY_PREFIX_SESSION),
            session.session_id
        );
        let session_value = Self::serialize_session(session)?;
        txn.set(session_key.as_bytes(), &session_value)
            .await
            .map_err(Self::map_core_error)?;

        // Append message if provided (within same transaction)
        if let Some(msg) = message {
            // Get current message count
            let count = match txn.get(KEY_PREFIX_MESSAGE_COUNT).await {
                Ok(Some(bytes)) => {
                    let count_str = String::from_utf8(bytes.to_vec()).map_err(|e| {
                        StorageError::DeserializationFailed {
                            reason: e.to_string(),
                        }
                    })?;
                    count_str
                        .parse::<u64>()
                        .map_err(|e| StorageError::DeserializationFailed {
                            reason: e.to_string(),
                        })?
                }
                Ok(None) => 0,
                Err(e) => return Err(Self::map_core_error(e)),
            };

            // Store message
            let message_key = format!("{}{}", String::from_utf8_lossy(KEY_PREFIX_MESSAGE), count);
            let message_value = Self::serialize_message(msg)?;
            txn.set(message_key.as_bytes(), &message_value)
                .await
                .map_err(Self::map_core_error)?;

            // Increment count
            let new_count = count + 1;
            txn.set(KEY_PREFIX_MESSAGE_COUNT, new_count.to_string().as_bytes())
                .await
                .map_err(Self::map_core_error)?;
        }

        // Commit transaction - session and message are atomic
        txn.commit().await.map_err(Self::map_core_error)?;

        Ok(())
    }

    // =========================================================================
    // MCP Server Operations
    // =========================================================================

    async fn save_mcp_server(&self, server: &crate::models::MCPServer) -> Result<(), StorageError> {
        assert!(!server.id.is_empty(), "server id cannot be empty");

        let registry_id = Self::mcp_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_MCP, server.id.as_bytes()].concat();
        let value = Self::serialize_mcp_server(server)?;

        self.fdb
            .set(&registry_id, &key, &value)
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    async fn load_mcp_server(
        &self,
        id: &str,
    ) -> Result<Option<crate::models::MCPServer>, StorageError> {
        assert!(!id.is_empty(), "server id cannot be empty");

        let registry_id = Self::mcp_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_MCP, id.as_bytes()].concat();

        match self.fdb.get(&registry_id, &key).await {
            Ok(Some(bytes)) => {
                let server = Self::deserialize_mcp_server(&bytes)?;
                Ok(Some(server))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(Self::map_core_error(e)),
        }
    }

    async fn delete_mcp_server(&self, id: &str) -> Result<(), StorageError> {
        assert!(!id.is_empty(), "server id cannot be empty");

        let registry_id = Self::mcp_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_MCP, id.as_bytes()].concat();

        self.fdb
            .delete(&registry_id, &key)
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    async fn list_mcp_servers(&self) -> Result<Vec<crate::models::MCPServer>, StorageError> {
        let registry_id = Self::mcp_registry_actor_id().map_err(Self::map_core_error)?;
        let keys = self
            .fdb
            .list_keys(&registry_id, KEY_PREFIX_MCP)
            .await
            .map_err(Self::map_core_error)?;

        let mut servers = Vec::new();
        for key in keys {
            if let Ok(Some(bytes)) = self.fdb.get(&registry_id, &key).await {
                if let Ok(server) = Self::deserialize_mcp_server(&bytes) {
                    servers.push(server);
                }
            }
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
        assert!(!group.id.is_empty(), "group id cannot be empty");

        let registry_id = Self::group_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_GROUP, group.id.as_bytes()].concat();
        let value = Self::serialize_agent_group(group)?;

        self.fdb
            .set(&registry_id, &key, &value)
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    async fn load_agent_group(
        &self,
        id: &str,
    ) -> Result<Option<crate::models::AgentGroup>, StorageError> {
        assert!(!id.is_empty(), "group id cannot be empty");

        let registry_id = Self::group_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_GROUP, id.as_bytes()].concat();

        match self.fdb.get(&registry_id, &key).await {
            Ok(Some(bytes)) => {
                let group = Self::deserialize_agent_group(&bytes)?;
                Ok(Some(group))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(Self::map_core_error(e)),
        }
    }

    async fn delete_agent_group(&self, id: &str) -> Result<(), StorageError> {
        assert!(!id.is_empty(), "group id cannot be empty");

        let registry_id = Self::group_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_GROUP, id.as_bytes()].concat();

        self.fdb
            .delete(&registry_id, &key)
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    async fn list_agent_groups(&self) -> Result<Vec<crate::models::AgentGroup>, StorageError> {
        let registry_id = Self::group_registry_actor_id().map_err(Self::map_core_error)?;
        let keys = self
            .fdb
            .list_keys(&registry_id, KEY_PREFIX_GROUP)
            .await
            .map_err(Self::map_core_error)?;

        let mut groups = Vec::new();
        for key in keys {
            if let Ok(Some(bytes)) = self.fdb.get(&registry_id, &key).await {
                if let Ok(group) = Self::deserialize_agent_group(&bytes) {
                    groups.push(group);
                }
            }
        }

        Ok(groups)
    }

    // =========================================================================
    // Identity Operations
    // =========================================================================

    async fn save_identity(&self, identity: &crate::models::Identity) -> Result<(), StorageError> {
        assert!(!identity.id.is_empty(), "identity id cannot be empty");

        let registry_id = Self::identity_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_IDENTITY, identity.id.as_bytes()].concat();
        let value = Self::serialize_identity(identity)?;

        self.fdb
            .set(&registry_id, &key, &value)
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    async fn load_identity(
        &self,
        id: &str,
    ) -> Result<Option<crate::models::Identity>, StorageError> {
        assert!(!id.is_empty(), "identity id cannot be empty");

        let registry_id = Self::identity_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_IDENTITY, id.as_bytes()].concat();

        match self.fdb.get(&registry_id, &key).await {
            Ok(Some(bytes)) => {
                let identity = Self::deserialize_identity(&bytes)?;
                Ok(Some(identity))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(Self::map_core_error(e)),
        }
    }

    async fn delete_identity(&self, id: &str) -> Result<(), StorageError> {
        assert!(!id.is_empty(), "identity id cannot be empty");

        let registry_id = Self::identity_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_IDENTITY, id.as_bytes()].concat();

        self.fdb
            .delete(&registry_id, &key)
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    async fn list_identities(&self) -> Result<Vec<crate::models::Identity>, StorageError> {
        let registry_id = Self::identity_registry_actor_id().map_err(Self::map_core_error)?;
        let keys = self
            .fdb
            .list_keys(&registry_id, KEY_PREFIX_IDENTITY)
            .await
            .map_err(Self::map_core_error)?;

        let mut identities = Vec::new();
        for key in keys {
            if let Ok(Some(bytes)) = self.fdb.get(&registry_id, &key).await {
                if let Ok(identity) = Self::deserialize_identity(&bytes) {
                    identities.push(identity);
                }
            }
        }

        Ok(identities)
    }

    // =========================================================================
    // Project Operations
    // =========================================================================

    async fn save_project(&self, project: &crate::models::Project) -> Result<(), StorageError> {
        assert!(!project.id.is_empty(), "project id cannot be empty");

        let registry_id = Self::project_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_PROJECT, project.id.as_bytes()].concat();
        let value = Self::serialize_project(project)?;

        self.fdb
            .set(&registry_id, &key, &value)
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    async fn load_project(&self, id: &str) -> Result<Option<crate::models::Project>, StorageError> {
        assert!(!id.is_empty(), "project id cannot be empty");

        let registry_id = Self::project_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_PROJECT, id.as_bytes()].concat();

        match self.fdb.get(&registry_id, &key).await {
            Ok(Some(bytes)) => {
                let project = Self::deserialize_project(&bytes)?;
                Ok(Some(project))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(Self::map_core_error(e)),
        }
    }

    async fn delete_project(&self, id: &str) -> Result<(), StorageError> {
        assert!(!id.is_empty(), "project id cannot be empty");

        let registry_id = Self::project_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_PROJECT, id.as_bytes()].concat();

        self.fdb
            .delete(&registry_id, &key)
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    async fn list_projects(&self) -> Result<Vec<crate::models::Project>, StorageError> {
        let registry_id = Self::project_registry_actor_id().map_err(Self::map_core_error)?;
        let keys = self
            .fdb
            .list_keys(&registry_id, KEY_PREFIX_PROJECT)
            .await
            .map_err(Self::map_core_error)?;

        let mut projects = Vec::new();
        for key in keys {
            if let Ok(Some(bytes)) = self.fdb.get(&registry_id, &key).await {
                if let Ok(project) = Self::deserialize_project(&bytes) {
                    projects.push(project);
                }
            }
        }

        Ok(projects)
    }

    // =========================================================================
    // Job Operations
    // =========================================================================

    async fn save_job(&self, job: &crate::models::Job) -> Result<(), StorageError> {
        assert!(!job.id.is_empty(), "job id cannot be empty");

        let registry_id = Self::job_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_JOB, job.id.as_bytes()].concat();
        let value = Self::serialize_job(job)?;

        self.fdb
            .set(&registry_id, &key, &value)
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    async fn load_job(&self, id: &str) -> Result<Option<crate::models::Job>, StorageError> {
        assert!(!id.is_empty(), "job id cannot be empty");

        let registry_id = Self::job_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_JOB, id.as_bytes()].concat();

        match self.fdb.get(&registry_id, &key).await {
            Ok(Some(bytes)) => {
                let job = Self::deserialize_job(&bytes)?;
                Ok(Some(job))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(Self::map_core_error(e)),
        }
    }

    async fn delete_job(&self, id: &str) -> Result<(), StorageError> {
        assert!(!id.is_empty(), "job id cannot be empty");

        let registry_id = Self::job_registry_actor_id().map_err(Self::map_core_error)?;
        let key = [KEY_PREFIX_JOB, id.as_bytes()].concat();

        self.fdb
            .delete(&registry_id, &key)
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    async fn list_jobs(&self) -> Result<Vec<crate::models::Job>, StorageError> {
        let registry_id = Self::job_registry_actor_id().map_err(Self::map_core_error)?;
        let keys = self
            .fdb
            .list_keys(&registry_id, KEY_PREFIX_JOB)
            .await
            .map_err(Self::map_core_error)?;

        let mut jobs = Vec::new();
        for key in keys {
            if let Ok(Some(bytes)) = self.fdb.get(&registry_id, &key).await {
                if let Ok(job) = Self::deserialize_job(&bytes) {
                    jobs.push(job);
                }
            }
        }

        Ok(jobs)
    }

    // =========================================================================
    // Archival Memory Operations (Per-Agent)
    // =========================================================================

    async fn save_archival_entry(
        &self,
        agent_id: &str,
        entry: &ArchivalEntry,
    ) -> Result<(), StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");
        assert!(!entry.id.is_empty(), "entry id cannot be empty");

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;
        let key = format!(
            "{}{}",
            String::from_utf8_lossy(KEY_PREFIX_ARCHIVAL),
            entry.id
        );
        let value = Self::serialize_archival_entry(entry)?;

        self.fdb
            .set(&actor_id, key.as_bytes(), &value)
            .await
            .map_err(Self::map_core_error)?;

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

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;

        // Scan all archival entries
        let kvs = self
            .fdb
            .scan_prefix(&actor_id, KEY_PREFIX_ARCHIVAL)
            .await
            .map_err(Self::map_core_error)?;

        let mut entries = Vec::new();
        for (_key, value) in kvs {
            if entries.len() >= limit {
                break;
            }
            let entry = Self::deserialize_archival_entry(&value)?;
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

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;
        let key = format!(
            "{}{}",
            String::from_utf8_lossy(KEY_PREFIX_ARCHIVAL),
            entry_id
        );

        match self.fdb.get(&actor_id, key.as_bytes()).await {
            Ok(Some(bytes)) => {
                let entry = Self::deserialize_archival_entry(&bytes)?;
                Ok(Some(entry))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(Self::map_core_error(e)),
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

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;
        let key = format!(
            "{}{}",
            String::from_utf8_lossy(KEY_PREFIX_ARCHIVAL),
            entry_id
        );

        self.fdb
            .delete(&actor_id, key.as_bytes())
            .await
            .map_err(Self::map_core_error)?;

        Ok(())
    }

    async fn delete_archival_entries(&self, agent_id: &str) -> Result<(), StorageError> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent id cannot be empty");

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;

        // Scan and delete all archival entries
        let kvs = self
            .fdb
            .scan_prefix(&actor_id, KEY_PREFIX_ARCHIVAL)
            .await
            .map_err(Self::map_core_error)?;

        for (key, _) in kvs {
            self.fdb
                .delete(&actor_id, &key)
                .await
                .map_err(Self::map_core_error)?;
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

        let actor_id = Self::agent_actor_id(agent_id).map_err(Self::map_core_error)?;

        // Scan all archival entries
        let kvs = self
            .fdb
            .scan_prefix(&actor_id, KEY_PREFIX_ARCHIVAL)
            .await
            .map_err(Self::map_core_error)?;

        let mut entries = Vec::new();
        for (_key, value) in kvs {
            let entry = Self::deserialize_archival_entry(&value)?;

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
    use crate::models::AgentType;

    #[test]
    fn test_registry_actor_id() {
        let id = FdbAgentRegistry::registry_actor_id().unwrap();
        assert_eq!(id.namespace(), "system");
        assert_eq!(id.id(), "agent_registry");
    }

    #[test]
    fn test_agent_actor_id() {
        let id = FdbAgentRegistry::agent_actor_id("test-agent").unwrap();
        assert_eq!(id.namespace(), "agents");
        assert_eq!(id.id(), "test-agent");
    }

    #[test]
    fn test_metadata_serialization() {
        let agent = AgentMetadata::new(
            "test-123".to_string(),
            "Test Agent".to_string(),
            AgentType::MemgptAgent,
        );

        let bytes = FdbAgentRegistry::serialize_metadata(&agent).unwrap();
        let deserialized = FdbAgentRegistry::deserialize_metadata(&bytes).unwrap();

        assert_eq!(agent.id, deserialized.id);
        assert_eq!(agent.name, deserialized.name);
    }
}
