//! Storage Traits
//!
//! TigerStyle: Trait-based abstraction for DST compatibility.
//!
//! The `AgentStorage` trait allows swapping between:
//! - `SimStorage` - In-memory with fault injection for DST
//! - `FdbStorage` - FoundationDB for production

use async_trait::async_trait;
use thiserror::Error;

use crate::models::{ArchivalEntry, Block, Message};

use super::types::{AgentMetadata, CustomToolRecord, SessionState};

// =============================================================================
// Storage Error
// =============================================================================

/// Errors from storage operations
#[derive(Error, Debug, Clone)]
pub enum StorageError {
    /// Entity not found
    #[error("not found: {resource} with id '{id}'")]
    NotFound { resource: &'static str, id: String },

    /// Entity already exists
    #[error("already exists: {resource} with id '{id}'")]
    AlreadyExists { resource: &'static str, id: String },

    /// Write failed
    #[error("write failed: {operation}, reason: {reason}")]
    WriteFailed { operation: String, reason: String },

    /// Read failed
    #[error("read failed: {operation}, reason: {reason}")]
    ReadFailed { operation: String, reason: String },

    /// Transaction conflict
    #[error("transaction conflict: {reason}")]
    TransactionConflict { reason: String },

    /// Serialization error
    #[error("serialization failed: {reason}")]
    SerializationFailed { reason: String },

    /// Deserialization error
    #[error("deserialization failed: {reason}")]
    DeserializationFailed { reason: String },

    /// Connection error
    #[error("connection failed: {reason}")]
    ConnectionFailed { reason: String },

    /// Limit exceeded
    #[error("limit exceeded: {resource}, limit: {limit}")]
    LimitExceeded {
        resource: &'static str,
        limit: usize,
    },

    /// Fault injected (DST only)
    #[cfg(feature = "dst")]
    #[error("fault injected: {operation}")]
    FaultInjected { operation: String },

    /// Internal error
    #[error("internal error: {message}")]
    Internal { message: String },
}

impl StorageError {
    /// Check if this error is retriable
    pub fn is_retriable(&self) -> bool {
        matches!(
            self,
            StorageError::WriteFailed { .. }
                | StorageError::ReadFailed { .. }
                | StorageError::TransactionConflict { .. }
                | StorageError::ConnectionFailed { .. }
        ) || {
            #[cfg(feature = "dst")]
            {
                matches!(self, StorageError::FaultInjected { .. })
            }
            #[cfg(not(feature = "dst"))]
            {
                false
            }
        }
    }

    /// Check if this error indicates data not found
    pub fn is_not_found(&self) -> bool {
        matches!(self, StorageError::NotFound { .. })
    }
}

// Convert StorageError to kelpie_core::Error for DST tests
impl From<StorageError> for kelpie_core::Error {
    fn from(err: StorageError) -> Self {
        match err {
            StorageError::NotFound { resource, id } => {
                if resource == "agent" {
                    kelpie_core::Error::ActorNotFound { id }
                } else {
                    kelpie_core::Error::Internal {
                        message: format!("{} not found: {}", resource, id),
                    }
                }
            }
            StorageError::AlreadyExists { resource, id } => kelpie_core::Error::Internal {
                message: format!("{} with id {} already exists", resource, id),
            },
            StorageError::WriteFailed { operation, reason } => kelpie_core::Error::Internal {
                message: format!("Write failed: {} - {}", operation, reason),
            },
            StorageError::ReadFailed { operation, reason } => kelpie_core::Error::Internal {
                message: format!("Read failed: {} - {}", operation, reason),
            },
            StorageError::TransactionConflict { reason } => kelpie_core::Error::Internal {
                message: format!("Transaction conflict: {}", reason),
            },
            StorageError::SerializationFailed { reason } => kelpie_core::Error::Internal {
                message: format!("Serialization failed: {}", reason),
            },
            StorageError::DeserializationFailed { reason } => kelpie_core::Error::Internal {
                message: format!("Deserialization failed: {}", reason),
            },
            StorageError::ConnectionFailed { reason } => kelpie_core::Error::Internal {
                message: format!("Connection failed: {}", reason),
            },
            StorageError::LimitExceeded { resource, limit } => kelpie_core::Error::Internal {
                message: format!("{} limit exceeded: {}", resource, limit),
            },
            #[cfg(feature = "dst")]
            StorageError::FaultInjected { operation } => kelpie_core::Error::Internal {
                message: format!("Fault injected: {}", operation),
            },
            StorageError::Internal { message } => kelpie_core::Error::Internal { message },
        }
    }
}

// =============================================================================
// Agent Storage Trait
// =============================================================================

/// Storage backend for agent persistence
///
/// TigerStyle: All operations are async and return explicit errors.
/// Implementations must be Send + Sync for concurrent access.
#[async_trait]
pub trait AgentStorage: Send + Sync {
    // =========================================================================
    // Agent Metadata Operations
    // =========================================================================

    /// Save agent metadata (create or update)
    async fn save_agent(&self, agent: &AgentMetadata) -> Result<(), StorageError>;

    /// Load agent metadata by ID
    async fn load_agent(&self, id: &str) -> Result<Option<AgentMetadata>, StorageError>;

    /// Delete agent and all associated data
    async fn delete_agent(&self, id: &str) -> Result<(), StorageError>;

    /// List all agents
    async fn list_agents(&self) -> Result<Vec<AgentMetadata>, StorageError>;

    /// Check if agent exists
    async fn agent_exists(&self, id: &str) -> Result<bool, StorageError> {
        Ok(self.load_agent(id).await?.is_some())
    }

    // =========================================================================
    // Core Memory Block Operations
    // =========================================================================

    /// Save core memory blocks for an agent
    async fn save_blocks(&self, agent_id: &str, blocks: &[Block]) -> Result<(), StorageError>;

    /// Load core memory blocks for an agent
    async fn load_blocks(&self, agent_id: &str) -> Result<Vec<Block>, StorageError>;

    /// Update a single block by label
    async fn update_block(
        &self,
        agent_id: &str,
        label: &str,
        value: &str,
    ) -> Result<Block, StorageError>;

    /// Append to a block (create if not exists)
    async fn append_block(
        &self,
        agent_id: &str,
        label: &str,
        content: &str,
    ) -> Result<Block, StorageError>;

    // =========================================================================
    // Session State Operations (Checkpointing)
    // =========================================================================

    /// Save session state (checkpoint)
    async fn save_session(&self, state: &SessionState) -> Result<(), StorageError>;

    /// Load session state
    async fn load_session(
        &self,
        agent_id: &str,
        session_id: &str,
    ) -> Result<Option<SessionState>, StorageError>;

    /// Delete session state
    async fn delete_session(&self, agent_id: &str, session_id: &str) -> Result<(), StorageError>;

    /// List active sessions for an agent
    async fn list_sessions(&self, agent_id: &str) -> Result<Vec<SessionState>, StorageError>;

    // =========================================================================
    // Custom Tool Operations
    // =========================================================================

    /// Save a custom tool definition
    async fn save_custom_tool(&self, tool: &CustomToolRecord) -> Result<(), StorageError>;

    /// Load a custom tool definition by name
    async fn load_custom_tool(&self, name: &str) -> Result<Option<CustomToolRecord>, StorageError>;

    /// Delete a custom tool definition
    async fn delete_custom_tool(&self, name: &str) -> Result<(), StorageError>;

    /// List all custom tool definitions
    async fn list_custom_tools(&self) -> Result<Vec<CustomToolRecord>, StorageError>;

    /// Get the latest session for an agent (for resume)
    async fn load_latest_session(
        &self,
        agent_id: &str,
    ) -> Result<Option<SessionState>, StorageError> {
        let sessions = self.list_sessions(agent_id).await?;
        Ok(sessions
            .into_iter()
            .filter(|s| !s.is_stopped())
            .max_by_key(|s| s.checkpointed_at))
    }

    // =========================================================================
    // Message Operations
    // =========================================================================

    /// Append a message to conversation history
    async fn append_message(&self, agent_id: &str, message: &Message) -> Result<(), StorageError>;

    /// Load recent messages (most recent first)
    async fn load_messages(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<Message>, StorageError>;

    /// Load messages since a timestamp (for incremental sync)
    async fn load_messages_since(
        &self,
        agent_id: &str,
        since_ms: u64,
    ) -> Result<Vec<Message>, StorageError>;

    /// Count messages for an agent
    async fn count_messages(&self, agent_id: &str) -> Result<usize, StorageError>;

    /// Delete all messages for an agent
    async fn delete_messages(&self, agent_id: &str) -> Result<(), StorageError>;

    // =========================================================================
    // Archival Memory Operations
    // =========================================================================

    /// Save an archival entry (long-term memory)
    async fn save_archival_entry(
        &self,
        agent_id: &str,
        entry: &ArchivalEntry,
    ) -> Result<(), StorageError>;

    /// Load archival entries for an agent
    async fn load_archival_entries(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<ArchivalEntry>, StorageError>;

    /// Get a specific archival entry by ID
    async fn get_archival_entry(
        &self,
        agent_id: &str,
        entry_id: &str,
    ) -> Result<Option<ArchivalEntry>, StorageError>;

    /// Delete an archival entry
    async fn delete_archival_entry(
        &self,
        agent_id: &str,
        entry_id: &str,
    ) -> Result<(), StorageError>;

    /// Delete all archival entries for an agent
    async fn delete_archival_entries(&self, agent_id: &str) -> Result<(), StorageError>;

    /// Search archival entries (basic text search)
    /// Returns entries matching the query, limited by `limit`
    async fn search_archival_entries(
        &self,
        agent_id: &str,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ArchivalEntry>, StorageError>;

    // =========================================================================
    // Transactional Operations
    // =========================================================================

    /// Atomic checkpoint: save session state + append message
    ///
    /// This is used at the end of each agent loop iteration to ensure
    /// session state and messages are consistent.
    async fn checkpoint(
        &self,
        session: &SessionState,
        message: Option<&Message>,
    ) -> Result<(), StorageError> {
        // Default implementation: non-atomic (override for FDB)
        self.save_session(session).await?;
        if let Some(msg) = message {
            self.append_message(&session.agent_id, msg).await?;
        }
        Ok(())
    }

    // =========================================================================
    // MCP Server Operations
    // =========================================================================

    /// Save MCP server configuration
    async fn save_mcp_server(&self, server: &crate::models::MCPServer) -> Result<(), StorageError>;

    /// Load MCP server by ID
    async fn load_mcp_server(
        &self,
        id: &str,
    ) -> Result<Option<crate::models::MCPServer>, StorageError>;

    /// Delete MCP server
    async fn delete_mcp_server(&self, id: &str) -> Result<(), StorageError>;

    /// List all MCP servers
    async fn list_mcp_servers(&self) -> Result<Vec<crate::models::MCPServer>, StorageError>;

    // =========================================================================
    // Agent Group Operations
    // =========================================================================

    /// Save agent group
    async fn save_agent_group(&self, group: &crate::models::AgentGroup)
        -> Result<(), StorageError>;

    /// Load agent group by ID
    async fn load_agent_group(
        &self,
        id: &str,
    ) -> Result<Option<crate::models::AgentGroup>, StorageError>;

    /// Delete agent group
    async fn delete_agent_group(&self, id: &str) -> Result<(), StorageError>;

    /// List all agent groups
    async fn list_agent_groups(&self) -> Result<Vec<crate::models::AgentGroup>, StorageError>;

    // =========================================================================
    // Identity Operations
    // =========================================================================

    /// Save identity
    async fn save_identity(&self, identity: &crate::models::Identity) -> Result<(), StorageError>;

    /// Load identity by ID
    async fn load_identity(
        &self,
        id: &str,
    ) -> Result<Option<crate::models::Identity>, StorageError>;

    /// Delete identity
    async fn delete_identity(&self, id: &str) -> Result<(), StorageError>;

    /// List all identities
    async fn list_identities(&self) -> Result<Vec<crate::models::Identity>, StorageError>;

    // =========================================================================
    // Project Operations
    // =========================================================================

    /// Save project
    async fn save_project(&self, project: &crate::models::Project) -> Result<(), StorageError>;

    /// Load project by ID
    async fn load_project(&self, id: &str) -> Result<Option<crate::models::Project>, StorageError>;

    /// Delete project
    async fn delete_project(&self, id: &str) -> Result<(), StorageError>;

    /// List all projects
    async fn list_projects(&self) -> Result<Vec<crate::models::Project>, StorageError>;

    // =========================================================================
    // Job Operations
    // =========================================================================

    /// Save scheduled job
    async fn save_job(&self, job: &crate::models::Job) -> Result<(), StorageError>;

    /// Load job by ID
    async fn load_job(&self, id: &str) -> Result<Option<crate::models::Job>, StorageError>;

    /// Delete job
    async fn delete_job(&self, id: &str) -> Result<(), StorageError>;

    /// List all jobs
    async fn list_jobs(&self) -> Result<Vec<crate::models::Job>, StorageError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_error_retriable() {
        let write_err = StorageError::WriteFailed {
            operation: "save_agent".to_string(),
            reason: "timeout".to_string(),
        };
        assert!(write_err.is_retriable());

        let not_found = StorageError::NotFound {
            resource: "agent",
            id: "123".to_string(),
        };
        assert!(!not_found.is_retriable());
        assert!(not_found.is_not_found());
    }
}
