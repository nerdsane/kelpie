//! API models for Letta-compatible REST API
//!
//! TigerStyle: These models mirror Letta's API schema for compatibility.

use chrono::{DateTime, Utc};
use croner::Cron;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// =============================================================================
// Agent Models
// =============================================================================

/// Agent type enumeration (matches Letta's agent types)
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::enum_variant_names)] // Matches Letta's API naming
pub enum AgentType {
    #[default]
    #[serde(alias = "memgpt")]
    MemgptAgent,
    #[serde(alias = "letta_v1")]
    LettaV1Agent,
    ReactAgent,
}

// =============================================================================
// Agent Capabilities (TigerStyle: Static capabilities per type)
// =============================================================================

/// Capabilities vary by agent type - determines what tools are available
///
/// TigerStyle: Use struct instead of trait for type-specific configuration.
/// Agent types differ in configuration, not behavior. The agent loop logic
/// is identical; only the available tools and settings differ.
#[derive(Debug, Clone)]
pub struct AgentCapabilities {
    /// Tools this agent type can use
    pub allowed_tools: Vec<String>,
    /// Whether this agent supports pause_heartbeats
    pub supports_heartbeats: bool,
    /// Default system prompt template (None = use default)
    pub system_prompt_template: Option<String>,
    /// Maximum agent loop iterations
    pub max_iterations: u32,
}

/// ReAct-style system prompt template
const REACT_PROMPT: &str = r#"You are a ReAct agent. Follow this pattern:
Thought: Think about what to do
Action: Use a tool
Observation: Observe the result
... repeat until done ...
Thought: I now know the answer
Final Answer: Your response"#;

impl AgentType {
    /// Get capabilities for this agent type
    ///
    /// TigerStyle: Static mapping - type determines capabilities.
    /// No per-agent capability persistence needed.
    pub fn capabilities(&self) -> AgentCapabilities {
        match self {
            AgentType::MemgptAgent => AgentCapabilities {
                allowed_tools: vec![
                    "shell".to_string(),
                    "core_memory_append".to_string(),
                    "core_memory_replace".to_string(),
                    "archival_memory_insert".to_string(),
                    "archival_memory_search".to_string(),
                    "conversation_search".to_string(),
                    "pause_heartbeats".to_string(),
                    "propose_improvement".to_string(),
                ],
                supports_heartbeats: true,
                system_prompt_template: None, // Use default
                max_iterations: 5,
            },
            AgentType::ReactAgent => AgentCapabilities {
                allowed_tools: vec!["shell".to_string()],
                supports_heartbeats: false,
                system_prompt_template: Some(REACT_PROMPT.to_string()),
                max_iterations: 10, // ReAct may need more iterations
            },
            AgentType::LettaV1Agent => AgentCapabilities {
                allowed_tools: vec![
                    "shell".to_string(),
                    "core_memory_append".to_string(),
                    "core_memory_replace".to_string(),
                ],
                supports_heartbeats: false,
                system_prompt_template: None,
                max_iterations: 5,
            },
        }
    }
}

/// Request to create a new agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAgentRequest {
    /// Name of the agent
    #[serde(default = "default_agent_name")]
    pub name: String,
    /// Agent type
    #[serde(default)]
    pub agent_type: AgentType,
    /// Model to use (e.g., "openai/gpt-4o")
    pub model: Option<String>,
    /// Embedding model to use (e.g., "openai/text-embedding-3-small")
    #[serde(default = "default_embedding_model")]
    pub embedding: Option<String>,
    /// System prompt
    pub system: Option<String>,
    /// Description of the agent
    pub description: Option<String>,
    /// Optional project ID (Phase 6: Projects)
    pub project_id: Option<String>,
    /// User ID (Letta compatibility - owner of the agent)
    #[serde(default)]
    pub user_id: Option<String>,
    /// Organization ID (Letta compatibility - org context)
    #[serde(default)]
    pub org_id: Option<String>,
    /// Initial memory blocks (inline creation)
    #[serde(default)]
    pub memory_blocks: Vec<CreateBlockRequest>,
    /// Existing block IDs to attach (letta-code compatibility)
    #[serde(default)]
    pub block_ids: Vec<String>,
    /// Tool IDs to attach
    #[serde(default)]
    pub tool_ids: Vec<String>,
    /// Tags for organization
    #[serde(default)]
    pub tags: Vec<String>,
    /// Additional metadata
    #[serde(default)]
    pub metadata: serde_json::Value,
}

fn default_agent_name() -> String {
    "Nameless Agent".to_string()
}

fn default_embedding_model() -> Option<String> {
    // Allow configuration via environment variable, fall back to sensible default
    std::env::var("KELPIE_DEFAULT_EMBEDDING_MODEL")
        .ok()
        .or_else(|| Some("openai/text-embedding-3-small".to_string()))
}

impl Default for CreateAgentRequest {
    fn default() -> Self {
        Self {
            name: default_agent_name(),
            agent_type: AgentType::default(),
            model: None,
            embedding: default_embedding_model(),
            system: None,
            description: None,
            project_id: None,
            user_id: None,
            org_id: None,
            memory_blocks: Vec::new(),
            block_ids: Vec::new(),
            tool_ids: Vec::new(),
            tags: Vec::new(),
            metadata: serde_json::Value::Null,
        }
    }
}

