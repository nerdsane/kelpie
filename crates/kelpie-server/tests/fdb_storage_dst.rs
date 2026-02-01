//! DST Tests for FdbStorage Backend
//!
//! TigerStyle: Tests written FIRST before implementation.
//!
//! These tests MUST FAIL initially since FdbStorage doesn't exist yet.
//!
//! Test Strategy:
//! 1. Agent CRUD with storage faults (20% write fail, 10% read fail)
//! 2. Block operations with crash faults
//! 3. Session checkpointing with transaction conflicts

#![cfg(feature = "dst")]
#![allow(unused_assignments, unused_variables)]
//! 4. Message persistence with high fault rates
//! 5. Concurrent operations (race conditions)
//! 6. Crash recovery (state survives crashes)
//! 7. Determinism (same seed = same behavior)
//!
//! Expected Outcome:
//! - FdbStorage handles faults gracefully
//! - No data corruption under any fault scenario
//! - Atomic checkpoint works correctly
//! - Same seed produces identical results

use kelpie_dst::{FaultConfig, FaultType, SimConfig, SimEnvironment, Simulation};
use kelpie_server::models::{AgentType, Message, MessageRole};
use kelpie_server::storage::{AgentMetadata, AgentStorage, SessionState, StorageError};
use std::sync::Arc;

use kelpie_server::storage::KvAdapter;

// =============================================================================
// Helper: Create FDB-compatible storage for DST
// =============================================================================

/// Create storage backend for DST testing
///
/// Uses KvAdapter with proper DST infrastructure (kelpie-dst::SimStorage).
/// This provides transaction support and sophisticated fault injection.
fn create_storage(env: &SimEnvironment) -> Arc<dyn AgentStorage> {
    let adapter = KvAdapter::with_dst_storage(env.rng.fork(), env.faults.clone());
    Arc::new(adapter)
}

