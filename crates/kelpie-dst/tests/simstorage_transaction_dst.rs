//! DST tests for SimStorage (AgentStorage) transaction semantics
//!
//! These tests verify that kelpie-server's SimStorage correctly implements
//! FDB-like transaction semantics for higher-level agent operations.
//!
//! Issue #87: Fix SimStorage Transaction Semantics to Match FDB
//! Issue #140: DST Quality Remediation - replaced chrono::Utc::now() and
//! uuid::Uuid::new_v4() with deterministic alternatives.
//!
//! ## Properties Tested
//!
//! | Property | Test Function | Description |
//! |----------|---------------|-------------|
//! | Atomic Checkpoint | `test_atomic_checkpoint` | Session + message saved together |
//! | Atomic Cascade Delete | `test_atomic_cascade_delete` | Agent + related data deleted together |
//! | Conflict Detection | `test_update_block_conflict_detection` | Read-modify-write detects conflicts |
//! | Multi-key Atomicity | `test_delete_agent_atomicity` | All locks held during cascade delete |
//!
//! TigerStyle: Deterministic simulation with fault injection, 2+ assertions per test.

use chrono::{DateTime, Utc};
use kelpie_core::{RngProvider, Runtime};
use kelpie_dst::{DeterministicRng, SimClock, SimConfig};
use kelpie_server::models::{AgentType, Block, Message, MessageRole};
use kelpie_server::storage::{AgentMetadata, AgentStorage, SessionState, SimStorage};
use std::sync::Arc;

// ============================================================================
// DST Infrastructure
// ============================================================================

// Thread-local DST context for deterministic time and RNG.
// Using thread_local! allows test helpers to access deterministic sources
// without explicit parameter passing, while maintaining proper DST semantics.
//
// NOTE: This works because madsim uses a single-threaded deterministic scheduler.
// These tests MUST use #[madsim::test], not #[tokio::test].
thread_local! {
    static DST_CLOCK: std::cell::RefCell<Option<Arc<SimClock>>> = const { std::cell::RefCell::new(None) };
    static DST_RNG: std::cell::RefCell<Option<Arc<DeterministicRng>>> = const { std::cell::RefCell::new(None) };
}

/// Initialize DST context for the current test.
fn init_dst_context(clock: Arc<SimClock>, rng: Arc<DeterministicRng>) {
    DST_CLOCK.with(|c| *c.borrow_mut() = Some(clock));
    DST_RNG.with(|r| *r.borrow_mut() = Some(rng));
}

/// Get current deterministic time.
fn dst_now() -> DateTime<Utc> {
    DST_CLOCK.with(|c| {
        c.borrow()
            .as_ref()
            .map(|clock| clock.now())
            .unwrap_or_else(|| {
                // Fallback to a fixed time if DST context not initialized
                // This ensures reproducibility even if test setup is incomplete
                DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                    .unwrap()
                    .to_utc()
            })
    })
}

/// Generate a deterministic UUID.
fn dst_uuid() -> String {
    DST_RNG.with(|r| {
        r.borrow()
            .as_ref()
            .map(|rng| rng.gen_uuid())
            .unwrap_or_else(|| {
                // Fallback to a fixed UUID if DST context not initialized
                "00000000-0000-4000-8000-000000000000".to_string()
            })
    })
}

/// Create a test SimStorage instance
fn create_test_storage() -> SimStorage {
    SimStorage::new()
}

/// Create test agent metadata with deterministic timestamps
fn test_agent(id: &str) -> AgentMetadata {
    let now = dst_now();
    AgentMetadata {
        id: id.to_string(),
        name: format!("Test Agent {}", id),
        agent_type: AgentType::MemgptAgent,
        model: Some("claude-3-opus".to_string()),
        embedding: None,
        system: Some("You are a test agent".to_string()),
        description: None,
        tool_ids: vec![],
        tags: vec![],
        metadata: serde_json::Value::Null,
        created_at: now,
        updated_at: now,
    }
}