/// Request to update an agent
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateAgentRequest {
    /// New name
    pub name: Option<String>,
    /// New embedding model
    pub embedding: Option<String>,
    /// New system prompt
    pub system: Option<String>,
    /// New description
    pub description: Option<String>,
    /// New tags
    pub tags: Option<Vec<String>>,
    /// New metadata
    pub metadata: Option<serde_json::Value>,
    /// New tool IDs (replaces existing)
    pub tool_ids: Option<Vec<String>>,
}

/// Agent state response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    /// Unique identifier
    pub id: String,
    /// Agent name
    pub name: String,
    /// Agent type
    pub agent_type: AgentType,
    /// Model being used
    pub model: Option<String>,
    /// Embedding model being used
    pub embedding: Option<String>,
    /// System prompt
    pub system: Option<String>,
    /// Description
    pub description: Option<String>,
    /// Optional project ID (Phase 6: Projects)
    pub project_id: Option<String>,
    /// User ID (Letta compatibility - owner of the agent)
    #[serde(default)]
    pub user_id: Option<String>,
    /// Organization ID (Letta compatibility - org context)
    #[serde(default)]
    pub org_id: Option<String>,
    /// Memory blocks
    pub blocks: Vec<Block>,
    /// Attached tool IDs
    pub tool_ids: Vec<String>,
    /// Tags
    pub tags: Vec<String>,
    /// Metadata
    pub metadata: serde_json::Value,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

impl AgentState {
    /// Create a new agent state from a create request
    pub fn from_request(request: CreateAgentRequest) -> Self {
        let now = Utc::now();
        let id = Uuid::new_v4().to_string();

        let blocks = request
            .memory_blocks
            .into_iter()
            .map(Block::from_request)
            .collect();

        Self {
            id,
            name: request.name,
            agent_type: request.agent_type,
            model: request.model,
            embedding: request.embedding,
            system: request.system,
            description: request.description,
            project_id: request.project_id,
            user_id: request.user_id,
            org_id: request.org_id,
            blocks,
            tool_ids: request.tool_ids,
            tags: request.tags,
            metadata: request.metadata,
            created_at: now,
            updated_at: now,
        }
    }

    /// Apply an update to the agent state
    pub fn apply_update(&mut self, update: UpdateAgentRequest) {
        if let Some(name) = update.name {
            self.name = name;
        }
        if let Some(embedding) = update.embedding {
            self.embedding = Some(embedding);
        }
        if let Some(system) = update.system {
            self.system = Some(system);
        }
        if let Some(description) = update.description {
            self.description = Some(description);
        }
        if let Some(tags) = update.tags {
            self.tags = tags;
        }
        if let Some(metadata) = update.metadata {
            self.metadata = metadata;
        }
        if let Some(tool_ids) = update.tool_ids {
            self.tool_ids = tool_ids;
        }
        self.updated_at = Utc::now();
    }
}

// =============================================================================
// Memory Block Models
// =============================================================================

/// Request to create a memory block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBlockRequest {
    /// Label for the block (e.g., "persona", "human", "task")
    pub label: String,
    /// Initial value/content
    pub value: String,
    /// Description for LLM understanding
    pub description: Option<String>,
    /// Maximum size limit
    pub limit: Option<usize>,
}

/// Request to update a memory block
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateBlockRequest {
    /// New value
    pub value: Option<String>,
    /// New description
    pub description: Option<String>,
    /// New limit
    pub limit: Option<usize>,
}

/// Memory block response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    /// Unique identifier
    pub id: String,
    /// Block label
    pub label: String,
    /// Current value
    pub value: String,
    /// Description
    pub description: Option<String>,
    /// Size limit
    pub limit: Option<usize>,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

impl Block {
    /// Create a new block with label and value
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            label: label.into(),
            value: value.into(),
            description: None,
            limit: Some(20000), // Default for Letta compatibility
            created_at: now,
            updated_at: now,
        }
    }

    /// Create a block from a create request
    pub fn from_request(request: CreateBlockRequest) -> Self {
        let now = Utc::now();
        // Default limit to 20000 if not specified (Letta compatibility)
        let limit = request.limit.or(Some(20000));
        Self {
            id: Uuid::new_v4().to_string(),
            label: request.label,
            value: request.value,
            description: request.description,
            limit,
            created_at: now,
            updated_at: now,
        }
    }

    /// Apply an update
    pub fn apply_update(&mut self, update: UpdateBlockRequest) {
        if let Some(value) = update.value {
            self.value = value;
        }
        if let Some(description) = update.description {
            self.description = Some(description);
        }
        if let Some(limit) = update.limit {
            self.limit = Some(limit);
        }
        self.updated_at = Utc::now();
    }
}

// =============================================================================
// Message Models
// =============================================================================

/// Message role
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

