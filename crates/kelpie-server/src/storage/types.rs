//! Storage Data Types
//!
//! TigerStyle: Explicit types with clear ownership and serialization.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::AgentType;

// =============================================================================
// Constants (TigerStyle: units in name)
// =============================================================================

/// Maximum agent name length in bytes
pub const AGENT_NAME_LENGTH_BYTES_MAX: usize = 256;

/// Maximum session ID length in bytes
pub const SESSION_ID_LENGTH_BYTES_MAX: usize = 64;

/// Maximum pending tool calls per session
pub const PENDING_TOOL_CALLS_MAX: usize = 10;

/// Maximum tool input size in bytes
pub const TOOL_INPUT_SIZE_BYTES_MAX: usize = 1024 * 1024; // 1MB

/// Maximum messages to load by default
#[allow(dead_code)]
pub const MESSAGES_LOAD_LIMIT_DEFAULT: usize = 50;

/// Maximum messages to load
#[allow(dead_code)]
pub const MESSAGES_LOAD_LIMIT_MAX: usize = 1000;

/// Maximum custom tool name length in bytes
///
/// NOTE: Existing tools (core memory tools) bypass this limit.
/// Only custom/client tools are validated against TOOL_NAME_LENGTH_MAX
#[allow(dead_code)]
pub const TOOL_NAME_LENGTH_MAX: usize = 255;

// =============================================================================
// Agent Metadata
// =============================================================================

/// Agent metadata stored in FDB
///
/// TigerStyle: This is the "cold" configuration data, separate from
/// dynamic session state. Rarely changes after creation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentMetadata {
    /// Unique identifier (UUID)
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Agent type determines capabilities
    pub agent_type: AgentType,

    /// LLM model to use (e.g., "claude-3-opus")
    pub model: Option<String>,

    /// Embedding model to use (e.g., "openai/text-embedding-3-small")
    pub embedding: Option<String>,

    /// System prompt override
    pub system: Option<String>,

    /// Description for display
    pub description: Option<String>,

    /// Attached tool IDs
    pub tool_ids: Vec<String>,

    /// Tags for organization
    pub tags: Vec<String>,

    /// Arbitrary metadata
    pub metadata: Value,

    /// Creation timestamp
    pub created_at: DateTime<Utc>,

    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

// =============================================================================
// Custom Tool Metadata
// =============================================================================

/// Custom tool definition stored in durable storage
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CustomToolRecord {
    /// Tool name (unique)
    pub name: String,
    /// Description for LLM and user display
    pub description: String,
    /// JSON schema for inputs
    pub input_schema: Value,
    /// Source code (runtime-specific)
    pub source_code: String,
    /// Runtime identifier (python, javascript, etc.)
    pub runtime: String,
    /// Optional runtime requirements
    pub requirements: Vec<String>,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

impl AgentMetadata {
    /// Create new agent metadata with defaults
    pub fn new(id: String, name: String, agent_type: AgentType) -> Self {
        // Preconditions
        assert!(!id.is_empty(), "agent id cannot be empty");
        assert!(!name.is_empty(), "agent name cannot be empty");
        assert!(
            name.len() <= AGENT_NAME_LENGTH_BYTES_MAX,
            "agent name exceeds maximum length"
        );

        let now = Utc::now();

        Self {
            id,
            name,
            agent_type,
            model: None,
            embedding: None,
            system: None,
            description: None,
            tool_ids: Vec::new(),
            tags: Vec::new(),
            metadata: Value::Object(serde_json::Map::new()),
            created_at: now,
            updated_at: now,
        }
    }

    /// Update the updated_at timestamp
    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }
}

// =============================================================================
// Session State
// =============================================================================

/// Session state for crash recovery
///
/// TigerStyle: Checkpointed after each agent loop iteration.
/// Contains everything needed to resume a crashed session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionState {
    /// Session identifier (UUID)
    pub session_id: String,

    /// Agent this session belongs to
    pub agent_id: String,

    /// Current loop iteration (0-indexed)
    pub iteration: u32,

    /// Pause until this timestamp (milliseconds since epoch)
    pub pause_until_ms: Option<u64>,

    /// Tool calls that were planned but may not have completed
    pub pending_tool_calls: Vec<PendingToolCall>,

    /// Result of the last completed tool call
    pub last_tool_result: Option<String>,

    /// Stop reason if session ended
    pub stop_reason: Option<String>,

    /// When the session started
    pub started_at: DateTime<Utc>,

    /// When this checkpoint was written
    pub checkpointed_at: DateTime<Utc>,
}

impl SessionState {
    /// Create a new session state
    pub fn new(session_id: String, agent_id: String) -> Self {
        // Preconditions
        assert!(!session_id.is_empty(), "session id cannot be empty");
        assert!(
            session_id.len() <= SESSION_ID_LENGTH_BYTES_MAX,
            "session id exceeds maximum length"
        );
        assert!(!agent_id.is_empty(), "agent id cannot be empty");

        let now = Utc::now();

        Self {
            session_id,
            agent_id,
            iteration: 0,
            pause_until_ms: None,
            pending_tool_calls: Vec::new(),
            last_tool_result: None,
            stop_reason: None,
            started_at: now,
            checkpointed_at: now,
        }
    }

    /// Checkpoint the session (update timestamp)
    pub fn checkpoint(&mut self) {
        self.checkpointed_at = Utc::now();
    }

    /// Increment iteration and checkpoint
    pub fn advance_iteration(&mut self) {
        self.iteration += 1;
        self.checkpoint();
    }

