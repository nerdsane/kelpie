//! Umi Memory Backend - Agent-Scoped Memory Operations
//!
//! TigerStyle: DST-first implementation with explicit error handling.
//!
//! Wraps Umi's Memory to provide agent-scoped operations for:
//! - Core memory (always in LLM context)
//! - Archival memory (semantic search)
//! - Conversation history (searchable messages)

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use umi_memory::dst::SimEnvironment;
use umi_memory::{Entity, Memory, RecallOptions, RememberOptions};

// TigerStyle: Explicit constants with units (configurable via env vars)
/// Maximum core memory block size in bytes (configurable via KELPIE_core_memory_block_size_bytes_max())
pub fn core_memory_block_size_bytes_max() -> usize {
    use std::sync::OnceLock;
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        std::env::var("KELPIE_core_memory_block_size_bytes_max()")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8 * 1024) // 8KB default
    })
}

/// Maximum number of archival search results
pub const ARCHIVAL_SEARCH_RESULTS_MAX: usize = 100;

/// Maximum conversation search results
pub const CONVERSATION_SEARCH_RESULTS_MAX: usize = 50;

/// A memory block for core memory (always in LLM context).
#[derive(Debug, Clone)]
pub struct CoreBlock {
    /// Block label (persona, human, facts, goals, scratch)
    pub label: String,
    /// Block content
    pub value: String,
    /// Size in bytes
    pub size_bytes: usize,
}

impl CoreBlock {
    /// Create a new core block.
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        let value = value.into();
        let size_bytes = value.len();
        Self {
            label: label.into(),
            value,
            size_bytes,
        }
    }
}

/// Agent-scoped memory backend using Umi.
///
/// Provides isolated memory operations for each agent:
/// - Core memory blocks (persona, human, facts, goals, scratch)
/// - Archival memory with semantic search
/// - Conversation history
///
/// TigerStyle: Interior mutability via RwLock for thread-safe access.
pub struct UmiMemoryBackend {
    /// Agent identifier for scoping
    agent_id: String,

    /// Umi memory instance (uses trait objects now)
    memory: Arc<RwLock<Memory>>,

    /// Core memory blocks (in-memory cache, synced to Umi)
    core_blocks: Arc<RwLock<HashMap<String, CoreBlock>>>,
}

impl UmiMemoryBackend {
    /// Create a new memory backend for simulation/testing.
    ///
    /// Uses Umi's simulation providers (deterministic, seeded).
    ///
    /// # Arguments
    /// * `seed` - Random seed for deterministic behavior
    pub async fn new_sim(seed: u64) -> Result<Self> {
        Self::new_sim_with_agent(seed, "default".to_string()).await
    }