/// Request to send a message to an agent
/// Supports multiple formats for letta-code compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMessageRequest {
    /// Message role (defaults to "user" if not provided)
    #[serde(default = "default_role")]
    pub role: MessageRole,
    /// Message content - supports both "content" and "text" field names
    #[serde(alias = "text", default)]
    pub content: String,
    /// Optional tool call ID (for tool responses)
    pub tool_call_id: Option<String>,
    /// Optional messages array (letta-code sends this format)
    /// If provided, takes precedence over content field
    pub messages: Option<Vec<LettaMessage>>,
    /// Optional message field (another letta-code format)
    #[serde(alias = "message")]
    pub msg: Option<String>,
    /// Enable streaming response
    #[serde(default)]
    pub streaming: bool,
    /// Stream individual tokens (not just message chunks)
    #[serde(default)]
    pub stream_tokens: bool,
    /// Client-side tools that require approval before execution
    #[serde(default)]
    pub client_tools: Vec<ClientTool>,
}

/// Client-side tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientTool {
    /// Tool name
    pub name: String,
    /// Whether tool requires approval
    #[serde(default)]
    pub requires_approval: bool,
}

/// Approval response for client-side tools
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolApproval {
    /// Tool call ID being approved/rejected
    pub tool_call_id: String,
    /// Tool return value (result of client-side execution)
    pub tool_return: String,
    /// Status: "success" or "error"
    pub status: String,
}

/// Approval message type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ApprovalMessage {
    #[serde(rename = "approval")]
    Approval { approvals: Vec<ToolApproval> },
}

fn default_role() -> MessageRole {
    MessageRole::User
}

/// Letta message format (used in messages array)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LettaMessage {
    /// Role (user, assistant, system, tool)
    #[serde(default = "default_role")]
    pub role: MessageRole,
    /// Content - can be string or array of content blocks
    #[serde(default, deserialize_with = "deserialize_content")]
    pub content: Option<String>,
    /// Alternative text field
    pub text: Option<String>,
    /// Message type for special messages (e.g., "approval")
    #[serde(rename = "type")]
    pub message_type: Option<String>,
    /// Approvals for client-side tool execution results
    #[serde(default)]
    pub approvals: Vec<ToolApproval>,
}

/// Deserialize content that can be either a string or an array of content blocks
fn deserialize_content<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct ContentVisitor;

    impl<'de> Visitor<'de> for ContentVisitor {
        type Value = Option<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a string or an array of content blocks")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(value.to_string()))
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(value))
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: de::SeqAccess<'de>,
        {
            // Content is an array of content blocks, extract text from them
            let mut texts = Vec::new();
            while let Some(block) = seq.next_element::<serde_json::Value>()? {
                // Extract text from content block
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    texts.push(text.to_string());
                }
            }
            if texts.is_empty() {
                Ok(None)
            } else {
                Ok(Some(texts.join("\n")))
            }
        }
    }

    deserializer.deserialize_any(ContentVisitor)
}

impl LettaMessage {
    /// Get the effective text content from either content or text field
    pub fn get_text(&self) -> Option<&str> {
        self.content.as_deref().or(self.text.as_deref())
    }
}

impl CreateMessageRequest {
    /// Get the effective content and role from the request
    /// Handles multiple formats for letta-code compatibility
    pub fn effective_content(&self) -> Option<(MessageRole, String)> {
        // If messages array provided, use first message with content
        if let Some(ref msgs) = self.messages {
            for msg in msgs {
                if let Some(text) = msg.get_text() {
                    if !text.is_empty() {
                        return Some((msg.role.clone(), text.to_string()));
                    }
                }
            }
        }
        // Check msg field
        if let Some(ref msg) = self.msg {
            if !msg.is_empty() {
                return Some((self.role.clone(), msg.clone()));
            }
        }
        // Fall back to direct content
        if !self.content.is_empty() {
            return Some((self.role.clone(), self.content.clone()));
        }
        None
    }
}

/// Message response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Unique identifier
    pub id: String,
    /// Agent ID
    pub agent_id: String,
    /// Message type discriminator (letta-code compatibility)
    pub message_type: String,
    /// Message role (legacy)
    pub role: MessageRole,
    /// Message content
    pub content: String,
    /// Tool call ID if this is a tool response
    pub tool_call_id: Option<String>,
    /// Tool calls made by assistant (OpenAI format - plural array)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Single tool call (Letta SDK format - singular)
    /// Used for tool_call_message types to match Letta SDK expectations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<LettaToolCall>,
    /// Tool return result (Letta SDK format)
    /// Used for tool_return_message types
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_return: Option<String>,
    /// Tool execution status ("success" or "error")
    /// Used for tool_return_message types
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Creation timestamp
    #[serde(rename = "date")]
    pub created_at: DateTime<Utc>,
}

impl Message {
    /// Get message_type from role
    pub fn message_type_from_role(role: &MessageRole) -> String {
        match role {
            MessageRole::User => "user_message".to_string(),
            MessageRole::Assistant => "assistant_message".to_string(),
            MessageRole::System => "system_message".to_string(),
            MessageRole::Tool => "tool_return_message".to_string(),
        }
    }
}

/// Tool call in a message (OpenAI format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Tool call ID
    pub id: String,
    /// Tool name
    pub name: String,
    /// Tool arguments as JSON
    pub arguments: serde_json::Value,
}

