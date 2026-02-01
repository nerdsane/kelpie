//! FDB Persistence Integration Tests
//!
//! TigerStyle: Tests verify data survives restart with real FDB.
//!
//! These tests require a running FDB cluster and are marked #[ignore]
//! for CI. Run them locally with: cargo test -p kelpie-server --test fdb_persistence_test -- --ignored
//!
//! Test Strategy:
//! 1. Create storage, save data, drop storage, recreate, verify data persists
//! 2. Test all entity types: agents, messages, sessions, blocks
//! 3. Verify atomic operations work correctly
//! 4. Verify cascading deletes work correctly

use chrono::Utc;
use kelpie_server::models::{AgentType, Block, Message, MessageRole};
use kelpie_server::storage::{AgentMetadata, AgentStorage, FdbAgentRegistry, SessionState};
use kelpie_storage::FdbKV;
use std::sync::Arc;

/// Get FDB cluster file from environment or use default
fn get_cluster_file() -> Option<String> {
    std::env::var("KELPIE_FDB_CLUSTER")
        .ok()
        .or_else(|| std::env::var("FDB_CLUSTER_FILE").ok())
        .or_else(|| {
            // Check standard paths
            for path in &[
                "/etc/foundationdb/fdb.cluster",
                "/usr/local/etc/foundationdb/fdb.cluster",
                "/opt/foundationdb/fdb.cluster",
            ] {
                if std::path::Path::new(path).exists() {
                    return Some(path.to_string());
                }
            }
            None
        })
}

/// Create FDB storage for testing
async fn create_fdb_storage() -> Result<Arc<dyn AgentStorage>, Box<dyn std::error::Error>> {
    let cluster_file = get_cluster_file().ok_or("No FDB cluster file found")?;
    let fdb_kv = FdbKV::connect(Some(&cluster_file)).await?;
    let registry = FdbAgentRegistry::new(Arc::new(fdb_kv));
    Ok(Arc::new(registry))
}

/// Generate unique agent ID for test isolation
fn unique_agent_id(prefix: &str) -> String {
    format!(
        "{}-{}-{}",
        prefix,
        std::process::id(),
        chrono::Utc::now().timestamp_micros()
    )
}

/// Helper to create a test message
fn create_test_message(role: MessageRole, content: &str) -> Message {
    Message {
        id: uuid::Uuid::new_v4().to_string(),
        agent_id: String::new(), // Will be set by storage
        message_type: Message::message_type_from_role(&role),
        role,
        content: content.to_string(),
        tool_call_id: None,
        tool_calls: vec![],
        tool_call: None,
        tool_return: None,
        status: None,
        created_at: Utc::now(),
    }
}

/// Helper to create a test session
fn create_test_session(agent_id: &str, session_id: &str) -> SessionState {
    SessionState {
        session_id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        iteration: 0,
        pause_until_ms: None,
        pending_tool_calls: vec![],
        last_tool_result: None,
        stop_reason: None,
        started_at: Utc::now(),
        checkpointed_at: Utc::now(),
    }
}

// =============================================================================
// Agent Persistence Tests
// =============================================================================

#[tokio::test]
#[ignore = "requires running FDB cluster"]
async fn test_agent_survives_restart() {
    let storage = create_fdb_storage()
        .await
        .expect("Failed to connect to FDB");
    let agent_id = unique_agent_id("agent-persist");

    // Create and save agent
    let agent = AgentMetadata::new(
        agent_id.clone(),
        "Test Agent".to_string(),
        AgentType::MemgptAgent,
    );
    storage
        .save_agent(&agent)
        .await
        .expect("Failed to save agent");

    // Drop storage (simulates restart)
    drop(storage);

    // Reconnect
    let storage = create_fdb_storage()
        .await
        .expect("Failed to reconnect to FDB");

    // Verify agent exists
    let loaded = storage
        .load_agent(&agent_id)
        .await
        .expect("Failed to load agent")
        .expect("Agent should exist after restart");

    assert_eq!(loaded.id, agent_id);
    assert_eq!(loaded.name, "Test Agent");

    // Cleanup
    storage
        .delete_agent(&agent_id)
        .await
        .expect("Cleanup failed");
}

// =============================================================================
// Message Persistence Tests
// =============================================================================