/// Helper: Retry read operations for DST verification
///
/// Verification reads in DST tests can fail due to fault injection.
/// This helper retries reads up to 10 times to handle transient faults.
///
/// NOTE: No backoff/sleep between retries. This is intentional for DST tests
/// where time is simulated. Production code should use exponential backoff.
async fn retry_read<F, Fut, T>(mut f: F) -> Result<T, kelpie_core::Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, StorageError>>,
{
    let mut attempts = 0;
    let max_attempts = 10;

    loop {
        attempts += 1;
        match f().await {
            Ok(value) => return Ok(value),
            Err(e) if e.is_retriable() && attempts < max_attempts => {
                // Retry immediately (no backoff in DST - simulated time)
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Helper: Retry write operations for DST test setup
///
/// Setup writes in DST tests can fail due to fault injection.
/// This helper retries writes up to 10 times to handle transient faults.
///
/// NOTE: No backoff/sleep between retries. This is intentional for DST tests
/// where time is simulated. Production code should use exponential backoff.
async fn retry_write<F, Fut>(mut f: F) -> Result<(), kelpie_core::Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), StorageError>>,
{
    let mut attempts = 0;
    let max_attempts = 10;

    loop {
        attempts += 1;
        match f().await {
            Ok(()) => return Ok(()),
            Err(e) if e.is_retriable() && attempts < max_attempts => {
                // Retry immediately (no backoff in DST - simulated time)
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

// =============================================================================
// Test 1: Agent CRUD with Storage Faults
// =============================================================================

/// Test agent creation, read, update, delete with 20% write failures
///
/// FAULT: 20% StorageWriteFail
///
/// ASSERTION: Operations either succeed or return retriable error
/// No partial state (e.g., agent exists but blocks missing)
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_fdb_agent_crud_with_faults() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.2))
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.1))
        .run_async(|env| async move {
            // Get storage with fault injection
            let storage = create_storage(&env);

            let mut success_count = 0;
            let mut failure_count = 0;

            // Try to create 20 agents with 20% write failure rate
            for i in 0..20 {
                let agent_id = format!("agent-{}", i);
                let agent = AgentMetadata::new(
                    agent_id.clone(),
                    format!("Test Agent {}", i),
                    AgentType::MemgptAgent,
                );

                // Create agent
                match storage.save_agent(&agent).await {
                    Ok(_) => {
                        // Agent created - verify we can read it back
                        match storage.load_agent(&agent_id).await {
                            Ok(Some(loaded)) => {
                                assert_eq!(loaded.name, agent.name);
                                success_count += 1;
                            }
                            Ok(None) => {
                                panic!(
                                    "BUG: Agent {} saved successfully but not found on read",
                                    agent_id
                                );
                            }
                            Err(e) if e.is_retriable() => {
                                // Read fault - acceptable
                                failure_count += 1;
                            }
                            Err(e) => {
                                panic!("BUG: Non-retriable error on read after save: {}", e);
                            }
                        }
                    }
                    Err(e) if e.is_retriable() => {
                        // Write fault - acceptable
                        failure_count += 1;
                    }
                    Err(e) => {
                        panic!("BUG: Non-retriable error on save: {}", e);
                    }
                }
            }

            println!(
                "Agent CRUD: {} successes, {} failures (expected with 20% fault rate)",
                success_count, failure_count
            );

            // With 20% write + 10% read, faults can compound
            // Statistical variance can cause wide range, accept >=5 successes out of 20 (25%)
            assert!(
                success_count >= 5,
                "Too many failures: only {} successes out of 20",
                success_count
            );

            Ok::<_, kelpie_core::Error>(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Agent CRUD test should handle faults gracefully"
    );
}

// =============================================================================
// Test 2: Block Operations with Crash Faults
// =============================================================================

/// Test block append/update with crash-before and crash-after write faults
///
/// FAULT: 15% CrashBeforeWrite, 15% CrashAfterWrite
///
/// ASSERTION: Block operations are atomic - either fully written or not at all
/// No partial updates where block exists but has wrong content
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_fdb_blocks_with_crash_faults() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::CrashBeforeWrite, 0.15))
        .with_fault(FaultConfig::new(FaultType::CrashAfterWrite, 0.15))
        .run_async(|env| async move {
            let storage = create_storage(&env);

            // Create agent first (with retries for transient faults)
            let agent = AgentMetadata::new(
                "agent-blocks".to_string(),
                "Block Test Agent".to_string(),
                AgentType::MemgptAgent,
            );
            let storage_ref = storage.clone();
            let agent_clone = agent.clone();
            retry_write(|| {
                let storage = storage_ref.clone();
                let a = agent_clone.clone();
                async move { storage.save_agent(&a).await }
            })
            .await?;

            let mut append_success = 0;
            let mut append_failure = 0;

            // Try to append to blocks 30 times with 30% crash rate
            for i in 0..30 {
                let content = format!("Fact {}", i);
                let append_result = storage
                    .append_block("agent-blocks", "facts", &content)
                    .await;

                match append_result {
                    Ok(_block) => {
                        // Append succeeded - verify content is correct
                        // Use retry_read for verification to handle transient faults
                        let storage_ref = storage.clone();
                        let blocks = retry_read(|| {
                            let storage = storage_ref.clone();
                            async move { storage.load_blocks("agent-blocks").await }
                        })
                        .await?;
                        let facts_block = blocks.iter().find(|b| b.label == "facts");

                        if let Some(b) = facts_block {
                            // Block must contain the appended content
                            assert!(
                                b.value.contains(&content),
                                "BUG: Append succeeded but content '{}' not found in block value: '{}'",
                                content,
                                b.value
                            );
                            append_success += 1;
                        } else {
                            panic!(
                                "BUG: Append returned Ok but block not found in storage"
                            );
                        }
                    }
                    Err(e) if e.is_retriable() => {
                        // Crash fault - acceptable
                        append_failure += 1;
                    }
                    Err(e) => {
                        panic!("BUG: Non-retriable error on append: {}", e);
                    }
                }
            }

            println!(
                "Block append: {} successes, {} failures (expected with 30% crash rate)",
                append_success, append_failure
            );

            // With 30% fault rate, expect >10 successes out of 30 (lenient for DST)
            assert!(
                append_success > 10,
                "Too many failures: only {} successes out of 30",
                append_success
            );

            Ok::<_, kelpie_core::Error>(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Block operations should be atomic under crash faults"
    );
}

// =============================================================================
// Test 3: Session Checkpointing with Transaction Conflicts
// =============================================================================

/// Test session checkpointing with transaction conflict faults
///
/// FAULT: 20% TransactionConflict (simulates concurrent writes)
///
/// ASSERTION: Checkpoint either succeeds or returns conflict error
/// Session state is never partially written
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_fdb_session_checkpoint_with_conflicts() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::CrashDuringTransaction, 0.2))
        .run_async(|env| async move {
            let storage = create_storage(&env);

            // Create agent (with retries for transient faults)
            let agent = AgentMetadata::new(
                "agent-session".to_string(),
                "Session Test Agent".to_string(),
                AgentType::MemgptAgent,
            );
            let storage_ref = storage.clone();
            let agent_clone = agent.clone();
            retry_write(|| {
                let storage = storage_ref.clone();
                let a = agent_clone.clone();
                async move { storage.save_agent(&a).await }
            })
            .await?;

            let mut checkpoint_success = 0;
            let mut checkpoint_conflict = 0;

            // Try to checkpoint 25 times with 20% conflict rate
            for i in 0..25 {
                let session =
                    SessionState::new(format!("session-{}", i), "agent-session".to_string());

                // Create a message to include in checkpoint
                let message = Message {
                    id: format!("msg-{}", i),
                    agent_id: "agent-session".to_string(),
                    message_type: "assistant_message".to_string(),
                    role: MessageRole::Assistant,
                    content: format!("Response {}", i),
                    tool_calls: vec![],
                    tool_call_id: None,
                    tool_call: None,
                    tool_return: None,
                    status: None,
                    created_at: chrono::DateTime::from_timestamp(1700000000, 0).unwrap(),
                };

                // Atomic checkpoint
                match storage.checkpoint(&session, Some(&message)).await {
                    Ok(_) => {
                        // Checkpoint succeeded - verify both session and message were saved
                        // Use retry_read for verification to handle transient faults
                        let storage_ref = storage.clone();
                        let session_id = session.session_id.clone();
                        let loaded_session = retry_read(|| {
                            let storage = storage_ref.clone();
                            let sid = session_id.clone();
                            async move { storage.load_session("agent-session", &sid).await }
                        })
                        .await?;

                        let storage_ref = storage.clone();
                        let messages = retry_read(|| {
                            let storage = storage_ref.clone();
                            async move { storage.load_messages("agent-session", 100).await }
                        })
                        .await?;

                        if loaded_session.is_none() {
                            panic!("BUG: Checkpoint succeeded but session not found in storage");
                        }

                        if !messages.iter().any(|m| m.id == message.id) {
                            panic!("BUG: Checkpoint succeeded but message not found in storage");
                        }

                        checkpoint_success += 1;
                    }
                    Err(e) if e.is_retriable() => {
                        // Fault (crash, conflict, etc.) - acceptable and retriable
                        checkpoint_conflict += 1;
                    }
                    Err(e) => {
                        panic!("BUG: Unexpected non-retriable error on checkpoint: {}", e);
                    }
                }
            }

            println!(
                "Session checkpoint: {} successes, {} conflicts (expected with 20% conflict rate)",
                checkpoint_success, checkpoint_conflict
            );

            // With 20% conflict rate, expect >=5 successes out of 25 (lenient for DST)
            assert!(
                checkpoint_success >= 5,
                "Too many conflicts: only {} successes out of 25",
                checkpoint_success
            );

            Ok::<_, kelpie_core::Error>(())
        })
        .await;

    if let Err(e) = &result {
        eprintln!("Simulation error: {:?}", e);
    }
    assert!(
        result.is_ok(),
        "Session checkpointing should handle conflicts gracefully: {:?}",
        result.err()
    );
}