/// Tool call in Letta format (singular, with tool_call_id inside)
/// Used for tool_call_message types to match Letta SDK expectations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LettaToolCall {
    /// Tool name
    pub name: String,
    /// Tool arguments as JSON string (Letta SDK expects string, not object)
    pub arguments: String,
    /// Tool call ID
    pub tool_call_id: String,
}

/// Tool that requires client-side execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// Tool call ID
    pub tool_call_id: String,
    /// Tool name
    pub tool_name: String,
    /// Tool arguments as JSON
    pub tool_arguments: serde_json::Value,
}

/// Response from sending a message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageResponse {
    /// Messages generated (may include multiple for tool use)
    pub messages: Vec<Message>,
    /// Usage statistics
    pub usage: Option<UsageStats>,
    /// Stop reason (for letta-code compatibility)
    #[serde(default = "default_stop_reason")]
    pub stop_reason: String,
    /// Tools that need client-side execution (when stop_reason is "requires_approval")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_requests: Option<Vec<ApprovalRequest>>,
}

fn default_stop_reason() -> String {
    "end_turn".to_string()
}

/// Token usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

// =============================================================================
// Batch Message Models
// =============================================================================

/// Request to send a batch of messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchMessagesRequest {
    pub messages: Vec<CreateMessageRequest>,
}

/// Result for a single message in a batch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchMessageResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<MessageResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Batch execution status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchStatus {
    pub id: String,
    pub agent_id: String,
    pub total: usize,
    pub completed: usize,
    pub results: Vec<BatchMessageResult>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// =============================================================================
// Tool Models
// =============================================================================

/// Tool definition (reserved for future LLM tool integration)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ToolDefinition {
    /// Unique identifier
    pub id: String,
    /// Tool name
    pub name: String,
    /// Description for LLM
    pub description: String,
    /// JSON schema for parameters
    pub parameters: serde_json::Value,
    /// Whether this is a built-in tool
    pub builtin: bool,
}

// =============================================================================
// API Response Wrappers
// =============================================================================

/// List response with pagination
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse<T> {
    /// Items in this page
    pub items: Vec<T>,
    /// Total count
    pub total: usize,
    /// Cursor for next page
    pub cursor: Option<String>,
}

/// Error response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// Error code
    pub code: String,
    /// Human-readable message
    pub message: String,
    /// Additional details
    pub details: Option<serde_json::Value>,
}

impl ErrorResponse {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn not_found(resource: &str, id: &str) -> Self {
        Self::new(
            "not_found",
            format!("{} with id '{}' not found", resource, id),
        )
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new("bad_request", message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal_error", message)
    }
}

// =============================================================================
// Archival Memory
// =============================================================================

/// Archival memory entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivalEntry {
    /// Entry ID
    pub id: String,
    /// Content stored
    pub content: String,
    /// Optional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// Creation timestamp
    pub created_at: String,
}

// =============================================================================
// Health Check
// =============================================================================

/// Health check response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
}

// =============================================================================
// Import/Export Models
// =============================================================================

/// Request to import an agent from external source
///
/// TigerStyle: Explicit structure for agent import with validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportAgentRequest {
    /// Agent configuration to import
    pub agent: AgentImportData,
    /// Optional messages to import (conversation history)
    #[serde(default)]
    pub messages: Vec<MessageImportData>,
}

/// Agent data for import (similar to AgentState but without generated fields)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentImportData {
    /// Agent name (required)
    pub name: String,
    /// Agent type
    #[serde(default)]
    pub agent_type: AgentType,
    /// Model being used
    pub model: Option<String>,
    /// Embedding model
    pub embedding: Option<String>,
    /// System prompt
    pub system: Option<String>,
    /// Description
    pub description: Option<String>,
    /// Memory blocks
    #[serde(default)]
    pub blocks: Vec<BlockImportData>,
    /// Attached tool IDs
    #[serde(default)]
    pub tool_ids: Vec<String>,
    /// Tags
    #[serde(default)]
    pub tags: Vec<String>,
    /// Metadata
    #[serde(default)]
    pub metadata: serde_json::Value,
    /// Project ID
    #[serde(default)]
    pub project_id: Option<String>,
}

/// Block data for import
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockImportData {
    /// Block label
    pub label: String,
    /// Current value
    pub value: String,
    /// Description
    pub description: Option<String>,
    /// Size limit
    pub limit: Option<usize>,
}

/// Message data for import
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageImportData {
    /// Message role
    pub role: MessageRole,
    /// Message content
    pub content: String,
    /// Tool call ID if this is a tool response
    pub tool_call_id: Option<String>,
    /// Tool calls made by assistant (OpenAI/Letta spec - plural array)
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

/// Response from exporting an agent
///
/// TigerStyle: Complete agent state for portability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportAgentResponse {
    /// Agent configuration
    pub agent: AgentState,
    /// Optional messages (conversation history)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub messages: Vec<Message>,
}