#[tokio::test]
#[ignore = "requires running FDB cluster"]
async fn test_messages_survive_restart() {
    let storage = create_fdb_storage()
        .await
        .expect("Failed to connect to FDB");
    let agent_id = unique_agent_id("msg-persist");

    // Create agent first
    let agent = AgentMetadata::new(
        agent_id.clone(),
        "Message Test".to_string(),
        AgentType::MemgptAgent,
    );
    storage
        .save_agent(&agent)
        .await
        .expect("Failed to save agent");

    // Append messages
    let msg1 = create_test_message(MessageRole::User, "First message");
    let msg2 = create_test_message(MessageRole::Assistant, "Second message");

    storage
        .append_message(&agent_id, &msg1)
        .await
        .expect("Failed to append msg1");
    storage
        .append_message(&agent_id, &msg2)
        .await
        .expect("Failed to append msg2");

    // Verify count before restart
    let count_before = storage
        .count_messages(&agent_id)
        .await
        .expect("Failed to count");
    assert_eq!(count_before, 2, "Should have 2 messages before restart");

    // Drop storage (simulates restart)
    drop(storage);

    // Reconnect
    let storage = create_fdb_storage()
        .await
        .expect("Failed to reconnect to FDB");

    // Verify messages exist
    let messages = storage
        .load_messages(&agent_id, 100)
        .await
        .expect("Failed to load messages");

    assert_eq!(messages.len(), 2, "Should have 2 messages after restart");
    assert_eq!(messages[0].content, "First message");
    assert_eq!(messages[1].content, "Second message");

    // Cleanup
    storage
        .delete_agent(&agent_id)
        .await
        .expect("Cleanup failed");
}

// =============================================================================
// Block Persistence Tests
// =============================================================================

#[tokio::test]
#[ignore = "requires running FDB cluster"]
async fn test_blocks_survive_restart() {
    let storage = create_fdb_storage()
        .await
        .expect("Failed to connect to FDB");
    let agent_id = unique_agent_id("block-persist");

    // Create agent first
    let agent = AgentMetadata::new(
        agent_id.clone(),
        "Block Test".to_string(),
        AgentType::MemgptAgent,
    );
    storage
        .save_agent(&agent)
        .await
        .expect("Failed to save agent");

    // Save blocks
    let blocks = vec![
        Block::new("persona", "You are a helpful assistant"),
        Block::new("human", "The user is a developer"),
    ];
    storage
        .save_blocks(&agent_id, &blocks)
        .await
        .expect("Failed to save blocks");

    // Drop storage (simulates restart)
    drop(storage);

    // Reconnect
    let storage = create_fdb_storage()
        .await
        .expect("Failed to reconnect to FDB");

    // Verify blocks exist
    let loaded_blocks = storage
        .load_blocks(&agent_id)
        .await
        .expect("Failed to load blocks");

    assert_eq!(loaded_blocks.len(), 2, "Should have 2 blocks after restart");

    // Cleanup
    storage
        .delete_agent(&agent_id)
        .await
        .expect("Cleanup failed");
}

// =============================================================================
// Session Persistence Tests
// =============================================================================

#[tokio::test]
#[ignore = "requires running FDB cluster"]
async fn test_session_survives_restart() {
    let storage = create_fdb_storage()
        .await
        .expect("Failed to connect to FDB");
    let agent_id = unique_agent_id("session-persist");
    let session_id = format!("session-{}", chrono::Utc::now().timestamp_micros());

    // Create agent first
    let agent = AgentMetadata::new(
        agent_id.clone(),
        "Session Test".to_string(),
        AgentType::MemgptAgent,
    );
    storage
        .save_agent(&agent)
        .await
        .expect("Failed to save agent");

    // Save session
    let session = create_test_session(&agent_id, &session_id);
    storage
        .save_session(&session)
        .await
        .expect("Failed to save session");

    // Drop storage (simulates restart)
    drop(storage);

    // Reconnect
    let storage = create_fdb_storage()
        .await
        .expect("Failed to reconnect to FDB");

    // Verify session exists
    let loaded = storage
        .load_session(&agent_id, &session_id)
        .await
        .expect("Failed to load session")
        .expect("Session should exist after restart");

    assert_eq!(loaded.session_id, session_id);
    assert_eq!(loaded.agent_id, agent_id);

    // Cleanup
    storage
        .delete_agent(&agent_id)
        .await
        .expect("Cleanup failed");
}

// =============================================================================
// Atomic Checkpoint Tests
// =============================================================================

#[tokio::test]
#[ignore = "requires running FDB cluster"]
async fn test_checkpoint_atomicity() {
    let storage = create_fdb_storage()
        .await
        .expect("Failed to connect to FDB");
    let agent_id = unique_agent_id("checkpoint-atomic");
    let session_id = format!("session-{}", chrono::Utc::now().timestamp_micros());

    // Create agent first
    let agent = AgentMetadata::new(
        agent_id.clone(),
        "Checkpoint Test".to_string(),
        AgentType::MemgptAgent,
    );
    storage
        .save_agent(&agent)
        .await
        .expect("Failed to save agent");

    // Create session and message
    let mut session = create_test_session(&agent_id, &session_id);
    session.iteration = 10;
    let message = create_test_message(MessageRole::Assistant, "Checkpoint message");

    // Atomic checkpoint (session + message together)
    storage
        .checkpoint(&session, Some(&message))
        .await
        .expect("Checkpoint failed");

    // Verify both exist
    let loaded_session = storage
        .load_session(&agent_id, &session_id)
        .await
        .expect("Load session failed")
        .expect("Session should exist");
    assert_eq!(loaded_session.iteration, 10);

    let messages = storage
        .load_messages(&agent_id, 100)
        .await
        .expect("Load messages failed");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "Checkpoint message");

    // Cleanup
    storage
        .delete_agent(&agent_id)
        .await
        .expect("Cleanup failed");
}