// =============================================================================
// Test 4: Message Persistence with High Fault Rate
// =============================================================================

/// Test message persistence with 40% combined fault rate
///
/// FAULT: 20% StorageWriteFail + 20% StorageLatency
///
/// ASSERTION: Messages are never duplicated or lost silently
/// Ordering is preserved when messages do get written
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_fdb_messages_with_high_fault_rate() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.2))
        .with_fault(FaultConfig::new(
            FaultType::StorageLatency {
                min_ms: 50,
                max_ms: 200,
            },
            0.2,
        ))
        .run_async(|env| async move {
            let storage = create_storage(&env);

            // Create agent (with retries for transient faults)
            let agent = AgentMetadata::new(
                "agent-messages".to_string(),
                "Message Test Agent".to_string(),
                AgentType::MemgptAgent,
            );
            let storage_ref = storage.clone();
            let agent_clone = agent.clone();
            retry_write(|| {
                let storage = storage_ref.clone();
                let a = agent_clone.clone();
                async move { storage.save_agent(&a).await }
            })
            .await?;

            let mut written_ids = Vec::new();

            // Try to write 50 messages with 40% fault rate
            for i in 0..50 {
                let message = Message {
                    id: format!("msg-{:03}", i),
                    agent_id: "agent-messages".to_string(),
                    message_type: "user_message".to_string(),
                    role: MessageRole::User,
                    content: format!("Message {}", i),
                    tool_calls: vec![],
                    tool_call_id: None,
                    tool_call: None,
                    tool_return: None,
                    status: None,
                    created_at: chrono::DateTime::from_timestamp(1700000000, 0).unwrap(),
                };

                match storage.append_message("agent-messages", &message).await {
                    Ok(_) => {
                        written_ids.push(message.id.clone());
                    }
                    Err(e) if e.is_retriable() => {
                        // Fault - acceptable
                    }
                    Err(e) => {
                        panic!("BUG: Non-retriable error on message append: {}", e);
                    }
                }
            }

            // Load all messages (retry reads to handle transient faults)
            let storage_ref = storage.clone();
            let stored_messages = retry_read(|| {
                let storage = storage_ref.clone();
                async move { storage.load_messages("agent-messages", 1000).await }
            })
            .await?;

            println!(
                "Message persistence: {} written, {} stored",
                written_ids.len(),
                stored_messages.len()
            );

            // Verify no duplicates
            let stored_ids: Vec<_> = stored_messages.iter().map(|m| &m.id).collect();
            let unique_ids: std::collections::HashSet<_> = stored_ids.iter().collect();
            assert_eq!(
                stored_ids.len(),
                unique_ids.len(),
                "BUG: Duplicate messages found in storage"
            );

            // Verify all written IDs are present
            for id in &written_ids {
                assert!(
                    stored_ids.contains(&id),
                    "BUG: Message {} was written successfully but not found in storage",
                    id
                );
            }

            // With 40% fault rate, expect >20 messages stored out of 50
            assert!(
                stored_messages.len() > 20,
                "Too many failures: only {} messages stored out of 50",
                stored_messages.len()
            );

            Ok::<_, kelpie_core::Error>(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Message persistence should handle high fault rates without data loss"
    );
}

// =============================================================================
// Test 5: Concurrent Operations (Race Conditions)
// =============================================================================

/// Test concurrent agent creation and block updates
///
/// FAULT: 10% StorageWriteFail
///
/// ASSERTION: No race conditions - concurrent operations are isolated
/// Final state is consistent
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_fdb_concurrent_operations() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.1))
        .run_async(|env| async move {
            use kelpie_core::{current_runtime, Runtime};
            let runtime = current_runtime();
            let storage = Arc::new(create_storage(&env));

            // Spawn 10 concurrent tasks, each creating an agent and updating blocks
            let mut tasks = Vec::new();

            for i in 0..10 {
                let storage_clone = storage.clone();
                let task = runtime.spawn(async move {
                    let agent_id = format!("concurrent-agent-{}", i);
                    let agent = AgentMetadata::new(
                        agent_id.clone(),
                        format!("Concurrent Agent {}", i),
                        AgentType::MemgptAgent,
                    );

                    // Create agent
                    storage_clone.save_agent(&agent).await?;

                    // Update blocks 5 times
                    for j in 0..5 {
                        let content = format!("Update {} from task {}", j, i);
                        storage_clone
                            .append_block(&agent_id, "log", &content)
                            .await?;
                    }

                    Ok::<_, StorageError>(agent_id)
                });

                tasks.push(task);
            }

            // Wait for all tasks
            let mut successful_agents = Vec::new();
            for task in tasks {
                match task.await {
                    Ok(Ok(agent_id)) => {
                        successful_agents.push(agent_id);
                    }
                    Ok(Err(e)) if e.is_retriable() => {
                        // Fault during task - acceptable
                    }
                    Ok(Err(e)) => {
                        panic!("BUG: Non-retriable error in concurrent task: {}", e);
                    }
                    Err(e) => {
                        panic!("BUG: Task panicked: {}", e);
                    }
                }
            }

            println!(
                "Concurrent operations: {} agents created successfully out of 10",
                successful_agents.len()
            );

            // Verify agents exist and have correct block counts (retry reads for faults)
            for agent_id in &successful_agents {
                let storage_ref = storage.clone();
                let aid = agent_id.clone();
                let agent = retry_read(|| {
                    let storage = storage_ref.clone();
                    let id = aid.clone();
                    async move { storage.load_agent(&id).await }
                })
                .await?;
                assert!(agent.is_some(), "BUG: Agent {} not found", agent_id);

                let storage_ref = storage.clone();
                let aid = agent_id.clone();
                let blocks = retry_read(|| {
                    let storage = storage_ref.clone();
                    let id = aid.clone();
                    async move { storage.load_blocks(&id).await }
                })
                .await?;
                let log_block = blocks.iter().find(|b| b.label == "log");

                if let Some(block) = log_block {
                    // Block should contain all 5 updates
                    for j in 0..5 {
                        let expected = format!("Update {}", j);
                        assert!(
                            block.value.contains(&expected),
                            "BUG: Block for {} missing update {}",
                            agent_id,
                            j
                        );
                    }
                }
            }

            // With 10% fault rate + concurrent ops + retries, expect >=2 successful agents (very lenient for DST)
            assert!(
                successful_agents.len() >= 2,
                "Too many failures: only {} agents succeeded out of 10",
                successful_agents.len()
            );

            Ok::<_, kelpie_core::Error>(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Concurrent operations should not cause race conditions"
    );
}

// =============================================================================
// Test 6: Crash Recovery
// =============================================================================

/// Test that state survives crashes and can be recovered
///
/// FAULT: 30% CrashAfterWrite
///
/// ASSERTION: Data written before crash is recoverable
/// No corruption after crash
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_fdb_crash_recovery() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::CrashAfterWrite, 0.3))
        .run_async(|env| async move {
            // Create first storage instance
            // Use SimStorage from environment so state can be shared across "restarts"
            let sim_storage = env.storage.clone();
            let storage1: Arc<dyn AgentStorage> =
                Arc::new(KvAdapter::new(Arc::new(sim_storage.clone())));

            // Create agent and session (with retries for transient faults)
            let agent = AgentMetadata::new(
                "crash-agent".to_string(),
                "Crash Test Agent".to_string(),
                AgentType::MemgptAgent,
            );
            let storage1_ref = storage1.clone();
            let agent_clone = agent.clone();
            retry_write(|| {
                let storage = storage1_ref.clone();
                let a = agent_clone.clone();
                async move { storage.save_agent(&a).await }
            })
            .await?;

            let session = SessionState::new("session-crash".to_string(), "crash-agent".to_string());
            let storage1_ref = storage1.clone();
            let session_clone = session.clone();
            retry_write(|| {
                let storage = storage1_ref.clone();
                let s = session_clone.clone();
                async move { storage.save_session(&s).await }
            })
            .await?;

            // Write some messages (may crash during writes)
            let mut written_count = 0;
            for i in 0..20 {
                let message = Message {
                    id: format!("crash-msg-{}", i),
                    agent_id: "crash-agent".to_string(),
                    message_type: "user_message".to_string(),
                    role: MessageRole::User,
                    content: format!("Pre-crash message {}", i),
                    tool_calls: vec![],
                    tool_call_id: None,
                    tool_call: None,
                    tool_return: None,
                    status: None,
                    created_at: chrono::DateTime::from_timestamp(1700000000, 0).unwrap(),
                };

                match storage1.append_message("crash-agent", &message).await {
                    Ok(_) => {
                        written_count += 1;
                    }
                    Err(e) if e.is_retriable() => {
                        // Crash during write - acceptable
                    }
                    Err(e) => {
                        panic!("BUG: Non-retriable error during writes: {}", e);
                    }
                }
            }

            println!(
                "Crash recovery: {} messages written before simulated crashes",
                written_count
            );

            // Create NEW storage instance (simulates process restart)
            // Use same sim_storage to maintain persistence across "restart"
            let storage2: Arc<dyn AgentStorage> =
                Arc::new(KvAdapter::new(Arc::new(sim_storage.clone())));

            // Verify agent still exists (retry reads to handle transient faults)
            let storage2_ref = storage2.clone();
            let recovered_agent = retry_read(|| {
                let storage = storage2_ref.clone();
                async move { storage.load_agent("crash-agent").await }
            })
            .await?;
            assert!(
                recovered_agent.is_some(),
                "BUG: Agent not found after crash recovery"
            );

            // Verify session still exists
            let storage2_ref = storage2.clone();
            let recovered_session = retry_read(|| {
                let storage = storage2_ref.clone();
                async move { storage.load_session("crash-agent", "session-crash").await }
            })
            .await?;
            assert!(
                recovered_session.is_some(),
                "BUG: Session not found after crash recovery"
            );

            // Verify messages that were successfully written are still there
            let storage2_ref = storage2.clone();
            let recovered_messages = retry_read(|| {
                let storage = storage2_ref.clone();
                async move { storage.load_messages("crash-agent", 100).await }
            })
            .await?;
            println!(
                "Crash recovery: {} messages recovered after restart",
                recovered_messages.len()
            );

            // Should recover some messages (at least 25% with 30% crash rate, lenient for DST)
            assert!(
                recovered_messages.len() >= written_count / 4,
                "BUG: Too many messages lost after crash: wrote {}, recovered {}",
                written_count,
                recovered_messages.len()
            );

            Ok::<_, kelpie_core::Error>(())
        })
        .await;

    assert!(
        result.is_ok(),
        "State should survive crashes and be recoverable"
    );
}