/// Streaming event emitted during agent message processing
///
/// Phase 7: Letta-compatible SSE events for real-time message streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// LLM thinking (assistant message chunk)
    MessageChunk { content: String },

    /// Tool call starting
    ToolCallStart {
        tool_call_id: String,
        tool_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
    },

    /// Tool call completed
    ToolCallComplete {
        tool_call_id: String,
        result: String,
    },

    /// Message processing complete
    MessageComplete { message_id: String },

    /// Error occurred during streaming
    Error { message: String },
}

impl StreamEvent {
    /// Get the SSE event name for this event type
    ///
    /// Used to set the "event:" field in Server-Sent Events
    pub fn event_name(&self) -> &'static str {
        match self {
            StreamEvent::MessageChunk { .. } => "message_chunk",
            StreamEvent::ToolCallStart { .. } => "tool_call_start",
            StreamEvent::ToolCallComplete { .. } => "tool_call_complete",
            StreamEvent::MessageComplete { .. } => "message_complete",
            StreamEvent::Error { .. } => "error",
        }
    }
}

// =========================================================================
// Scheduling models (Phase 5)
// =========================================================================

/// Job schedule type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleType {
    /// Cron expression (e.g., "0 0 * * *" for daily at midnight)
    Cron,
    /// Interval in seconds
    Interval,
    /// One-time execution at specific time
    Once,
}

/// Job action type (what the job does)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobAction {
    /// Send a message to the agent
    SendMessage,
    /// Summarize agent's conversation
    SummarizeConversation,
    /// Summarize agent's memory
    SummarizeMemory,
    /// Export agent state
    ExportAgent,
    /// Custom action (for extensibility)
    Custom,
}

/// Job status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Job is active and will run
    Active,
    /// Job is paused (won't run)
    Paused,
    /// Job completed (for one-time jobs)
    Completed,
    /// Job failed
    Failed,
}

/// Request to create a scheduled job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateJobRequest {
    /// Agent ID this job is for
    pub agent_id: String,
    /// Schedule type
    pub schedule_type: ScheduleType,
    /// Schedule pattern (cron expression, interval seconds, or ISO timestamp)
    pub schedule: String,
    /// Action to perform
    pub action: JobAction,
    /// Optional action parameters (JSON)
    #[serde(default)]
    pub action_params: serde_json::Value,
    /// Job description
    pub description: Option<String>,
}

/// Request to update a scheduled job
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateJobRequest {
    /// New status
    pub status: Option<JobStatus>,
    /// New schedule pattern
    pub schedule: Option<String>,
    /// New action parameters
    pub action_params: Option<serde_json::Value>,
    /// New description
    pub description: Option<String>,
}

/// Scheduled job response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    /// Unique identifier
    pub id: String,
    /// Agent ID
    pub agent_id: String,
    /// Schedule type
    pub schedule_type: ScheduleType,
    /// Schedule pattern
    pub schedule: String,
    /// Action to perform
    pub action: JobAction,
    /// Action parameters
    pub action_params: serde_json::Value,
    /// Job description
    pub description: Option<String>,
    /// Job status
    pub status: JobStatus,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last execution timestamp
    pub last_run: Option<DateTime<Utc>>,
    /// Next scheduled execution
    pub next_run: Option<DateTime<Utc>>,
    /// Execution count
    pub run_count: u64,
}

impl Job {
    /// Create a new job from request
    pub fn from_request(request: CreateJobRequest) -> Self {
        let now = Utc::now();
        let next_run = calculate_next_run(&request.schedule_type, &request.schedule, now);

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: request.agent_id,
            schedule_type: request.schedule_type,
            schedule: request.schedule,
            action: request.action,
            action_params: request.action_params,
            description: request.description,
            status: JobStatus::Active,
            created_at: now,
            last_run: None,
            next_run,
            run_count: 0,
        }
    }

    /// Apply an update to the job
    pub fn apply_update(&mut self, update: UpdateJobRequest) {
        if let Some(status) = update.status {
            self.status = status;
        }
        if let Some(schedule) = update.schedule {
            self.schedule = schedule.clone();
            // Recalculate next_run if schedule changed
            self.next_run = calculate_next_run(&self.schedule_type, &schedule, Utc::now());
        }
        if let Some(params) = update.action_params {
            self.action_params = params;
        }
        if let Some(description) = update.description {
            self.description = Some(description);
        }
    }
}

/// Calculate next run time based on schedule
///
/// TigerStyle: Deterministic calculation with explicit error handling.
fn calculate_next_run(
    schedule_type: &ScheduleType,
    schedule: &str,
    from: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    match schedule_type {
        ScheduleType::Interval => {
            // Parse interval in seconds
            if let Ok(seconds) = schedule.parse::<i64>() {
                Some(from + chrono::Duration::seconds(seconds))
            } else {
                None
            }
        }
        ScheduleType::Once => {
            // Parse ISO timestamp
            DateTime::parse_from_rfc3339(schedule)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        }
        ScheduleType::Cron => {
            // Parse cron expression using croner (builder pattern)
            match Cron::new(schedule).parse() {
                Ok(cron) => cron.find_next_occurrence(&from, false).ok(),
                Err(_) => None,
            }
        }
    }
}

// =========================================================================
// Project models (Phase 6)
// =========================================================================