    /// Create a new memory backend for simulation with explicit agent ID.
    ///
    /// NOTE: This constructor does NOT support fault injection. Use `from_sim_env`
    /// for DST tests that need fault injection.
    ///
    /// # Arguments
    /// * `seed` - Random seed for deterministic behavior
    /// * `agent_id` - Agent identifier for scoping
    pub async fn new_sim_with_agent(seed: u64, agent_id: String) -> Result<Self> {
        // TigerStyle: Preconditions
        assert!(!agent_id.is_empty(), "agent_id cannot be empty");

        let memory = Memory::sim(seed);

        Ok(Self {
            agent_id,
            memory: Arc::new(RwLock::new(memory)),
            core_blocks: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Create a new memory backend from DST simulation environment.
    ///
    /// This constructor connects to the Simulation's fault injector, enabling
    /// proper fault injection testing of memory operations.
    ///
    /// # Arguments
    /// * `env` - DST simulation environment with fault injection
    /// * `agent_id` - Agent identifier for scoping
    ///
    /// # Example
    /// ```ignore
    /// Simulation::new(config)
    ///     .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.1))
    ///     .run(|env| async move {
    ///         let backend = UmiMemoryBackend::from_sim_env(&env, "agent_001").await?;
    ///         backend.append_core("persona", "test").await?; // Faults applied!
    ///         Ok(())
    ///     })
    /// ```
    pub async fn from_sim_env(env: &SimEnvironment, agent_id: impl Into<String>) -> Result<Self> {
        let agent_id = agent_id.into();

        // TigerStyle: Preconditions
        assert!(!agent_id.is_empty(), "agent_id cannot be empty");

        // Create Memory with providers connected to fault injector
        let memory = env.create_memory();

        Ok(Self {
            agent_id,
            memory: Arc::new(RwLock::new(memory)),
            core_blocks: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    // =========================================================================
    // Core Memory Operations
    // =========================================================================

    /// Get all core memory blocks.
    ///
    /// Returns blocks in render order (system, persona, human, facts, goals, scratch).
    pub async fn get_core_blocks(&self) -> Result<Vec<CoreBlock>> {
        let blocks = self.core_blocks.read().await;

        // TigerStyle: Return in deterministic order
        let order = ["system", "persona", "human", "facts", "goals", "scratch"];
        let mut result = Vec::new();

        for label in order {
            if let Some(block) = blocks.get(label) {
                result.push(block.clone());
            }
        }

        // TigerStyle: Postcondition - blocks are ordered
        debug_assert!(result
            .windows(2)
            .all(|w| order.iter().position(|&l| l == w[0].label)
                <= order.iter().position(|&l| l == w[1].label)));

        Ok(result)
    }

    /// Append content to a core memory block.
    ///
    /// Creates the block if it doesn't exist.
    ///
    /// # Arguments
    /// * `label` - Block label (persona, human, facts, goals, scratch)
    /// * `content` - Content to append
    pub async fn append_core(&self, label: &str, content: &str) -> Result<()> {
        // TigerStyle: Preconditions
        assert!(!label.is_empty(), "label cannot be empty");
        assert!(!content.is_empty(), "content cannot be empty");

        let mut blocks = self.core_blocks.write().await;

        if let Some(block) = blocks.get_mut(label) {
            // Append to existing block
            let new_value = format!("{}\n{}", block.value, content);

            // TigerStyle: Check size limit
            if new_value.len() > core_memory_block_size_bytes_max() {
                return Err(anyhow!(
                    "core memory block '{}' would exceed max size ({} > {})",
                    label,
                    new_value.len(),
                    core_memory_block_size_bytes_max()
                ));
            }

            block.value = new_value;
            block.size_bytes = block.value.len();
        } else {
            // Create new block
            if content.len() > core_memory_block_size_bytes_max() {
                return Err(anyhow!(
                    "content exceeds max block size ({} > {})",
                    content.len(),
                    core_memory_block_size_bytes_max()
                ));
            }

            blocks.insert(label.to_string(), CoreBlock::new(label, content));
        }

        // Also store in Umi for persistence
        self.sync_core_to_umi(label, &blocks.get(label).unwrap().value)
            .await?;

        Ok(())
    }

    /// Replace content in a core memory block.
    ///
    /// # Arguments
    /// * `label` - Block label
    /// * `old_content` - Content to find and replace
    /// * `new_content` - Replacement content
    pub async fn replace_core(
        &self,
        label: &str,
        old_content: &str,
        new_content: &str,
    ) -> Result<()> {
        // TigerStyle: Preconditions
        assert!(!label.is_empty(), "label cannot be empty");
        assert!(!old_content.is_empty(), "old_content cannot be empty");

        let mut blocks = self.core_blocks.write().await;

        let block = blocks
            .get_mut(label)
            .ok_or_else(|| anyhow!("core memory block '{}' not found", label))?;

        if !block.value.contains(old_content) {
            return Err(anyhow!(
                "content '{}' not found in block '{}'",
                old_content,
                label
            ));
        }

        let new_value = block.value.replace(old_content, new_content);

        // TigerStyle: Check size limit
        if new_value.len() > core_memory_block_size_bytes_max() {
            return Err(anyhow!(
                "replacement would exceed max block size ({} > {})",
                new_value.len(),
                core_memory_block_size_bytes_max()
            ));
        }

        block.value = new_value;
        block.size_bytes = block.value.len();

        // Sync to Umi
        self.sync_core_to_umi(label, &block.value).await?;

        Ok(())
    }

    /// Sync a core memory block to Umi storage.
    async fn sync_core_to_umi(&self, label: &str, content: &str) -> Result<()> {
        let mut memory = self.memory.write().await;

        // Store as an entity with agent_id prefix and core_memory tag
        let text = format!(
            "[agent:{}][core_memory:{}] {}",
            self.agent_id, label, content
        );

        memory
            .remember(&text, RememberOptions::default())
            .await
            .map_err(|e| anyhow!("failed to sync core memory to Umi: {}", e))?;

        Ok(())
    }

    // =========================================================================
    // Archival Memory Operations
    // =========================================================================

    /// Insert content into archival memory.
    ///
    /// Content is stored with embeddings for semantic search.
    ///
    /// # Arguments
    /// * `content` - Content to store
    ///
    /// # Returns
    /// Entity ID of the stored content
    pub async fn insert_archival(&self, content: &str) -> Result<String> {
        // TigerStyle: Preconditions
        assert!(!content.is_empty(), "content cannot be empty");

        let mut memory = self.memory.write().await;

        // Tag with agent_id and archival marker
        let text = format!("[agent:{}][archival] {}", self.agent_id, content);

        let result = memory
            .remember(&text, RememberOptions::default())
            .await
            .map_err(|e| anyhow!("failed to insert archival memory: {}", e))?;

        // Return first entity ID (or generate one if none)
        let entity_id = result
            .entities
            .first()
            .map(|e| e.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        Ok(entity_id)
    }

    /// Search archival memory semantically.
    ///
    /// # Arguments
    /// * `query` - Search query
    /// * `limit` - Maximum number of results
    ///
    /// # Returns
    /// List of matching entities belonging to this agent only
    pub async fn search_archival(&self, query: &str, limit: usize) -> Result<Vec<Entity>> {
        // TigerStyle: Preconditions
        assert!(!query.is_empty(), "query cannot be empty");
        assert!(limit > 0, "limit must be positive");
        assert!(
            limit <= ARCHIVAL_SEARCH_RESULTS_MAX,
            "limit {} exceeds max {}",
            limit,
            ARCHIVAL_SEARCH_RESULTS_MAX
        );

        let memory = self.memory.read().await;

        // Search with agent scoping in query (helps with semantic similarity)
        let scoped_query = format!("[agent:{}][archival] {}", self.agent_id, query);

        // Request more results than needed because we'll filter by agent
        // TigerStyle: Over-fetch to account for filtering
        let fetch_limit = (limit * 3).min(ARCHIVAL_SEARCH_RESULTS_MAX);

        let raw_results = memory
            .recall(
                &scoped_query,
                RecallOptions::default().with_limit(fetch_limit)?,
            )
            .await
            .map_err(|e| anyhow!("archival search failed: {}", e))?;

        // BUG-002 FIX: Filter results to only include this agent's data
        // Semantic search returns ALL similar content, so we must enforce isolation
        let agent_prefix = format!("[agent:{}][archival]", self.agent_id);
        let filtered: Vec<Entity> = raw_results
            .into_iter()
            .filter(|entity| entity.content.contains(&agent_prefix))
            .take(limit)
            .collect();

        // TigerStyle: Postcondition - results within limit and all belong to this agent
        debug_assert!(filtered.len() <= limit);
        debug_assert!(filtered.iter().all(|e| e.content.contains(&agent_prefix)));

        Ok(filtered)
    }

    // =========================================================================
    // Conversation History Operations
    // =========================================================================

    /// Store a conversation message.
    ///
    /// # Arguments
    /// * `role` - Message role (user, assistant, system)
    /// * `content` - Message content
    pub async fn store_message(&self, role: &str, content: &str) -> Result<()> {
        // TigerStyle: Preconditions
        assert!(!role.is_empty(), "role cannot be empty");
        assert!(!content.is_empty(), "content cannot be empty");
        assert!(
            role == "user" || role == "assistant" || role == "system",
            "role must be user, assistant, or system"
        );

        let mut memory = self.memory.write().await;

        // Tag with agent_id, conversation marker, and role
        let text = format!(
            "[agent:{}][conversation][role:{}] {}",
            self.agent_id, role, content
        );

        memory
            .remember(&text, RememberOptions::default())
            .await
            .map_err(|e| anyhow!("failed to store message: {}", e))?;

        Ok(())
    }

    /// Search conversation history.
    ///
    /// # Arguments
    /// * `query` - Search query
    /// * `limit` - Maximum number of results
    ///
    /// # Returns
    /// List of matching entities (conversation messages) belonging to this agent only
    pub async fn search_conversations(&self, query: &str, limit: usize) -> Result<Vec<Entity>> {
        // TigerStyle: Preconditions
        assert!(!query.is_empty(), "query cannot be empty");
        assert!(limit > 0, "limit must be positive");
        assert!(
            limit <= CONVERSATION_SEARCH_RESULTS_MAX,
            "limit {} exceeds max {}",
            limit,
            CONVERSATION_SEARCH_RESULTS_MAX
        );

        let memory = self.memory.read().await;

        // Search with agent and conversation scoping
        let scoped_query = format!("[agent:{}][conversation] {}", self.agent_id, query);

        // TigerStyle: Over-fetch to account for filtering
        let fetch_limit = (limit * 3).min(CONVERSATION_SEARCH_RESULTS_MAX);

        let raw_results = memory
            .recall(
                &scoped_query,
                RecallOptions::default().with_limit(fetch_limit)?,
            )
            .await
            .map_err(|e| anyhow!("conversation search failed: {}", e))?;

        // BUG-002 FIX: Filter results to only include this agent's conversations
        let agent_prefix = format!("[agent:{}][conversation]", self.agent_id);
        let filtered: Vec<Entity> = raw_results
            .into_iter()
            .filter(|entity| entity.content.contains(&agent_prefix))
            .take(limit)
            .collect();

        // TigerStyle: Postcondition - results within limit and all belong to this agent
        debug_assert!(filtered.len() <= limit);
        debug_assert!(filtered.iter().all(|e| e.content.contains(&agent_prefix)));

        Ok(filtered)
    }

    // =========================================================================
    // Utility Methods
    // =========================================================================

    /// Get the agent ID this backend is scoped to.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Build system prompt from core memory blocks.
    ///
    /// Formats blocks as XML for LLM context.
    pub async fn build_system_prompt(&self) -> Result<String> {
        let blocks = self.get_core_blocks().await?;

        if blocks.is_empty() {
            return Ok(String::new());
        }

        let mut parts = Vec::new();
        for block in blocks {
            parts.push(format!(
                "<{}>\n{}\n</{}>",
                block.label, block.value, block.label
            ));
        }

        Ok(parts.join("\n\n"))
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_new_sim_creates_backend() {
        let backend = UmiMemoryBackend::new_sim(42).await.unwrap();
        assert_eq!(backend.agent_id(), "default");
    }

    #[tokio::test]
    async fn test_new_sim_with_agent() {
        let backend = UmiMemoryBackend::new_sim_with_agent(42, "agent_001".to_string())
            .await
            .unwrap();
        assert_eq!(backend.agent_id(), "agent_001");
    }

    #[tokio::test]
    async fn test_core_memory_append() {
        let backend = UmiMemoryBackend::new_sim(42).await.unwrap();

        backend
            .append_core("persona", "I am a helpful assistant")
            .await
            .unwrap();

        let blocks = backend.get_core_blocks().await.unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].label, "persona");
        assert!(blocks[0].value.contains("helpful assistant"));
    }

    #[tokio::test]
    async fn test_core_memory_append_creates_and_appends() {
        let backend = UmiMemoryBackend::new_sim(42).await.unwrap();

        backend.append_core("facts", "Fact 1").await.unwrap();
        backend.append_core("facts", "Fact 2").await.unwrap();

        let blocks = backend.get_core_blocks().await.unwrap();
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].value.contains("Fact 1"));
        assert!(blocks[0].value.contains("Fact 2"));
    }

    #[tokio::test]
    async fn test_core_memory_replace() {
        let backend = UmiMemoryBackend::new_sim(42).await.unwrap();

        backend
            .append_core("persona", "I am a helpful assistant")
            .await
            .unwrap();
        backend
            .replace_core("persona", "helpful", "friendly")
            .await
            .unwrap();

        let blocks = backend.get_core_blocks().await.unwrap();
        assert!(blocks[0].value.contains("friendly"));
        assert!(!blocks[0].value.contains("helpful"));
    }

    #[tokio::test]
    async fn test_core_memory_order() {
        let backend = UmiMemoryBackend::new_sim(42).await.unwrap();

        // Add in random order
        backend.append_core("scratch", "notes").await.unwrap();
        backend.append_core("persona", "personality").await.unwrap();
        backend.append_core("facts", "knowledge").await.unwrap();

        let blocks = backend.get_core_blocks().await.unwrap();

        // Should be in render order
        assert_eq!(blocks[0].label, "persona");
        assert_eq!(blocks[1].label, "facts");
        assert_eq!(blocks[2].label, "scratch");
    }

    #[tokio::test]
    async fn test_build_system_prompt() {
        let backend = UmiMemoryBackend::new_sim(42).await.unwrap();

        backend
            .append_core("persona", "I am helpful")
            .await
            .unwrap();
        backend
            .append_core("human", "User likes cats")
            .await
            .unwrap();

        let prompt = backend.build_system_prompt().await.unwrap();

        assert!(prompt.contains("<persona>"));
        assert!(prompt.contains("I am helpful"));
        assert!(prompt.contains("</persona>"));
        assert!(prompt.contains("<human>"));
        assert!(prompt.contains("User likes cats"));
    }

    #[tokio::test]
    #[should_panic(expected = "agent_id cannot be empty")]
    async fn test_empty_agent_id_panics() {
        let _ = UmiMemoryBackend::new_sim_with_agent(42, "".to_string()).await;
    }

    #[tokio::test]
    #[should_panic(expected = "label cannot be empty")]
    async fn test_empty_label_panics() {
        let backend = UmiMemoryBackend::new_sim(42).await.unwrap();
        let _ = backend.append_core("", "content").await;
    }
}