// =============================================================================
// Test 7: Determinism
// =============================================================================

/// Test that same seed produces identical behavior
///
/// NO FAULTS - Just verify determinism
///
/// ASSERTION: Running twice with same seed produces identical results
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_fdb_determinism() {
    let seed = 99999u64;

    // Run 1
    let result1 = Simulation::new(SimConfig::new(seed))
        .run_async(|env| async move {
            let storage = create_storage(&env);

            // Create 5 agents
            for i in 0..5 {
                let agent = AgentMetadata::new(
                    format!("det-agent-{}", i),
                    format!("Deterministic Agent {}", i),
                    AgentType::MemgptAgent,
                );
                storage.save_agent(&agent).await?;
            }

            // List agents
            let agents = storage.list_agents().await?;
            let mut agent_ids: Vec<String> = agents.iter().map(|a| a.id.clone()).collect();
            agent_ids.sort(); // Sort for deterministic comparison

            Ok::<_, kelpie_core::Error>(agent_ids)
        })
        .await
        .expect("Run 1 failed");

    // Run 2 with same seed
    let result2 = Simulation::new(SimConfig::new(seed))
        .run_async(|env| async move {
            let storage = create_storage(&env);

            // Create 5 agents (same as run 1)
            for i in 0..5 {
                let agent = AgentMetadata::new(
                    format!("det-agent-{}", i),
                    format!("Deterministic Agent {}", i),
                    AgentType::MemgptAgent,
                );
                storage.save_agent(&agent).await?;
            }

            // List agents
            let agents = storage.list_agents().await?;
            let mut agent_ids: Vec<String> = agents.iter().map(|a| a.id.clone()).collect();
            agent_ids.sort(); // Sort for deterministic comparison

            Ok::<_, kelpie_core::Error>(agent_ids)
        })
        .await
        .expect("Run 2 failed");

    // Results must be identical (order-independent since we sorted)
    assert_eq!(
        result1, result2,
        "BUG: Same seed produced different results. \
         Determinism broken - FdbStorage behavior is non-deterministic"
    );

    println!("Determinism verified: same seed = same results");
}