/// Request to create a project
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    /// Project name
    pub name: String,
    /// Optional description
    pub description: Option<String>,
    /// Optional tags
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional metadata
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Request to update a project
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateProjectRequest {
    /// New name
    pub name: Option<String>,
    /// New description
    pub description: Option<String>,
    /// New tags
    pub tags: Option<Vec<String>>,
    /// New metadata
    pub metadata: Option<serde_json::Value>,
}

/// Project response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Unique identifier
    pub id: String,
    /// Project name
    pub name: String,
    /// Project description
    pub description: Option<String>,
    /// Tags
    pub tags: Vec<String>,
    /// Metadata
    pub metadata: serde_json::Value,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

impl Project {
    /// Create a new project from request
    pub fn from_request(request: CreateProjectRequest) -> Self {
        let now = Utc::now();

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: request.name,
            description: request.description,
            tags: request.tags,
            metadata: request.metadata,
            created_at: now,
            updated_at: now,
        }
    }

    /// Apply an update to the project
    pub fn apply_update(&mut self, update: UpdateProjectRequest) {
        if let Some(name) = update.name {
            self.name = name;
        }
        if let Some(description) = update.description {
            self.description = Some(description);
        }
        if let Some(tags) = update.tags {
            self.tags = tags;
        }
        if let Some(metadata) = update.metadata {
            self.metadata = metadata;
        }
        self.updated_at = Utc::now();
    }
}

// =========================================================================
// Agent Group models (Phase 8)
// =========================================================================

/// Routing policy for agent groups (Letta ManagerType compatibility)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RoutingPolicy {
    #[default]
    RoundRobin,
    Broadcast,
    Intelligent,
    /// Supervisor-based routing (Letta compatibility)
    Supervisor,
    /// Dynamic routing (Letta compatibility)
    Dynamic,
    /// Sleeptime-based routing (Letta compatibility)
    Sleeptime,
    /// Voice sleeptime routing (Letta compatibility)
    VoiceSleeptime,
    /// Swarm routing (Letta compatibility)
    Swarm,
}

/// Request to create an agent group
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAgentGroupRequest {
    /// Optional name (auto-generated if not provided, for Letta compatibility)
    #[serde(default)]
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub agent_ids: Vec<String>,
    /// Routing policy (accepted as "manager_type" in JSON for Letta compatibility)
    #[serde(default, alias = "manager_type")]
    pub routing_policy: RoutingPolicy,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Request to update an agent group
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateAgentGroupRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    /// Routing policy (accepted as "manager_type" in JSON for Letta compatibility)
    #[serde(alias = "manager_type")]
    pub routing_policy: Option<RoutingPolicy>,
    #[serde(default)]
    pub add_agent_ids: Vec<String>,
    #[serde(default)]
    pub remove_agent_ids: Vec<String>,
    pub metadata: Option<serde_json::Value>,
}

// ============================================================================
// Identities
// ============================================================================

/// Identity type
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum IdentityType {
    #[default]
    User,
    Org,
    Other,
}

/// Request to create an identity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateIdentityRequest {
    pub name: String,
    #[serde(default)]
    pub identifier_key: Option<String>,
    #[serde(default)]
    pub identity_type: IdentityType,
    #[serde(default)]
    pub agent_ids: Vec<String>,
    #[serde(default)]
    pub block_ids: Vec<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub properties: serde_json::Value,
}

/// Request to update an identity
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateIdentityRequest {
    pub name: Option<String>,
    pub identifier_key: Option<String>,
    pub identity_type: Option<IdentityType>,
    #[serde(default)]
    pub add_agent_ids: Vec<String>,
    #[serde(default)]
    pub remove_agent_ids: Vec<String>,
    #[serde(default)]
    pub add_block_ids: Vec<String>,
    #[serde(default)]
    pub remove_block_ids: Vec<String>,
    pub project_id: Option<String>,
    pub properties: Option<serde_json::Value>,
}

/// Identity response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub id: String,
    pub name: String,
    pub identifier_key: String,
    pub identity_type: IdentityType,
    pub agent_ids: Vec<String>,
    pub block_ids: Vec<String>,
    pub project_id: Option<String>,
    pub properties: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Identity {
    pub fn from_request(request: CreateIdentityRequest) -> Self {
        let now = Utc::now();
        let id = uuid::Uuid::new_v4().to_string();
        let identifier_key = request
            .identifier_key
            .unwrap_or_else(|| format!("identity-{}", &id[..8]));

        Self {
            id,
            name: request.name,
            identifier_key,
            identity_type: request.identity_type,
            agent_ids: request.agent_ids,
            block_ids: request.block_ids,
            project_id: request.project_id,
            properties: request.properties,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn apply_update(&mut self, request: UpdateIdentityRequest) {
        if let Some(name) = request.name {
            self.name = name;
        }
        if let Some(identifier_key) = request.identifier_key {
            self.identifier_key = identifier_key;
        }
        if let Some(identity_type) = request.identity_type {
            self.identity_type = identity_type;
        }
        if let Some(project_id) = request.project_id {
            self.project_id = Some(project_id);
        }
        if let Some(properties) = request.properties {
            self.properties = properties;
        }

        // Add agent IDs
        for agent_id in request.add_agent_ids {
            if !self.agent_ids.contains(&agent_id) {
                self.agent_ids.push(agent_id);
            }
        }

        // Remove agent IDs
        self.agent_ids
            .retain(|id| !request.remove_agent_ids.contains(id));

        // Add block IDs
        for block_id in request.add_block_ids {
            if !self.block_ids.contains(&block_id) {
                self.block_ids.push(block_id);
            }
        }

        // Remove block IDs
        self.block_ids
            .retain(|id| !request.remove_block_ids.contains(id));

        self.updated_at = Utc::now();
    }
}

/// Agent group response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentGroup {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub agent_ids: Vec<String>,
    /// Routing policy (serialized as "manager_type" for Letta compatibility)
    #[serde(rename = "manager_type")]
    pub routing_policy: RoutingPolicy,
    pub shared_state: serde_json::Value,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip)]
    pub last_routed_index: usize,
}