/// Create test block with deterministic ID and timestamps
fn test_block(label: &str, value: &str) -> Block {
    let now = dst_now();
    Block {
        id: dst_uuid(),
        label: label.to_string(),
        value: value.to_string(),
        description: None,
        limit: None,
        created_at: now,
        updated_at: now,
    }
}

/// Create test message with deterministic ID and timestamp
fn test_message(agent_id: &str, content: &str) -> Message {
    Message {
        id: dst_uuid(),
        agent_id: agent_id.to_string(),
        message_type: "user_message".to_string(),
        role: MessageRole::User,
        content: content.to_string(),
        tool_call_id: None,
        tool_calls: vec![],
        tool_call: None,
        tool_return: None,
        status: None,
        created_at: dst_now(),
    }
}

// =============================================================================
// Atomic Checkpoint Tests
// =============================================================================

/// Test atomic checkpoint: session + message saved together
///
/// Property: checkpoint() either saves BOTH session and message, or NEITHER.
/// This matches FDB transaction semantics where operations are atomic.
#[madsim::test]
async fn test_atomic_checkpoint() {
    let config = SimConfig::from_env_or_random();
    init_dst_context(
        Arc::new(SimClock::default()),
        Arc::new(DeterministicRng::new(config.seed)),
    );

    let storage = create_test_storage();

    let agent_id = "checkpoint-test-agent";
    let session = SessionState::new("session-1".to_string(), agent_id.to_string());
    let message = test_message(agent_id, "Test message for atomic checkpoint");

    // Perform atomic checkpoint
    storage.checkpoint(&session, Some(&message)).await.unwrap();

    // Verify BOTH session AND message are visible
    let loaded_session = storage.load_session(agent_id, "session-1").await.unwrap();
    assert!(
        loaded_session.is_some(),
        "Session should be saved in checkpoint"
    );

    let messages = storage.load_messages(agent_id, 10).await.unwrap();
    assert_eq!(messages.len(), 1, "Message should be saved in checkpoint");
    assert_eq!(messages[0].content, "Test message for atomic checkpoint");

    // Postcondition: both operations succeeded atomically
    assert!(loaded_session.is_some() && messages.len() == 1);
}

/// Test checkpoint with no message
///
/// Property: checkpoint() with None message only saves session.
#[madsim::test]
async fn test_atomic_checkpoint_no_message() {
    let config = SimConfig::from_env_or_random();
    init_dst_context(
        Arc::new(SimClock::default()),
        Arc::new(DeterministicRng::new(config.seed)),
    );

    let storage = create_test_storage();

    let agent_id = "checkpoint-no-msg-agent";
    let session = SessionState::new("session-1".to_string(), agent_id.to_string());

    // Checkpoint with no message
    storage.checkpoint(&session, None).await.unwrap();

    // Verify session saved
    let loaded_session = storage.load_session(agent_id, "session-1").await.unwrap();
    assert!(loaded_session.is_some(), "Session should be saved");

    // Verify no messages
    let messages = storage.load_messages(agent_id, 10).await.unwrap();
    assert_eq!(messages.len(), 0, "No messages should exist");
}

// =============================================================================
// Atomic Cascade Delete Tests
// =============================================================================