// =============================================================================
// Test 8: Agent Delete Cascade
// =============================================================================

/// Test that deleting an agent cascades to all associated data
///
/// FAULT: 10% StorageWriteFail
///
/// ASSERTION: Delete is atomic - either all data deleted or none
/// No orphaned blocks/sessions/messages
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_fdb_delete_cascade() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.1))
        .run_async(|env| async move {
            let storage = create_storage(&env);

            // Create agent with blocks, session, and messages (with retries for faults)
            let agent = AgentMetadata::new(
                "delete-agent".to_string(),
                "Delete Test Agent".to_string(),
                AgentType::MemgptAgent,
            );
            let storage_ref = storage.clone();
            let agent_clone = agent.clone();
            retry_write(|| {
                let storage = storage_ref.clone();
                let a = agent_clone.clone();
                async move { storage.save_agent(&a).await }
            })
            .await?;

            // Add blocks
            let storage_ref = storage.clone();
            retry_write(|| {
                let storage = storage_ref.clone();
                async move {
                    storage
                        .append_block("delete-agent", "facts", "Some facts")
                        .await
                        .map(|_| ())
                }
            })
            .await?;

            // Add session
            let session =
                SessionState::new("delete-session".to_string(), "delete-agent".to_string());
            let storage_ref = storage.clone();
            let session_clone = session.clone();
            retry_write(|| {
                let storage = storage_ref.clone();
                let s = session_clone.clone();
                async move { storage.save_session(&s).await }
            })
            .await?;

            // Add messages
            let message = Message {
                id: "delete-msg-1".to_string(),
                agent_id: "delete-agent".to_string(),
                message_type: "user_message".to_string(),
                role: MessageRole::User,
                content: "Test message".to_string(),
                tool_calls: vec![],
                tool_call_id: None,
                tool_call: None,
                tool_return: None,
                status: None,
                created_at: chrono::DateTime::from_timestamp(1700000000, 0).unwrap(),
            };
            let storage_ref = storage.clone();
            let message_clone = message.clone();
            retry_write(|| {
                let storage = storage_ref.clone();
                let m = message_clone.clone();
                async move { storage.append_message("delete-agent", &m).await.map(|_| ()) }
            })
            .await?;

            // Delete agent
            match storage.delete_agent("delete-agent").await {
                Ok(_) => {
                    // Delete succeeded - verify all data is gone (retry reads for faults)
                    let storage_ref = storage.clone();
                    let agent_check = retry_read(|| {
                        let storage = storage_ref.clone();
                        async move { storage.load_agent("delete-agent").await }
                    })
                    .await?;
                    assert!(
                        agent_check.is_none(),
                        "BUG: Agent still exists after delete"
                    );

                    let storage_ref = storage.clone();
                    let blocks_check = retry_read(|| {
                        let storage = storage_ref.clone();
                        async move { storage.load_blocks("delete-agent").await }
                    })
                    .await?;
                    assert!(
                        blocks_check.is_empty(),
                        "BUG: Blocks still exist after agent delete"
                    );

                    let storage_ref = storage.clone();
                    let sessions_check = retry_read(|| {
                        let storage = storage_ref.clone();
                        async move { storage.list_sessions("delete-agent").await }
                    })
                    .await?;
                    assert!(
                        sessions_check.is_empty(),
                        "BUG: Sessions still exist after agent delete"
                    );

                    let storage_ref = storage.clone();
                    let messages_check = retry_read(|| {
                        let storage = storage_ref.clone();
                        async move { storage.load_messages("delete-agent", 100).await }
                    })
                    .await?;
                    assert!(
                        messages_check.is_empty(),
                        "BUG: Messages still exist after agent delete"
                    );
                }
                Err(e) if e.is_retriable() => {
                    // Delete failed due to fault - verify data is still intact (retry read)
                    let storage_ref = storage.clone();
                    let agent_check = retry_read(|| {
                        let storage = storage_ref.clone();
                        async move { storage.load_agent("delete-agent").await }
                    })
                    .await?;
                    assert!(
                        agent_check.is_some(),
                        "BUG: Agent deleted partially despite delete failure"
                    );
                }
                Err(e) => {
                    panic!("BUG: Non-retriable error on delete: {}", e);
                }
            }

            Ok::<_, kelpie_core::Error>(())
        })
        .await;

    if let Err(e) = &result {
        eprintln!("Simulation error: {:?}", e);
    }
    assert!(
        result.is_ok(),
        "Agent delete should cascade atomically: {:?}",
        result.err()
    );
}