    /// Check if session is paused
    pub fn is_paused(&self, current_time_ms: u64) -> bool {
        self.pause_until_ms
            .map(|until| current_time_ms < until)
            .unwrap_or(false)
    }

    /// Set pause duration
    pub fn set_pause(&mut self, until_ms: u64) {
        self.pause_until_ms = Some(until_ms);
        self.checkpoint();
    }

    /// Clear pause
    pub fn clear_pause(&mut self) {
        self.pause_until_ms = None;
        self.checkpoint();
    }

    /// Add a pending tool call
    pub fn add_pending_tool(&mut self, tool: PendingToolCall) {
        // Precondition
        assert!(
            self.pending_tool_calls.len() < PENDING_TOOL_CALLS_MAX,
            "too many pending tool calls"
        );

        self.pending_tool_calls.push(tool);
    }

    /// Clear pending tool calls (after completion)
    pub fn clear_pending_tools(&mut self) {
        self.pending_tool_calls.clear();
    }

    /// Mark session as stopped
    pub fn stop(&mut self, reason: &str) {
        self.stop_reason = Some(reason.to_string());
        self.checkpoint();
    }

    /// Check if session has stopped
    pub fn is_stopped(&self) -> bool {
        self.stop_reason.is_some()
    }
}

// =============================================================================
// Pending Tool Call
// =============================================================================

/// A tool call that was started but may not have completed
///
/// TigerStyle: Used for crash recovery to avoid re-executing completed tools.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingToolCall {
    /// Unique identifier for this call
    pub id: String,

    /// Tool name
    pub tool_name: String,

    /// Tool input (JSON)
    pub tool_input: Value,

    /// When the tool call started
    pub started_at: DateTime<Utc>,

    /// Whether the tool completed (for idempotency)
    pub completed: bool,

    /// Tool result if completed
    pub result: Option<String>,
}

impl PendingToolCall {
    /// Create a new pending tool call
    pub fn new(id: String, tool_name: String, tool_input: Value) -> Self {
        // Preconditions
        assert!(!id.is_empty(), "tool call id cannot be empty");
        assert!(!tool_name.is_empty(), "tool name cannot be empty");

        // Check input size
        let input_size = serde_json::to_string(&tool_input)
            .map(|s| s.len())
            .unwrap_or(0);
        assert!(
            input_size <= TOOL_INPUT_SIZE_BYTES_MAX,
            "tool input exceeds maximum size"
        );

        Self {
            id,
            tool_name,
            tool_input,
            started_at: Utc::now(),
            completed: false,
            result: None,
        }
    }

    /// Mark the tool call as completed
    pub fn complete(&mut self, result: String) {
        self.completed = true;
        self.result = Some(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_metadata_new() {
        let meta = AgentMetadata::new(
            "agent-123".to_string(),
            "Test Agent".to_string(),
            AgentType::MemgptAgent,
        );

        assert_eq!(meta.id, "agent-123");
        assert_eq!(meta.name, "Test Agent");
        assert_eq!(meta.agent_type, AgentType::MemgptAgent);
        assert!(meta.model.is_none());
        assert!(meta.tool_ids.is_empty());
    }

    #[test]
    fn test_session_state_new() {
        let session = SessionState::new("session-456".to_string(), "agent-123".to_string());

        assert_eq!(session.session_id, "session-456");
        assert_eq!(session.agent_id, "agent-123");
        assert_eq!(session.iteration, 0);
        assert!(session.pause_until_ms.is_none());
        assert!(!session.is_stopped());
    }

    #[test]
    fn test_session_state_advance() {
        let mut session = SessionState::new("session-456".to_string(), "agent-123".to_string());

        session.advance_iteration();
        assert_eq!(session.iteration, 1);

        session.advance_iteration();
        assert_eq!(session.iteration, 2);
    }

    #[test]
    fn test_session_state_pause() {
        let mut session = SessionState::new("session-456".to_string(), "agent-123".to_string());

        // Not paused initially
        assert!(!session.is_paused(1000));

        // Set pause until 5000ms
        session.set_pause(5000);
        assert!(session.is_paused(1000));
        assert!(session.is_paused(4999));
        assert!(!session.is_paused(5000));
        assert!(!session.is_paused(6000));

        // Clear pause
        session.clear_pause();
        assert!(!session.is_paused(1000));
    }

    #[test]
    fn test_session_state_stop() {
        let mut session = SessionState::new("session-456".to_string(), "agent-123".to_string());

        assert!(!session.is_stopped());

        session.stop("max_iterations");
        assert!(session.is_stopped());
        assert_eq!(session.stop_reason, Some("max_iterations".to_string()));
    }

    #[test]
    fn test_pending_tool_call() {
        let mut tool = PendingToolCall::new(
            "call-789".to_string(),
            "shell".to_string(),
            serde_json::json!({"command": "echo hello"}),
        );

        assert!(!tool.completed);
        assert!(tool.result.is_none());

        tool.complete("hello\n".to_string());
        assert!(tool.completed);
        assert_eq!(tool.result, Some("hello\n".to_string()));
    }

    #[test]
    #[should_panic(expected = "agent id cannot be empty")]
    fn test_agent_metadata_empty_id() {
        AgentMetadata::new(
            "".to_string(),
            "Test Agent".to_string(),
            AgentType::MemgptAgent,
        );
    }

    #[test]
    #[should_panic(expected = "session id cannot be empty")]
    fn test_session_state_empty_id() {
        SessionState::new("".to_string(), "agent-123".to_string());
    }
}