impl AgentGroup {
    pub fn from_request(request: CreateAgentGroupRequest) -> Self {
        let now = Utc::now();
        let id = uuid::Uuid::new_v4().to_string();
        // Auto-generate name if not provided (Letta compatibility)
        let name = request
            .name
            .unwrap_or_else(|| format!("group-{}", &id[..8]));

        Self {
            id,
            name,
            description: request.description,
            agent_ids: request.agent_ids,
            routing_policy: request.routing_policy,
            shared_state: serde_json::json!([]),
            metadata: request.metadata,
            created_at: now,
            updated_at: now,
            last_routed_index: 0,
        }
    }

    pub fn apply_update(&mut self, update: UpdateAgentGroupRequest) {
        if let Some(name) = update.name {
            self.name = name;
        }
        if let Some(description) = update.description {
            self.description = Some(description);
        }
        if let Some(routing_policy) = update.routing_policy {
            self.routing_policy = routing_policy;
        }
        for agent_id in update.add_agent_ids {
            if !self.agent_ids.contains(&agent_id) {
                self.agent_ids.push(agent_id);
            }
        }
        if !update.remove_agent_ids.is_empty() {
            self.agent_ids
                .retain(|id| !update.remove_agent_ids.contains(id));
        }
        if let Some(metadata) = update.metadata {
            self.metadata = metadata;
        }
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_agent_state() {
        let request = CreateAgentRequest {
            name: "test-agent".to_string(),
            agent_type: AgentType::LettaV1Agent,
            model: Some("openai/gpt-4o".to_string()),
            embedding: None,
            system: Some("You are a helpful assistant".to_string()),
            description: Some("A test agent".to_string()),
            project_id: None,
            user_id: None,
            org_id: None,
            memory_blocks: vec![CreateBlockRequest {
                label: "persona".to_string(),
                value: "I am a helpful AI.".to_string(),
                description: Some("Agent persona".to_string()),
                limit: Some(1000),
            }],
            block_ids: vec![],
            tool_ids: vec![],
            tags: vec!["test".to_string()],
            metadata: serde_json::json!({}),
        };

        let state = AgentState::from_request(request);
        assert_eq!(state.name, "test-agent");
        assert_eq!(state.blocks.len(), 1);
        assert_eq!(state.blocks[0].label, "persona");
    }

    #[test]
    fn test_update_agent() {
        let request = CreateAgentRequest {
            name: "test-agent".to_string(),
            agent_type: AgentType::default(),
            model: None,
            embedding: None,
            system: None,
            description: None,
            project_id: None,
            user_id: None,
            org_id: None,
            memory_blocks: vec![],
            block_ids: vec![],
            tool_ids: vec![],
            tags: vec![],
            metadata: serde_json::json!({}),
        };

        let mut state = AgentState::from_request(request);
        let original_updated_at = state.updated_at;

        // Small delay to ensure timestamp changes
        std::thread::sleep(std::time::Duration::from_millis(10));

        state.apply_update(UpdateAgentRequest {
            name: Some("updated-agent".to_string()),
            description: Some("Updated description".to_string()),
            ..Default::default()
        });

        assert_eq!(state.name, "updated-agent");
        assert_eq!(state.description, Some("Updated description".to_string()));
        assert!(state.updated_at > original_updated_at);
    }

    #[test]
    fn test_error_response() {
        let err = ErrorResponse::not_found("Agent", "abc123");
        assert_eq!(err.code, "not_found");
        assert!(err.message.contains("abc123"));
    }

    #[test]
    fn test_calculate_next_run_interval() {
        let from = Utc::now();
        let next = calculate_next_run(&ScheduleType::Interval, "3600", from);
        assert!(next.is_some());
        let next_time = next.unwrap();
        // Should be approximately 3600 seconds in the future
        let diff = (next_time - from).num_seconds();
        assert_eq!(diff, 3600);
    }

    #[test]
    fn test_calculate_next_run_interval_invalid() {
        let from = Utc::now();
        let next = calculate_next_run(&ScheduleType::Interval, "not_a_number", from);
        assert!(next.is_none());
    }

    #[test]
    fn test_calculate_next_run_cron() {
        let from = Utc::now();
        // Every minute
        let next = calculate_next_run(&ScheduleType::Cron, "* * * * *", from);
        assert!(next.is_some());
        let next_time = next.unwrap();
        // Should be within 60 seconds (next minute)
        let diff = (next_time - from).num_seconds();
        assert!(
            (0..=60).contains(&diff),
            "Expected 0-60 seconds, got {}",
            diff
        );
    }

    #[test]
    fn test_calculate_next_run_cron_hourly() {
        let from = Utc::now();
        // At minute 0 of every hour
        let next = calculate_next_run(&ScheduleType::Cron, "0 * * * *", from);
        assert!(next.is_some());
        let next_time = next.unwrap();
        // Should be within 60 minutes
        let diff = (next_time - from).num_seconds();
        assert!(
            (0..=3600).contains(&diff),
            "Expected 0-3600 seconds, got {}",
            diff
        );
    }

    #[test]
    fn test_calculate_next_run_cron_invalid() {
        let from = Utc::now();
        let next = calculate_next_run(&ScheduleType::Cron, "invalid cron", from);
        assert!(next.is_none());
    }

    #[test]
    fn test_calculate_next_run_once() {
        let from = Utc::now();
        let future_time = from + chrono::Duration::hours(1);
        let schedule = future_time.to_rfc3339();
        let next = calculate_next_run(&ScheduleType::Once, &schedule, from);
        assert!(next.is_some());
        // The returned time should match the schedule
        let next_time = next.unwrap();
        let diff = (next_time - future_time).num_seconds().abs();
        assert!(diff < 2, "Times should match within 2 seconds");
    }

    #[test]
    fn test_calculate_next_run_once_invalid() {
        let from = Utc::now();
        let next = calculate_next_run(&ScheduleType::Once, "not-a-date", from);
        assert!(next.is_none());
    }

    #[test]
    fn test_agent_type_letta_aliases() {
        // Test that "memgpt" deserializes to AgentType::MemgptAgent
        let json = r#"{"agent_type": "memgpt"}"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let agent_type: AgentType = serde_json::from_value(parsed["agent_type"].clone()).unwrap();
        assert_eq!(agent_type, AgentType::MemgptAgent);

        // Test that "letta_v1" deserializes to AgentType::LettaV1Agent
        let json = r#"{"agent_type": "letta_v1"}"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let agent_type: AgentType = serde_json::from_value(parsed["agent_type"].clone()).unwrap();
        assert_eq!(agent_type, AgentType::LettaV1Agent);

        // Test that snake_case names still work (backward compatibility)
        let json = r#"{"agent_type": "memgpt_agent"}"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let agent_type: AgentType = serde_json::from_value(parsed["agent_type"].clone()).unwrap();
        assert_eq!(agent_type, AgentType::MemgptAgent);

        let json = r#"{"agent_type": "letta_v1_agent"}"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let agent_type: AgentType = serde_json::from_value(parsed["agent_type"].clone()).unwrap();
        assert_eq!(agent_type, AgentType::LettaV1Agent);
    }

    #[test]
    fn test_create_agent_request_with_letta_alias() {
        // Test that CreateAgentRequest accepts "memgpt" alias
        let json = r#"{"name": "test", "agent_type": "memgpt"}"#;
        let request: CreateAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.agent_type, AgentType::MemgptAgent);
        assert_eq!(request.name, "test");
    }