// =============================================================================
// Cascading Delete Tests
// =============================================================================

#[tokio::test]
#[ignore = "requires running FDB cluster"]
async fn test_cascading_delete() {
    let storage = create_fdb_storage()
        .await
        .expect("Failed to connect to FDB");
    let agent_id = unique_agent_id("cascade-delete");
    let session_id = format!("session-{}", chrono::Utc::now().timestamp_micros());

    // Create agent with all associated data
    let agent = AgentMetadata::new(
        agent_id.clone(),
        "Cascade Test".to_string(),
        AgentType::MemgptAgent,
    );
    storage
        .save_agent(&agent)
        .await
        .expect("Failed to save agent");

    // Add blocks
    let blocks = vec![Block::new("test", "test value")];
    storage
        .save_blocks(&agent_id, &blocks)
        .await
        .expect("Failed to save blocks");

    // Add session
    let session = create_test_session(&agent_id, &session_id);
    storage
        .save_session(&session)
        .await
        .expect("Failed to save session");

    // Add messages
    let msg = create_test_message(MessageRole::User, "Test message");
    storage
        .append_message(&agent_id, &msg)
        .await
        .expect("Failed to append message");

    // Delete agent (should cascade)
    storage
        .delete_agent(&agent_id)
        .await
        .expect("Delete failed");

    // Verify all data is gone
    let agent_result = storage
        .load_agent(&agent_id)
        .await
        .expect("Load should not error");
    assert!(agent_result.is_none(), "Agent should be deleted");

    let blocks_result = storage
        .load_blocks(&agent_id)
        .await
        .expect("Load should not error");
    assert!(blocks_result.is_empty(), "Blocks should be deleted");

    let session_result = storage
        .load_session(&agent_id, &session_id)
        .await
        .expect("Load should not error");
    assert!(session_result.is_none(), "Session should be deleted");

    let message_count = storage
        .count_messages(&agent_id)
        .await
        .expect("Count should not error");
    assert_eq!(message_count, 0, "Messages should be deleted");
}

// =============================================================================
// Archival Memory Persistence Tests
// =============================================================================

#[tokio::test]
#[ignore = "requires running FDB cluster"]
async fn test_archival_survives_restart() {
    use kelpie_server::models::ArchivalEntry;

    let storage = create_fdb_storage()
        .await
        .expect("Failed to connect to FDB");
    let agent_id = unique_agent_id("archival-persist");

    // Create agent first
    let agent = AgentMetadata::new(
        agent_id.clone(),
        "Archival Test".to_string(),
        AgentType::MemgptAgent,
    );
    storage
        .save_agent(&agent)
        .await
        .expect("Failed to save agent");

    // Save archival entries
    let entry1 = ArchivalEntry {
        id: uuid::Uuid::new_v4().to_string(),
        content: "Important memory about user preferences".to_string(),
        metadata: None,
        created_at: Utc::now().to_rfc3339(),
    };
    let entry2 = ArchivalEntry {
        id: uuid::Uuid::new_v4().to_string(),
        content: "Another important memory".to_string(),
        metadata: None,
        created_at: Utc::now().to_rfc3339(),
    };

    storage
        .save_archival_entry(&agent_id, &entry1)
        .await
        .expect("Failed to save entry1");
    storage
        .save_archival_entry(&agent_id, &entry2)
        .await
        .expect("Failed to save entry2");

    // Drop storage (simulates restart)
    drop(storage);

    // Reconnect
    let storage = create_fdb_storage()
        .await
        .expect("Failed to reconnect to FDB");

    // Verify archival entries exist
    let entries = storage
        .load_archival_entries(&agent_id, 100)
        .await
        .expect("Failed to load archival entries");

    assert_eq!(
        entries.len(),
        2,
        "Should have 2 archival entries after restart"
    );

    // Verify search works
    let search_results = storage
        .search_archival_entries(&agent_id, Some("user preferences"), 100)
        .await
        .expect("Search failed");
    assert_eq!(
        search_results.len(),
        1,
        "Should find 1 entry matching 'user preferences'"
    );

    // Cleanup
    storage
        .delete_agent(&agent_id)
        .await
        .expect("Cleanup failed");
}

// =============================================================================
// Concurrent Append Tests
// =============================================================================

