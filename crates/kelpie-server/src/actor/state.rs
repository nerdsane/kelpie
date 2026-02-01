//! AgentActor state definition
//!
//! TigerStyle: Explicit state structure, serializable, with documented fields.

use crate::models::{AgentState, ArchivalEntry, Block, Message};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum messages to keep in memory (Phase 6.7)
const MAX_MESSAGES_DEFAULT: usize = 100;

/// Maximum archival entries per agent
const MAX_ARCHIVAL_ENTRIES_DEFAULT: usize = 100_000;

/// State for AgentActor
///
/// This is the in-memory state of an agent actor. It's loaded on activation
/// and persisted on deactivation.
///
/// TigerStyle: All fields have clear purpose and units where applicable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentActorState {
    /// Agent metadata (id, name, type, model, system prompt, etc.)
    /// This is the source of truth for agent configuration.
    pub agent: Option<AgentState>,

    /// Message history (Phase 6.7)
    ///
    /// Recent messages kept in memory for LLM context.
    /// Truncated to max_messages to prevent memory bloat.
    #[serde(default)]
    pub messages: Vec<Message>,

    /// Maximum messages to keep in memory (Phase 6.7)
    #[serde(default = "default_max_messages")]
    pub max_messages: usize,

    /// Archival memory entries (long-term storage)
    ///
    /// Persistent memory that survives restarts.
    /// Used for knowledge the agent needs to remember long-term.
    #[serde(default)]
    pub archival: Vec<ArchivalEntry>,

    /// Current session ID (for checkpoint/resume)
    pub session_id: Option<String>,

    /// Current iteration count in agent loop
    pub iteration: u32,

    /// Whether the agent is paused (waiting for user input)
    pub is_paused: bool,

    /// Unix timestamp in milliseconds when pause expires
    pub pause_until_ms: Option<u64>,
}

fn default_max_messages() -> usize {
    MAX_MESSAGES_DEFAULT
}

impl Default for AgentActorState {
    fn default() -> Self {
        Self {
            agent: None,
            messages: Vec::new(),
            max_messages: MAX_MESSAGES_DEFAULT,
            archival: Vec::new(),
            session_id: None,
            iteration: 0,
            is_paused: false,
            pause_until_ms: None,
        }
    }
}

impl AgentActorState {
    /// Create a new AgentActorState from AgentState
    pub fn from_agent(agent: AgentState) -> Self {
        Self {
            agent: Some(agent),
            messages: Vec::new(),
            max_messages: MAX_MESSAGES_DEFAULT,
            archival: Vec::new(),
            session_id: None,
            iteration: 0,
            is_paused: false,
            pause_until_ms: None,
        }
    }

    /// Get a reference to the agent state
    ///
    /// Returns None if agent hasn't been created yet.
    pub fn agent(&self) -> Option<&AgentState> {
        self.agent.as_ref()
    }

    /// Get a mutable reference to the agent state
    pub fn agent_mut(&mut self) -> Option<&mut AgentState> {
        self.agent.as_mut()
    }

    /// Get a block by label
    pub fn get_block(&self, label: &str) -> Option<&Block> {
        self.agent
            .as_ref()?
            .blocks
            .iter()
            .find(|b| b.label == label)
    }

    /// Update a block by label
    pub fn update_block<F>(&mut self, label: &str, f: F) -> bool
    where
        F: FnOnce(&mut Block),
    {
        if let Some(agent) = &mut self.agent {
            if let Some(block) = agent.blocks.iter_mut().find(|b| b.label == label) {
                f(block);
                return true;
            }
        }
        false
    }

    /// Create a new block with the given label and initial content
    ///
    /// Used when core_memory_append needs to create a block that doesn't exist.
    pub fn create_block(&mut self, label: &str, content: &str) {
        if let Some(agent) = &mut self.agent {
            let now = Utc::now();
            let block = Block {
                id: Uuid::new_v4().to_string(),
                label: label.to_string(),
                value: content.to_string(),
                description: None,
                limit: None,
                created_at: now,
                updated_at: now,
            };
            agent.blocks.push(block);
        }
    }