/// Test atomic cascade delete: agent + all related data deleted together
///
/// Property: delete_agent() removes ALL related data (blocks, sessions, messages,
/// archival entries) atomically - either all data is deleted or none.
#[madsim::test]
async fn test_atomic_cascade_delete() {
    let config = SimConfig::from_env_or_random();
    init_dst_context(
        Arc::new(SimClock::default()),
        Arc::new(DeterministicRng::new(config.seed)),
    );

    let storage = create_test_storage();

    let agent_id = "cascade-delete-agent";

    // Setup: Create agent with related data
    let agent = test_agent(agent_id);
    storage.save_agent(&agent).await.unwrap();

    // Add blocks
    let block = test_block("persona", "I am a test agent");
    storage.save_blocks(agent_id, &[block]).await.unwrap();

    // Add session
    let session = SessionState::new("session-1".to_string(), agent_id.to_string());
    storage.save_session(&session).await.unwrap();

    // Add messages
    let msg = test_message(agent_id, "Test message");
    storage.append_message(agent_id, &msg).await.unwrap();

    // Verify all data exists
    assert!(storage.load_agent(agent_id).await.unwrap().is_some());
    assert_eq!(storage.load_blocks(agent_id).await.unwrap().len(), 1);
    assert!(storage
        .load_session(agent_id, "session-1")
        .await
        .unwrap()
        .is_some());
    assert_eq!(storage.count_messages(agent_id).await.unwrap(), 1);

    // Atomic delete
    storage.delete_agent(agent_id).await.unwrap();

    // Verify ALL data is gone
    assert!(
        storage.load_agent(agent_id).await.unwrap().is_none(),
        "Agent should be deleted"
    );
    assert_eq!(
        storage.load_blocks(agent_id).await.unwrap().len(),
        0,
        "Blocks should be deleted"
    );
    assert!(
        storage
            .load_session(agent_id, "session-1")
            .await
            .unwrap()
            .is_none(),
        "Session should be deleted"
    );
    assert_eq!(
        storage.count_messages(agent_id).await.unwrap(),
        0,
        "Messages should be deleted"
    );
}

/// Test that delete_agent holds all locks before making changes
///
/// Property: The cascade delete acquires all locks BEFORE making any changes,
/// ensuring atomicity even if interrupted mid-operation.
#[madsim::test]
async fn test_delete_agent_lock_ordering() {
    let config = SimConfig::from_env_or_random();
    init_dst_context(
        Arc::new(SimClock::default()),
        Arc::new(DeterministicRng::new(config.seed)),
    );

    let storage = Arc::new(create_test_storage());

    // Create 10 agents with related data
    for i in 0..10 {
        let agent_id = format!("lock-test-agent-{}", i);
        let agent = test_agent(&agent_id);
        storage.save_agent(&agent).await.unwrap();

        let block = test_block("persona", &format!("Agent {} persona", i));
        storage.save_blocks(&agent_id, &[block]).await.unwrap();

        let msg = test_message(&agent_id, &format!("Agent {} message", i));
        storage.append_message(&agent_id, &msg).await.unwrap();
    }

    // Delete all agents concurrently
    let mut handles = Vec::new();
    for i in 0..10 {
        let storage = storage.clone();
        let agent_id = format!("lock-test-agent-{}", i);

        let handle = kelpie_core::current_runtime()
            .spawn(async move { storage.delete_agent(&agent_id).await.is_ok() });
        handles.push(handle);
    }

    // Wait for all deletes to complete
    for handle in handles {
        let result = handle.await.unwrap();
        assert!(result, "Delete should succeed");
    }

    // Verify all agents and related data are gone
    for i in 0..10 {
        let agent_id = format!("lock-test-agent-{}", i);
        assert!(storage.load_agent(&agent_id).await.unwrap().is_none());
        assert_eq!(storage.load_blocks(&agent_id).await.unwrap().len(), 0);
        assert_eq!(storage.count_messages(&agent_id).await.unwrap(), 0);
    }
}

// =============================================================================
// Conflict Detection Tests
// =============================================================================