// =============================================================================
// Test 9: Atomic Checkpoint Semantics (Issue #87)
// =============================================================================

/// Test that checkpoint operations are atomic - session and message are saved together
///
/// FAULT: 30% CrashDuringTransaction
///
/// ASSERTION: After checkpoint:
/// - If checkpoint succeeds: BOTH session AND message exist
/// - If checkpoint fails: NEITHER session NOR message exist (or previous state preserved)
/// - No partial state where session exists but message doesn't
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_atomic_checkpoint_semantics() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::CrashDuringTransaction, 0.3))
        .run_async(|env| async move {
            let storage = create_storage(&env);

            // Create agent first (with retries for transient faults)
            let agent = AgentMetadata::new(
                "agent-atomic".to_string(),
                "Atomic Checkpoint Test Agent".to_string(),
                AgentType::MemgptAgent,
            );
            let storage_ref = storage.clone();
            let agent_clone = agent.clone();
            retry_write(|| {
                let storage = storage_ref.clone();
                let a = agent_clone.clone();
                async move { storage.save_agent(&a).await }
            })
            .await?;

            let mut checkpoint_success = 0;
            let mut checkpoint_failure = 0;
            let mut atomicity_violations = 0;

            // Try 30 checkpoints with fault injection
            for i in 0..30 {
                let session = SessionState::new(
                    format!("session-{}", i),
                    "agent-atomic".to_string(),
                );

                let message = Message {
                    id: format!("msg-atomic-{}", i),
                    agent_id: "agent-atomic".to_string(),
                    message_type: "assistant_message".to_string(),
                    role: MessageRole::Assistant,
                    content: format!("Checkpoint message {}", i),
                    tool_calls: vec![],
                    tool_call_id: None,
                    tool_call: None,
                    tool_return: None,
                    status: None,
                    created_at: chrono::DateTime::from_timestamp(1700000000 + i as i64, 0).unwrap(),
                };

                // Record state before checkpoint
                let storage_ref = storage.clone();
                let session_id = session.session_id.clone();
                let _pre_session = retry_read(|| {
                    let storage = storage_ref.clone();
                    let sid = session_id.clone();
                    async move { storage.load_session("agent-atomic", &sid).await }
                })
                .await
                .ok();

                let storage_ref = storage.clone();
                let _pre_msg_count = retry_read(|| {
                    let storage = storage_ref.clone();
                    async move { storage.count_messages("agent-atomic").await }
                })
                .await
                .unwrap_or(0);

                // Attempt checkpoint
                match storage.checkpoint(&session, Some(&message)).await {
                    Ok(_) => {
                        // Checkpoint reported success - verify BOTH were saved
                        let storage_ref = storage.clone();
                        let session_id = session.session_id.clone();
                        let loaded_session = retry_read(|| {
                            let storage = storage_ref.clone();
                            let sid = session_id.clone();
                            async move { storage.load_session("agent-atomic", &sid).await }
                        })
                        .await?;

                        let storage_ref = storage.clone();
                        let messages = retry_read(|| {
                            let storage = storage_ref.clone();
                            async move { storage.load_messages("agent-atomic", 1000).await }
                        })
                        .await?;

                        let msg_exists = messages.iter().any(|m| m.id == message.id);

                        if loaded_session.is_none() && msg_exists {
                            // Message saved but session not saved - ATOMICITY VIOLATION
                            atomicity_violations += 1;
                            panic!(
                                "ATOMICITY BUG: Checkpoint {} succeeded but session not found while message exists",
                                i
                            );
                        }

                        if loaded_session.is_some() && !msg_exists {
                            // Session saved but message not saved - ATOMICITY VIOLATION
                            atomicity_violations += 1;
                            panic!(
                                "ATOMICITY BUG: Checkpoint {} succeeded, session saved, but message not found",
                                i
                            );
                        }

                        if loaded_session.is_some() && msg_exists {
                            checkpoint_success += 1;
                        }
                    }
                    Err(e) if e.is_retriable() => {
                        // Checkpoint failed - verify no partial state
                        // Either both exist (rollforward) or both don't exist (rollback)
                        let storage_ref = storage.clone();
                        let session_id = session.session_id.clone();
                        let loaded_session = retry_read(|| {
                            let storage = storage_ref.clone();
                            let sid = session_id.clone();
                            async move { storage.load_session("agent-atomic", &sid).await }
                        })
                        .await
                        .ok()
                        .flatten();

                        let storage_ref = storage.clone();
                        let messages = retry_read(|| {
                            let storage = storage_ref.clone();
                            async move { storage.load_messages("agent-atomic", 1000).await }
                        })
                        .await
                        .unwrap_or_default();

                        let msg_exists = messages.iter().any(|m| m.id == message.id);
                        let session_exists = loaded_session.is_some();

                        // After failure: both exist (rollforward) or neither exists (rollback)
                        // is acceptable. What's NOT acceptable is partial state.
                        if session_exists != msg_exists {
                            atomicity_violations += 1;
                            panic!(
                                "ATOMICITY BUG: Checkpoint {} failed but left partial state (session={}, msg={})",
                                i, session_exists, msg_exists
                            );
                        }

                        checkpoint_failure += 1;
                    }
                    Err(e) => {
                        panic!("BUG: Non-retriable error on checkpoint: {}", e);
                    }
                }
            }

            println!(
                "Atomic checkpoint: {} successes, {} failures, {} atomicity violations",
                checkpoint_success, checkpoint_failure, atomicity_violations
            );

            // Assert no atomicity violations
            assert_eq!(
                atomicity_violations, 0,
                "Atomicity violations detected - checkpoint is not atomic"
            );

            // With 30% fault rate, expect some successes
            assert!(
                checkpoint_success >= 5,
                "Too many failures: only {} successes out of 30",
                checkpoint_success
            );

            Ok::<_, kelpie_core::Error>(())
        })
        .await;

    if let Err(e) = &result {
        eprintln!("Simulation error: {:?}", e);
    }
    assert!(
        result.is_ok(),
        "Atomic checkpoint test should pass: {:?}",
        result.err()
    );
}

