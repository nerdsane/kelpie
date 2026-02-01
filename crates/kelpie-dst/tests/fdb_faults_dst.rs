//! FoundationDB-Critical Fault Types DST Tests (Issue #36)
//!
//! TigerStyle: DST tests for production-critical fault types.
//! These tests verify the new fault types work correctly in full simulation.

#![allow(clippy::disallowed_methods)]

use bytes::Bytes;
use kelpie_dst::{
    DeterministicRng, FaultConfig, FaultInjectorBuilder, FaultType, SimConfig, Simulation,
};

// ============================================================================
// Storage Semantics Fault Tests
// ============================================================================

/// Test that misdirected writes send data to wrong location silently
#[madsim::test]
async fn test_dst_storage_misdirected_write_simulation() {
    let config = SimConfig::new(42);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::StorageMisdirectedWrite {
                target_key: b"__corrupted__".to_vec(),
            },
            0.5, // 50% of writes go to wrong location
        ))
        .run_async(|env| async move {
            // Write multiple values
            for i in 0..10 {
                let key = format!("key-{}", i);
                let value = format!("value-{}", i);
                env.storage.write(key.as_bytes(), value.as_bytes()).await?;
            }

            // Count how many ended up at their intended location
            let mut correct_count = 0;
            for i in 0..10 {
                let key = format!("key-{}", i);
                let expected = format!("value-{}", i);
                if let Some(value) = env.storage.read(key.as_bytes()).await? {
                    if value.as_ref() == expected.as_bytes() {
                        correct_count += 1;
                    }
                }
            }

            // With 50% misdirection, expect some to be misdirected
            // CRITICAL: Assert faults actually occurred
            assert!(
                correct_count < 10,
                "Some writes should be misdirected with 50% fault rate, but all {} were correct",
                correct_count
            );

            // Check that misdirected data ended up somewhere
            let misdirected = env.storage.read(b"__corrupted__").await?;
            assert!(
                misdirected.is_some(),
                "Misdirected data should exist at __corrupted__ key"
            );

            println!(
                "✅ Misdirected write test: {} of 10 correct, misdirected data found",
                correct_count
            );

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Simulation should complete: {:?}", result);
}

/// Test partial writes truncate data silently
#[madsim::test]
async fn test_dst_storage_partial_write_simulation() {
    let config = SimConfig::new(42);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::StoragePartialWrite { bytes_written: 5 },
            0.5, // 50% of writes are truncated
        ))
        .run_async(|env| async move {
            let full_value = b"hello_world_this_is_a_long_value";

            // Try multiple writes
            for i in 0..10 {
                let key = format!("partial-{}", i);
                env.storage.write(key.as_bytes(), full_value).await.ok();
            }

            // Count truncated vs full writes
            let mut truncated = 0;
            let mut full = 0;
            for i in 0..10 {
                let key = format!("partial-{}", i);
                if let Some(value) = env.storage.read(key.as_bytes()).await? {
                    if value.len() < full_value.len() {
                        truncated += 1;
                    } else {
                        full += 1;
                    }
                }
            }

            // CRITICAL: Assert faults actually occurred
            assert!(
                truncated > 0,
                "Some writes should be truncated with 50% fault rate, but got {} truncated",
                truncated
            );

            println!(
                "✅ Partial write test: {} truncated, {} full out of 10",
                truncated, full
            );

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Simulation should complete: {:?}", result);
}

/// Test unflushed loss - writes appear successful but data is lost
#[madsim::test]
async fn test_dst_storage_unflushed_loss_simulation() {
    let config = SimConfig::new(42);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageUnflushedLoss, 0.5))
        .run_async(|env| async move {
            // Write values
            for i in 0..10 {
                let key = format!("volatile-{}", i);
                let result = env.storage.write(key.as_bytes(), b"important_data").await;
                // All writes should "succeed" (return Ok)
                assert!(result.is_ok(), "Unflushed loss should appear successful");
            }

            // Count how many actually persisted
            let mut persisted = 0;
            for i in 0..10 {
                let key = format!("volatile-{}", i);
                if env.storage.read(key.as_bytes()).await?.is_some() {
                    persisted += 1;
                }
            }

            // CRITICAL: Assert faults actually occurred - some data should be lost
            assert!(
                persisted < 10,
                "Some writes should be lost with 50% fault rate, but all {} persisted",
                persisted
            );

            println!(
                "✅ Unflushed loss test: {} of 10 writes actually persisted ({} lost)",
                persisted,
                10 - persisted
            );

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Simulation should complete: {:?}", result);
}

// ============================================================================
// Network Fault Tests
// ============================================================================

/// Test packet corruption corrupts data in transit
#[madsim::test]
async fn test_dst_network_packet_corruption_simulation() {
    let config = SimConfig::new(42).with_network_latency(0, 0);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::NetworkPacketCorruption {
                corruption_rate: 0.5, // 50% of bytes corrupted
            },
            0.5, // 50% of packets corrupted
        ))
        .run_async(|env| async move {
            let original = Bytes::from("sensitive_data_that_should_not_be_corrupted");

            // Send multiple messages
            let mut corrupted_count = 0;
            for i in 0..20 {
                env.network
                    .send("sender", &format!("receiver-{}", i), original.clone())
                    .await;
            }

            // Check received messages
            for i in 0..20 {
                let msgs = env.network.receive(&format!("receiver-{}", i)).await;
                for msg in msgs {
                    if msg.payload != original {
                        corrupted_count += 1;
                    }
                }
            }

            // CRITICAL: Assert faults actually occurred
            assert!(
                corrupted_count > 0,
                "Some packets should be corrupted with 50% fault rate, but got {} corrupted",
                corrupted_count
            );

            println!(
                "✅ Packet corruption test: {} of 20 messages were corrupted",
                corrupted_count
            );

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Simulation should complete: {:?}", result);
}

/// Test network jitter adds variable latency
#[madsim::test]
async fn test_dst_network_jitter_simulation() {
    let config = SimConfig::new(42).with_network_latency(10, 0);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::NetworkJitter {
                mean_ms: 100,
                stddev_ms: 50,
            },
            1.0, // Always apply jitter
        ))
        .run_async(|env| async move {
            // Send messages to different receivers
            for i in 0..10 {
                env.network
                    .send(
                        "sender",
                        &format!("receiver-{}", i),
                        Bytes::from(format!("msg-{}", i)),
                    )
                    .await;
            }

            // Verify messages were sent (jitter doesn't prevent sending)
            // We can check pending counts to verify messages are queued
            let mut total_pending = 0;
            for i in 0..10 {
                total_pending += env.network.pending_count(&format!("receiver-{}", i)).await;
            }

            println!(
                "Network jitter test: {} messages queued with variable delivery times",
                total_pending
            );
            assert_eq!(total_pending, 10, "All messages should be queued");

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Simulation should complete: {:?}", result);
}

/// Test connection exhaustion drops messages
#[madsim::test]
async fn test_dst_network_connection_exhaustion_simulation() {
    let config = SimConfig::new(42).with_network_latency(0, 0);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::NetworkConnectionExhaustion,
            0.3, // 30% of connections exhausted
        ))
        .run_async(|env| async move {
            // Try to send many messages
            let mut sent = 0;
            let mut dropped = 0;
            for i in 0..20 {
                if env
                    .network
                    .send("sender", "receiver", Bytes::from(format!("msg-{}", i)))
                    .await
                {
                    sent += 1;
                } else {
                    dropped += 1;
                }
            }

            // CRITICAL: Assert faults actually occurred
            assert!(
                dropped > 0,
                "Some connections should be exhausted with 30% fault rate, but got {} dropped",
                dropped
            );

            println!(
                "✅ Connection exhaustion test: {} sent, {} dropped out of 20",
                sent, dropped
            );

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Simulation should complete: {:?}", result);
}

// ============================================================================
// Combined Chaos Test
// ============================================================================

/// Test all new fault types simultaneously
#[madsim::test]
async fn test_dst_fdb_faults_chaos() {
    let config = SimConfig::new(42).with_network_latency(5, 0);

    let result = Simulation::new(config)
        // Storage semantics faults
        .with_fault(FaultConfig::new(
            FaultType::StorageMisdirectedWrite {
                target_key: b"__misdirected__".to_vec(),
            },
            0.05,
        ))
        .with_fault(FaultConfig::new(
            FaultType::StoragePartialWrite { bytes_written: 10 },
            0.05,
        ))
        .with_fault(FaultConfig::new(FaultType::StorageFsyncFail, 0.05))
        .with_fault(FaultConfig::new(FaultType::StorageUnflushedLoss, 0.05))
        // Network faults
        .with_fault(FaultConfig::new(
            FaultType::NetworkPacketCorruption {
                corruption_rate: 0.1,
            },
            0.10,
        ))
        .with_fault(FaultConfig::new(
            FaultType::NetworkJitter {
                mean_ms: 50,
                stddev_ms: 25,
            },
            0.20,
        ))
        .with_fault(FaultConfig::new(
            FaultType::NetworkConnectionExhaustion,
            0.05,
        ))
        .run_async(|env| async move {
            // Perform a mix of storage and network operations
            let mut storage_errors = 0;
            let mut network_failures = 0;

            for i in 0..50 {
                // Storage operations
                let write_result = env
                    .storage
                    .write(format!("key-{}", i).as_bytes(), b"value")
                    .await;
                if write_result.is_err() {
                    storage_errors += 1;
                }

                // Network operations
                if !env
                    .network
                    .send("node-1", "node-2", Bytes::from(format!("msg-{}", i)))
                    .await
                {
                    network_failures += 1;
                }
            }

            // CRITICAL: With 50 operations and various fault rates, SOMETHING should fail
            // This verifies fault injection is actually working
            let total_faults = storage_errors + network_failures;
            assert!(
                total_faults > 0,
                "With multiple faults configured, at least one should trigger in 100 operations"
            );

            println!(
                "✅ Chaos test: {} storage errors, {} network failures out of 50 ops each (total faults: {})",
                storage_errors, network_failures, total_faults
            );

            // The key invariant: no panics, no hangs, graceful handling of all faults
            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Chaos simulation should complete without panic: {:?}",
        result
    );
}

// ============================================================================
// Determinism Verification Tests
// ============================================================================

/// Verify that fault injection is deterministic across runs
#[madsim::test]
async fn test_dst_fdb_faults_determinism() {
    let seed = 12345u64;

    // Run simulation twice with same seed
    let run_simulation = || async {
        let config = SimConfig::new(seed);

        Simulation::new(config)
            .with_fault(FaultConfig::new(
                FaultType::StorageMisdirectedWrite {
                    target_key: b"__target__".to_vec(),
                },
                0.5,
            ))
            .with_fault(FaultConfig::new(
                FaultType::NetworkPacketCorruption {
                    corruption_rate: 0.3,
                },
                0.5,
            ))
            .run_async(|env| async move {
                let mut results = Vec::new();

                // Storage operations
                for i in 0..10 {
                    env.storage
                        .write(format!("k{}", i).as_bytes(), b"v")
                        .await
                        .ok();
                    let exists = env
                        .storage
                        .read(format!("k{}", i).as_bytes())
                        .await?
                        .is_some();
                    results.push(exists);
                }

                // Network operations
                for i in 0..10 {
                    let sent = env
                        .network
                        .send("a", "b", Bytes::from(format!("m{}", i)))
                        .await;
                    results.push(sent);
                }

                Ok(results)
            })
            .await
    };

    let result1 = run_simulation().await.unwrap();
    let result2 = run_simulation().await.unwrap();

    assert_eq!(
        result1, result2,
        "Same seed should produce identical results"
    );
}

// ============================================================================
// Builder Helper Tests
// ============================================================================

/// Test the FaultInjectorBuilder helpers for new fault categories
#[test]
fn test_fdb_fault_builder_helpers() {
    let rng = DeterministicRng::new(42);

    // Storage semantics faults
    let injector = FaultInjectorBuilder::new(rng.fork())
        .with_storage_semantics_faults(0.1)
        .build();
    let stats = injector.stats();
    assert_eq!(
        stats.len(),
        4,
        "Should have 4 storage semantics faults: {:?}",
        stats.iter().map(|s| &s.fault_type).collect::<Vec<_>>()
    );

    // Coordination faults
    let injector = FaultInjectorBuilder::new(rng.fork())
        .with_coordination_faults(0.1)
        .build();
    let stats = injector.stats();
    assert_eq!(
        stats.len(),
        3,
        "Should have 3 coordination faults: {:?}",
        stats.iter().map(|s| &s.fault_type).collect::<Vec<_>>()
    );

    // Infrastructure faults
    let injector = FaultInjectorBuilder::new(rng.fork())
        .with_infrastructure_faults(0.1)
        .build();
    let stats = injector.stats();
    assert_eq!(
        stats.len(),
        4,
        "Should have 4 infrastructure faults: {:?}",
        stats.iter().map(|s| &s.fault_type).collect::<Vec<_>>()
    );
}