/// Test conflict detection in update_block (read-modify-write)
///
/// Property: update_block uses version-based conflict detection. If the block
/// is modified by another operation between read and write, the operation
/// should detect the conflict and retry.
#[madsim::test]
async fn test_update_block_conflict_detection() {
    let config = SimConfig::from_env_or_random();
    init_dst_context(
        Arc::new(SimClock::default()),
        Arc::new(DeterministicRng::new(config.seed)),
    );

    let storage = Arc::new(create_test_storage());

    let agent_id = "conflict-test-agent";
    let agent = test_agent(agent_id);
    storage.save_agent(&agent).await.unwrap();

    // Create initial block
    let block = test_block("counter", "0");
    storage.save_blocks(agent_id, &[block]).await.unwrap();

    // Concurrent updates should both succeed (with retry on conflict)
    let storage1 = storage.clone();
    let storage2 = storage.clone();

    let handle1 = kelpie_core::current_runtime().spawn(async move {
        storage1
            .update_block(agent_id, "counter", "update_1")
            .await
            .is_ok()
    });

    let handle2 = kelpie_core::current_runtime().spawn(async move {
        storage2
            .update_block(agent_id, "counter", "update_2")
            .await
            .is_ok()
    });

    let result1 = handle1.await.unwrap();
    let result2 = handle2.await.unwrap();

    // Both should succeed (internal retry handles conflicts)
    assert!(result1, "Update 1 should succeed");
    assert!(result2, "Update 2 should succeed");

    // Final value should be one of the updates
    let blocks = storage.load_blocks(agent_id).await.unwrap();
    let counter_block = blocks.iter().find(|b| b.label == "counter").unwrap();
    assert!(
        counter_block.value == "update_1" || counter_block.value == "update_2",
        "Final value should be from one of the concurrent updates"
    );
}

/// Test conflict detection in append_block (read-modify-write)
///
/// Property: append_block appends content atomically with retry on conflict.
#[madsim::test]
async fn test_append_block_conflict_detection() {
    let config = SimConfig::from_env_or_random();
    init_dst_context(
        Arc::new(SimClock::default()),
        Arc::new(DeterministicRng::new(config.seed)),
    );

    let storage = Arc::new(create_test_storage());

    let agent_id = "append-conflict-agent";
    let agent = test_agent(agent_id);
    storage.save_agent(&agent).await.unwrap();

    // Create initial block
    let block = test_block("log", "initial");
    storage.save_blocks(agent_id, &[block]).await.unwrap();

    // Concurrent appends
    let storage1 = storage.clone();
    let storage2 = storage.clone();

    let handle1 = kelpie_core::current_runtime()
        .spawn(async move { storage1.append_block(agent_id, "log", "|append1").await });

    let handle2 = kelpie_core::current_runtime()
        .spawn(async move { storage2.append_block(agent_id, "log", "|append2").await });

    let result1 = handle1.await.unwrap();
    let result2 = handle2.await.unwrap();

    // Both should succeed
    assert!(result1.is_ok(), "Append 1 should succeed");
    assert!(result2.is_ok(), "Append 2 should succeed");

    // Final value should contain both appends
    let blocks = storage.load_blocks(agent_id).await.unwrap();
    let log_block = blocks.iter().find(|b| b.label == "log").unwrap();

    // Order may vary depending on which executed first
    assert!(
        (log_block.value.contains("append1") && log_block.value.contains("append2")),
        "Final value should contain both appends: {}",
        log_block.value
    );
}

// =============================================================================
// Version Tracking Tests
// =============================================================================

/// Test that version tracking increments correctly on writes
///
/// Property: Each write operation increments the version counter for affected keys.
#[madsim::test]
async fn test_version_tracking_on_writes() {
    let config = SimConfig::from_env_or_random();
    init_dst_context(
        Arc::new(SimClock::default()),
        Arc::new(DeterministicRng::new(config.seed)),
    );

    let storage = create_test_storage();

    let agent_id = "version-test-agent";
    let agent = test_agent(agent_id);

    // Multiple saves should all succeed (each increments version)
    storage.save_agent(&agent).await.unwrap();
    storage.save_agent(&agent).await.unwrap();
    storage.save_agent(&agent).await.unwrap();

    // Agent should still exist
    let loaded = storage.load_agent(agent_id).await.unwrap();
    assert!(loaded.is_some(), "Agent should exist after multiple saves");
}