    /// Add message to history (Phase 6.7)
    ///
    /// Automatically truncates to max_messages to prevent memory bloat.
    ///
    /// # Arguments
    /// * `message` - Message to add to history
    ///
    /// # TigerStyle
    /// - Explicit truncation with assertion
    /// - Clear size limit documented
    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);

        // Truncate if exceeds max
        if self.messages.len() > self.max_messages {
            let start = self.messages.len() - self.max_messages;
            self.messages = self.messages[start..].to_vec();

            // TigerStyle: Assert postcondition
            assert!(
                self.messages.len() <= self.max_messages,
                "messages should be truncated to max_messages"
            );
        }
    }

    /// Get recent messages (Phase 6.7)
    ///
    /// Returns the last N messages, or all messages if fewer than N.
    ///
    /// # Arguments
    /// * `limit` - Maximum number of messages to return
    ///
    /// # Returns
    /// Slice of most recent messages (up to limit)
    pub fn recent_messages(&self, limit: usize) -> &[Message] {
        let start = self.messages.len().saturating_sub(limit);
        &self.messages[start..]
    }

    /// Get all messages in history (Phase 6.7)
    pub fn all_messages(&self) -> &[Message] {
        &self.messages
    }

    /// Clear message history (Phase 6.7)
    pub fn clear_messages(&mut self) {
        self.messages.clear();
    }

    // =========================================================================
    // Archival memory operations
    // =========================================================================

    /// Add an entry to archival memory
    ///
    /// # Arguments
    /// * `content` - The content to store
    /// * `metadata` - Optional metadata for the entry
    ///
    /// # Returns
    /// The created ArchivalEntry with generated ID
    ///
    /// # TigerStyle
    /// - Explicit limit enforcement
    /// - Clear postcondition assertion
    pub fn add_archival_entry(
        &mut self,
        content: String,
        metadata: Option<serde_json::Value>,
    ) -> Result<ArchivalEntry, String> {
        // TigerStyle: Enforce limits
        if self.archival.len() >= MAX_ARCHIVAL_ENTRIES_DEFAULT {
            return Err(format!(
                "Archival entry limit exceeded: {}",
                MAX_ARCHIVAL_ENTRIES_DEFAULT
            ));
        }

        let entry = ArchivalEntry {
            id: Uuid::new_v4().to_string(),
            content,
            metadata,
            created_at: Utc::now().to_rfc3339(),
        };

        let result = entry.clone();
        self.archival.push(entry);

        // TigerStyle: Assert postcondition
        assert!(
            self.archival.len() <= MAX_ARCHIVAL_ENTRIES_DEFAULT,
            "archival should not exceed limit"
        );

        Ok(result)
    }

    /// Search archival memory by text query
    ///
    /// # Arguments
    /// * `query` - Optional text to search for (case-insensitive)
    /// * `limit` - Maximum number of results to return
    ///
    /// # Returns
    /// Matching archival entries
    pub fn search_archival(&self, query: Option<&str>, limit: usize) -> Vec<ArchivalEntry> {
        let results: Vec<_> = if let Some(q) = query {
            let q_lower = q.to_lowercase();
            self.archival
                .iter()
                .filter(|e| e.content.to_lowercase().contains(&q_lower))
                .take(limit)
                .cloned()
                .collect()
        } else {
            self.archival.iter().take(limit).cloned().collect()
        };

        results
    }

    /// Get a specific archival entry by ID
    ///
    /// # Arguments
    /// * `entry_id` - The ID of the entry to retrieve
    ///
    /// # Returns
    /// The entry if found, None otherwise
    pub fn get_archival_entry(&self, entry_id: &str) -> Option<ArchivalEntry> {
        self.archival.iter().find(|e| e.id == entry_id).cloned()
    }

    /// Delete an archival entry by ID
    ///
    /// # Arguments
    /// * `entry_id` - The ID of the entry to delete
    ///
    /// # Returns
    /// Ok(()) if deleted, Err if not found
    pub fn delete_archival_entry(&mut self, entry_id: &str) -> Result<(), String> {
        let initial_len = self.archival.len();
        self.archival.retain(|e| e.id != entry_id);

        if self.archival.len() == initial_len {
            return Err(format!("Archival entry not found: {}", entry_id));
        }

        Ok(())
    }

    // =========================================================================
    // Conversation search operations
    // =========================================================================

    /// Search messages by text query
    ///
    /// # Arguments
    /// * `query` - Text to search for (case-insensitive)
    /// * `limit` - Maximum number of results to return
    ///
    /// # Returns
    /// Matching messages
    pub fn search_messages(&self, query: &str, limit: usize) -> Vec<Message> {
        let query_lower = query.to_lowercase();
        self.messages
            .iter()
            .filter(|m| m.content.to_lowercase().contains(&query_lower))
            .take(limit)
            .cloned()
            .collect()
    }

    /// Search messages by text query with date filter
    ///
    /// # Arguments
    /// * `query` - Text to search for (case-insensitive)
    /// * `start_date` - Optional start date filter (inclusive)
    /// * `end_date` - Optional end date filter (inclusive)
    /// * `limit` - Maximum number of results to return
    ///
    /// # Returns
    /// Matching messages within date range
    pub fn search_messages_with_date(
        &self,
        query: &str,
        start_date: Option<DateTime<Utc>>,
        end_date: Option<DateTime<Utc>>,
        limit: usize,
    ) -> Vec<Message> {
        let query_lower = query.to_lowercase();
        self.messages
            .iter()
            .filter(|m| {
                let matches_query = m.content.to_lowercase().contains(&query_lower);
                let matches_dates = match (start_date, end_date) {
                    (Some(start), Some(end)) => m.created_at >= start && m.created_at <= end,
                    (Some(start), None) => m.created_at >= start,
                    (None, Some(end)) => m.created_at <= end,
                    (None, None) => true,
                };
                matches_query && matches_dates
            })
            .take(limit)
            .cloned()
            .collect()
    }

    /// List messages with pagination
    ///
    /// # Arguments
    /// * `limit` - Maximum number of messages to return
    /// * `before` - Optional message ID to return messages before
    ///
    /// # Returns
    /// Messages (most recent first, up to limit)
    pub fn list_messages_paginated(&self, limit: usize, before: Option<&str>) -> Vec<Message> {
        let end_idx = if let Some(before_id) = before {
            self.messages
                .iter()
                .position(|m| m.id == before_id)
                .unwrap_or(self.messages.len())
        } else {
            self.messages.len()
        };

        let start_idx = end_idx.saturating_sub(limit);
        self.messages[start_idx..end_idx].to_vec()
    }

    /// Replace content in a memory block
    ///
    /// # Arguments
    /// * `label` - Block label
    /// * `old_content` - Content to find
    /// * `new_content` - Replacement content
    ///
    /// # Returns
    /// Ok(()) if replaced, Err if block not found or old_content not found
    pub fn replace_block_content(
        &mut self,
        label: &str,
        old_content: &str,
        new_content: &str,
    ) -> Result<(), String> {
        if let Some(agent) = &mut self.agent {
            if let Some(block) = agent.blocks.iter_mut().find(|b| b.label == label) {
                if !block.value.contains(old_content) {
                    return Err(format!(
                        "Content '{}' not found in block '{}'",
                        old_content, label
                    ));
                }
                block.value = block.value.replace(old_content, new_content);
                block.updated_at = Utc::now();
                return Ok(());
            }
        }
        Err(format!("Block '{}' not found", label))
    }
}