    #[test]
    fn test_job_from_request_with_cron() {
        let request = CreateJobRequest {
            agent_id: "agent-1".to_string(),
            schedule_type: ScheduleType::Cron,
            schedule: "0 0 * * *".to_string(), // Daily at midnight
            action: JobAction::SummarizeConversation,
            action_params: serde_json::json!({}),
            description: Some("Daily summary".to_string()),
        };

        let job = Job::from_request(request);
        assert_eq!(job.schedule_type, ScheduleType::Cron);
        assert!(
            job.next_run.is_some(),
            "Cron job should have next_run calculated"
        );
    }
}

// =============================================================================
// MCP Server Models
// =============================================================================

/// MCP Server configuration types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mcp_server_type")]
pub enum MCPServerConfig {
    #[serde(rename = "stdio")]
    Stdio {
        command: String,
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        env: Option<serde_json::Map<String, serde_json::Value>>,
    },
    #[serde(rename = "sse")]
    Sse {
        server_url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        auth_header: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        auth_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom_headers: Option<serde_json::Map<String, serde_json::Value>>,
    },
    #[serde(rename = "streamable_http")]
    StreamableHttp {
        server_url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        auth_header: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        auth_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom_headers: Option<serde_json::Map<String, serde_json::Value>>,
    },
}

/// MCP Server state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MCPServer {
    /// Unique identifier
    pub id: String,
    /// Server name (user-assigned)
    pub server_name: String,
    /// Server configuration (type-specific)
    #[serde(flatten)]
    pub config: MCPServerConfig,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

impl MCPServer {
    /// Create a new MCP server
    pub fn new(server_name: impl Into<String>, config: MCPServerConfig) -> Self {
        let now = Utc::now();
        Self {
            id: format!("mcp_server-{}", Uuid::new_v4()),
            server_name: server_name.into(),
            config,
            created_at: now,
            updated_at: now,
        }
    }
}