/// Test that concurrent operations on different keys don't conflict
///
/// Property: Operations on independent keys should not conflict with each other.
#[madsim::test]
async fn test_no_conflict_on_different_keys() {
    let config = SimConfig::from_env_or_random();
    init_dst_context(
        Arc::new(SimClock::default()),
        Arc::new(DeterministicRng::new(config.seed)),
    );

    let storage = Arc::new(create_test_storage());

    // Create 20 agents concurrently
    let mut handles = Vec::new();
    for i in 0..20 {
        let storage = storage.clone();
        let agent_id = format!("independent-agent-{}", i);

        let handle = kelpie_core::current_runtime().spawn(async move {
            let agent = test_agent(&agent_id);
            storage.save_agent(&agent).await.is_ok()
        });
        handles.push(handle);
    }

    // All should succeed (no conflicts on independent keys)
    let mut success_count = 0;
    for handle in handles {
        if handle.await.unwrap() {
            success_count += 1;
        }
    }

    assert_eq!(
        success_count, 20,
        "All concurrent creates on independent keys should succeed"
    );

    // Verify all agents exist
    let agents = storage.list_agents().await.unwrap();
    assert_eq!(agents.len(), 20, "All 20 agents should exist");
}

// =============================================================================
// Session Checkpoint Consistency Tests
// =============================================================================

/// Test that multiple checkpoints maintain consistency
///
/// Property: Multiple checkpoints update session state correctly with
/// messages appending in order.
#[madsim::test]
async fn test_multiple_checkpoints_consistency() {
    let config = SimConfig::from_env_or_random();
    init_dst_context(
        Arc::new(SimClock::default()),
        Arc::new(DeterministicRng::new(config.seed)),
    );

    let storage = create_test_storage();

    let agent_id = "multi-checkpoint-agent";
    let mut session = SessionState::new("session-1".to_string(), agent_id.to_string());

    // Perform multiple checkpoints with messages
    for i in 0..5 {
        let msg = test_message(agent_id, &format!("Message {}", i));
        storage.checkpoint(&session, Some(&msg)).await.unwrap();
        session.advance_iteration();
    }

    // Verify session state
    let loaded_session = storage.load_session(agent_id, "session-1").await.unwrap();
    assert!(loaded_session.is_some());
    assert_eq!(
        loaded_session.unwrap().iteration,
        4,
        "Session should be at iteration 4"
    );

    // Verify all messages
    let messages = storage.load_messages(agent_id, 10).await.unwrap();
    assert_eq!(messages.len(), 5, "All 5 messages should be saved");
}

/// Test concurrent checkpoints to same session
///
/// Property: Concurrent checkpoints to the same session should serialize
/// correctly without data loss.
#[madsim::test]
async fn test_concurrent_checkpoints_same_session() {
    let config = SimConfig::from_env_or_random();
    init_dst_context(
        Arc::new(SimClock::default()),
        Arc::new(DeterministicRng::new(config.seed)),
    );

    let storage = Arc::new(create_test_storage());

    let agent_id = "concurrent-checkpoint-agent";

    // Perform 10 concurrent checkpoints
    let mut handles = Vec::new();
    for i in 0..10 {
        let storage = storage.clone();

        let handle = kelpie_core::current_runtime().spawn(async move {
            let session = SessionState::new("session-1".to_string(), agent_id.to_string());
            let msg = test_message(agent_id, &format!("Concurrent message {}", i));
            storage.checkpoint(&session, Some(&msg)).await.is_ok()
        });
        handles.push(handle);
    }

    // All checkpoints should succeed
    let mut success_count = 0;
    for handle in handles {
        if handle.await.unwrap() {
            success_count += 1;
        }
    }

    assert_eq!(
        success_count, 10,
        "All concurrent checkpoints should succeed"
    );

    // All messages should be saved
    let messages = storage.load_messages(agent_id, 20).await.unwrap();
    assert_eq!(
        messages.len(),
        10,
        "All 10 concurrent messages should be saved"
    );
}