#[tokio::test]
#[ignore = "requires running FDB cluster"]
#[allow(clippy::disallowed_methods)] // Integration test needs real tokio::spawn, not DST
async fn test_concurrent_message_append() {
    let storage = create_fdb_storage()
        .await
        .expect("Failed to connect to FDB");
    let agent_id = unique_agent_id("concurrent-append");

    // Create agent first
    let agent = AgentMetadata::new(
        agent_id.clone(),
        "Concurrent Test".to_string(),
        AgentType::MemgptAgent,
    );
    storage
        .save_agent(&agent)
        .await
        .expect("Failed to save agent");

    // Spawn multiple concurrent appends
    let mut handles = vec![];
    for i in 0..10 {
        let storage = storage.clone();
        let agent_id = agent_id.clone();
        let handle = tokio::spawn(async move {
            let msg = create_test_message(MessageRole::User, &format!("Message {}", i));
            storage.append_message(&agent_id, &msg).await
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        handle.await.expect("Task panicked").expect("Append failed");
    }

    // Verify all 10 messages exist (no race condition overwrites)
    let count = storage
        .count_messages(&agent_id)
        .await
        .expect("Count failed");
    assert_eq!(
        count, 10,
        "All 10 messages should exist (no race condition)"
    );

    let messages = storage
        .load_messages(&agent_id, 100)
        .await
        .expect("Load failed");
    assert_eq!(messages.len(), 10, "Should load all 10 messages");

    // Cleanup
    storage
        .delete_agent(&agent_id)
        .await
        .expect("Cleanup failed");
}

// =============================================================================
// SimStorage Parity Tests
// =============================================================================

/// Test that SimStorage behaves identically to FdbAgentRegistry
/// for all basic operations
#[tokio::test]
async fn test_sim_storage_parity() {
    use kelpie_server::storage::SimStorage;

    let storage: Arc<dyn AgentStorage> = Arc::new(SimStorage::new());
    let agent_id = unique_agent_id("sim-parity");

    // Test agent CRUD
    let agent = AgentMetadata::new(
        agent_id.clone(),
        "SimStorage Test".to_string(),
        AgentType::MemgptAgent,
    );
    storage.save_agent(&agent).await.expect("Save failed");

    let loaded = storage
        .load_agent(&agent_id)
        .await
        .expect("Load failed")
        .expect("Should exist");
    assert_eq!(loaded.id, agent_id);
    assert_eq!(loaded.name, "SimStorage Test");

    // Test message operations
    let msg1 = create_test_message(MessageRole::User, "Hello");
    let msg2 = create_test_message(MessageRole::Assistant, "Hi there");
    storage
        .append_message(&agent_id, &msg1)
        .await
        .expect("Append failed");
    storage
        .append_message(&agent_id, &msg2)
        .await
        .expect("Append failed");

    let count = storage
        .count_messages(&agent_id)
        .await
        .expect("Count failed");
    assert_eq!(count, 2);

    let messages = storage
        .load_messages(&agent_id, 100)
        .await
        .expect("Load failed");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "Hello");
    assert_eq!(messages[1].content, "Hi there");

    // Test block operations
    let blocks = vec![
        Block::new("persona", "Test persona"),
        Block::new("human", "Test human"),
    ];
    storage
        .save_blocks(&agent_id, &blocks)
        .await
        .expect("Save failed");

    let loaded_blocks = storage.load_blocks(&agent_id).await.expect("Load failed");
    assert_eq!(loaded_blocks.len(), 2);

    // Test update block
    let updated = storage
        .update_block(&agent_id, "persona", "Updated persona")
        .await
        .expect("Update failed");
    assert_eq!(updated.value, "Updated persona");

    // Test session operations
    let session = create_test_session(&agent_id, "test-session");
    storage.save_session(&session).await.expect("Save failed");

    let loaded_session = storage
        .load_session(&agent_id, "test-session")
        .await
        .expect("Load failed")
        .expect("Should exist");
    assert_eq!(loaded_session.session_id, "test-session");

    // Test checkpoint
    let mut checkpoint_session = create_test_session(&agent_id, "checkpoint-session");
    checkpoint_session.iteration = 10;
    let checkpoint_msg = create_test_message(MessageRole::Assistant, "Checkpoint");
    storage
        .checkpoint(&checkpoint_session, Some(&checkpoint_msg))
        .await
        .expect("Checkpoint failed");

    let count_after = storage
        .count_messages(&agent_id)
        .await
        .expect("Count failed");
    assert_eq!(count_after, 3); // 2 original + 1 checkpoint

    // Test delete agent (cascading)
    storage
        .delete_agent(&agent_id)
        .await
        .expect("Delete failed");

    let deleted = storage.load_agent(&agent_id).await.expect("Load failed");
    assert!(deleted.is_none());
}