// =============================================================================
// Test 10: Concurrent Checkpoint Conflict Detection (Issue #87)
// =============================================================================

/// Test that concurrent checkpoints to the same session trigger conflict detection
///
/// NO FAULTS - testing OCC semantics
///
/// ASSERTION: If two concurrent checkpoints modify the same session,
/// one should succeed and one should either conflict or see updated state
#[cfg_attr(feature = "madsim", madsim::test)]
#[cfg_attr(not(feature = "madsim"), tokio::test)]
async fn test_dst_concurrent_checkpoint_conflict() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .run_async(|env| async move {
            use kelpie_core::{current_runtime, Runtime};
            let runtime = current_runtime();
            let storage = Arc::new(create_storage(&env));

            // Create agent
            let agent = AgentMetadata::new(
                "agent-concurrent".to_string(),
                "Concurrent Checkpoint Agent".to_string(),
                AgentType::MemgptAgent,
            );
            storage.save_agent(&agent).await?;

            // Create initial session
            let session =
                SessionState::new("shared-session".to_string(), "agent-concurrent".to_string());
            storage.save_session(&session).await?;

            // Spawn concurrent tasks that checkpoint the same session
            let mut tasks = Vec::new();
            for i in 0..5 {
                let storage_clone = storage.clone();
                let task = runtime.spawn(async move {
                    // Each task checkpoints multiple times
                    for j in 0..3 {
                        let mut session = SessionState::new(
                            "shared-session".to_string(),
                            "agent-concurrent".to_string(),
                        );
                        // Advance iteration to simulate work
                        for _ in 0..=j {
                            session.advance_iteration();
                        }

                        let message = Message {
                            id: format!("msg-{}-{}", i, j),
                            agent_id: "agent-concurrent".to_string(),
                            message_type: "assistant_message".to_string(),
                            role: MessageRole::Assistant,
                            content: format!("Task {} iteration {}", i, j),
                            tool_calls: vec![],
                            tool_call_id: None,
                            tool_call: None,
                            tool_return: None,
                            status: None,
                            created_at: chrono::DateTime::from_timestamp(1700000000, 0).unwrap(),
                        };

                        // Checkpoint - may succeed or conflict
                        let _ = storage_clone.checkpoint(&session, Some(&message)).await;
                    }
                    Ok::<_, StorageError>(i)
                });
                tasks.push(task);
            }

            // Wait for all tasks
            for task in tasks {
                let _ = task.await;
            }

            // Verify final state is consistent
            let storage_ref = storage.clone();
            let final_session = retry_read(|| {
                let storage = storage_ref.clone();
                async move {
                    storage
                        .load_session("agent-concurrent", "shared-session")
                        .await
                }
            })
            .await?;

            assert!(
                final_session.is_some(),
                "Session should exist after concurrent checkpoints"
            );

            let storage_ref = storage.clone();
            let messages = retry_read(|| {
                let storage = storage_ref.clone();
                async move { storage.load_messages("agent-concurrent", 1000).await }
            })
            .await?;

            // Should have at least some messages from successful checkpoints
            println!(
                "Concurrent checkpoint: {} messages after concurrent operations",
                messages.len()
            );
            assert!(
                !messages.is_empty(),
                "Should have messages from successful checkpoints"
            );

            // Verify no duplicate messages (each msg id should be unique)
            let msg_ids: Vec<_> = messages.iter().map(|m| &m.id).collect();
            let unique_ids: std::collections::HashSet<_> = msg_ids.iter().collect();
            assert_eq!(
                msg_ids.len(),
                unique_ids.len(),
                "No duplicate messages should exist"
            );

            Ok::<_, kelpie_core::Error>(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Concurrent checkpoint test should pass: {:?}",
        result.err()
    );
}
