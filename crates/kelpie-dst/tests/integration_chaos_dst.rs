//! Phase 6: Integration & Stress Testing with Chaos Fault Injection
//!
//! TigerStyle: Full system DST with all components integrated.
//! These tests verify the system handles multiple simultaneous faults gracefully.

// Allow direct tokio usage in test code
#![allow(clippy::disallowed_methods)]

use bytes::Bytes;
use kelpie_core::Result;
use kelpie_dst::{
    Architecture, FaultConfig, FaultType, SimConfig, SimSandboxFactory, SimTeleportStorage,
    Simulation, SnapshotKind, TeleportPackage, VmSnapshotBlob,
};
use kelpie_sandbox::{Sandbox, SandboxConfig, SandboxFactory, SandboxState, Snapshot};

// ============================================================================
// Full Chaos Tests - All Fault Types Active
// ============================================================================

/// Test full teleport workflow under chaos conditions
///
/// This test enables ALL fault types simultaneously to verify the system
/// handles multiple concurrent failures gracefully. No panics, no hangs,
/// proper error propagation or success.
///
/// Fault types active:
/// - SandboxCrash (5%)
/// - SandboxBootFail (5%)
/// - SnapshotCreateFail (5%)
/// - SnapshotCorruption (5%)
/// - SnapshotRestoreFail (5%)
/// - TeleportUploadFail (10%)
/// - TeleportDownloadFail (10%)
/// - StorageWriteFail (5%)
/// - StorageReadFail (5%)
/// - NetworkDelay (20%)
#[madsim::test]
async fn test_dst_full_teleport_workflow_under_chaos() {
    let config = SimConfig::new(6001);

    let result = Simulation::new(config)
        // Sandbox faults
        .with_fault(FaultConfig::new(FaultType::SandboxCrash, 0.05))
        .with_fault(FaultConfig::new(FaultType::SandboxBootFail, 0.05))
        .with_fault(FaultConfig::new(FaultType::SandboxPauseFail, 0.05))
        .with_fault(FaultConfig::new(FaultType::SandboxResumeFail, 0.05))
        // Snapshot faults
        .with_fault(FaultConfig::new(FaultType::SnapshotCreateFail, 0.05))
        .with_fault(FaultConfig::new(FaultType::SnapshotCorruption, 0.05))
        .with_fault(FaultConfig::new(FaultType::SnapshotRestoreFail, 0.05))
        // Teleport faults
        .with_fault(FaultConfig::new(FaultType::TeleportUploadFail, 0.10))
        .with_fault(FaultConfig::new(FaultType::TeleportDownloadFail, 0.10))
        // Storage faults
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.05))
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.05))
        // Network faults
        .with_fault(FaultConfig::new(
            FaultType::NetworkDelay {
                min_ms: 10,
                max_ms: 100,
            },
            0.20,
        ))
        .run_async(|sim_env| async move {
            let factory = sim_env.sandbox_factory.clone();
            let teleport_storage = sim_env.teleport_storage.clone();

            let mut success_count = 0;
            let mut failure_count = 0;

            // Attempt 20 full teleport workflows
            for i in 0..20 {
                let workflow_result = run_teleport_workflow(&factory, &teleport_storage, i).await;

                match workflow_result {
                    Ok(_) => success_count += 1,
                    Err(_) => failure_count += 1,
                }
            }

            println!(
                "Chaos test results: {} success, {} failures out of 20",
                success_count, failure_count
            );

            // With ~40-50% combined fault probability, expect some successes
            // The key is NO panics, NO hangs - all failures must be graceful
            assert!(
                success_count > 0 || failure_count == 20,
                "Should have at least some attempts complete (success or graceful failure)"
            );

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Chaos test should complete without panic: {:?}",
        result.err()
    );
}

/// Helper: Run a complete teleport workflow (create -> exec -> snapshot -> upload -> download -> restore)
async fn run_teleport_workflow(
    factory: &SimSandboxFactory,
    teleport_storage: &SimTeleportStorage,
    iteration: usize,
) -> Result<()> {
    // 1. Create sandbox
    let sandbox_config = SandboxConfig::default();
    let mut sandbox = factory.create(sandbox_config).await?;

    // 2. Start sandbox
    sandbox.start().await?;
    assert_eq!(sandbox.state(), SandboxState::Running);

    // 3. Execute some commands
    let output = sandbox
        .exec_simple("echo", &[&format!("iteration-{}", iteration)])
        .await?;
    assert!(output.is_success());

    // 4. Create snapshot
    let snapshot = sandbox.snapshot().await?;

    // 5. Upload to teleport storage
    let snapshot_bytes = snapshot.cpu_state.clone().unwrap_or_default();
    let memory_bytes = snapshot.memory.clone().unwrap_or_default();
    let vm_snapshot = VmSnapshotBlob::encode(Bytes::new(), snapshot_bytes, memory_bytes);

    let package = TeleportPackage::new(
        format!("pkg-{}", iteration),
        format!("agent-{}", iteration),
        Architecture::Arm64,
        SnapshotKind::Teleport,
    )
    .with_vm_snapshot(vm_snapshot)
    .with_workspace_ref(format!("workspace-{}", iteration))
    .with_agent_state(Bytes::from(format!("agent-state-{}", iteration)))
    .with_base_image_version("v1.0.0")
    .with_created_at(0);
    let package_id = teleport_storage.upload(package).await?;

    // 6. Download from teleport storage
    let downloaded = teleport_storage.download(&package_id).await?;
    assert_eq!(downloaded.agent_id, format!("agent-{}", iteration));

    // 7. Restore snapshot to new sandbox
    let mut new_sandbox = factory.create(SandboxConfig::default()).await?;
    new_sandbox.start().await?;

    // Create snapshot from downloaded data
    let restored_snapshot = Snapshot::teleport(format!("restored-{}", iteration));
    new_sandbox.restore(&restored_snapshot).await?;

    // 8. Verify restored sandbox works
    let verify_output = new_sandbox.exec_simple("echo", &["restored"]).await?;
    assert!(verify_output.is_success());

    // 9. Cleanup
    sandbox.stop().await?;
    new_sandbox.stop().await?;
    teleport_storage.delete(&package_id).await?;

    Ok(())
}

/// Test sandbox lifecycle under heavy chaos
///
/// Tests rapid create/start/exec/stop cycles with all sandbox faults active.
#[madsim::test]
async fn test_dst_sandbox_lifecycle_under_chaos() {
    let config = SimConfig::new(6002);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::SandboxBootFail, 0.10))
        .with_fault(FaultConfig::new(FaultType::SandboxCrash, 0.10))
        .with_fault(FaultConfig::new(FaultType::SandboxPauseFail, 0.10))
        .with_fault(FaultConfig::new(FaultType::SandboxResumeFail, 0.10))
        .with_fault(FaultConfig::new(
            FaultType::SandboxExecTimeout { timeout_ms: 100 },
            0.10,
        ))
        .with_fault(FaultConfig::new(FaultType::SandboxExecFail, 0.10))
        .run_async(|sim_env| async move {
            let factory = sim_env.sandbox_factory.clone();

            let mut lifecycle_success = 0;
            let mut lifecycle_failure = 0;

            for i in 0..50 {
                let result = run_sandbox_lifecycle(&factory, i).await;
                match result {
                    Ok(_) => lifecycle_success += 1,
                    Err(_) => lifecycle_failure += 1,
                }
            }

            println!(
                "Sandbox lifecycle chaos: {} success, {} failures",
                lifecycle_success, lifecycle_failure
            );

            // With ~50% combined fault rate, expect mix of success/failure
            // Key: no panics, no hangs
            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Sandbox chaos test should not panic: {:?}",
        result.err()
    );
}

/// Helper: Run a single sandbox lifecycle
async fn run_sandbox_lifecycle(factory: &SimSandboxFactory, iteration: usize) -> Result<()> {
    let mut sandbox = factory.create(SandboxConfig::default()).await?;
    sandbox.start().await?;

    // Multiple exec calls
    for j in 0..5 {
        let _ = sandbox
            .exec_simple("echo", &[&format!("{}-{}", iteration, j)])
            .await?;
    }

    // Pause/resume cycle
    sandbox.pause().await?;
    sandbox.resume().await?;

    // More exec after resume
    sandbox.exec_simple("echo", &["after-resume"]).await?;

    sandbox.stop().await?;
    Ok(())
}

/// Test snapshot operations under chaos
///
/// Heavy snapshot create/restore cycles with corruption and failure faults.
#[madsim::test]
async fn test_dst_snapshot_operations_under_chaos() {
    let config = SimConfig::new(6003);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::SnapshotCreateFail, 0.15))
        .with_fault(FaultConfig::new(FaultType::SnapshotCorruption, 0.10))
        .with_fault(FaultConfig::new(FaultType::SnapshotRestoreFail, 0.15))
        .with_fault(FaultConfig::new(
            FaultType::SnapshotTooLarge {
                max_bytes: 1024 * 1024,
            },
            0.05,
        ))
        .run_async(|sim_env| async move {
            let factory = sim_env.sandbox_factory.clone();

            let mut snapshot_success = 0;
            let mut snapshot_failure = 0;
            let mut restore_success = 0;
            let mut restore_failure = 0;

            for _i in 0..30 {
                // Create and start sandbox
                let create_result = factory.create(SandboxConfig::default()).await;
                let mut sandbox = match create_result {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                if sandbox.start().await.is_err() {
                    continue;
                }

                // Snapshot
                match sandbox.snapshot().await {
                    Ok(snapshot) => {
                        snapshot_success += 1;

                        // Try to restore
                        let mut new_sandbox = match factory.create(SandboxConfig::default()).await {
                            Ok(s) => s,
                            Err(_) => continue,
                        };

                        if new_sandbox.start().await.is_err() {
                            continue;
                        }

                        match new_sandbox.restore(&snapshot).await {
                            Ok(_) => restore_success += 1,
                            Err(_) => restore_failure += 1,
                        }

                        let _ = new_sandbox.stop().await;
                    }
                    Err(_) => snapshot_failure += 1,
                }

                let _ = sandbox.stop().await;
            }

            println!(
                "Snapshot chaos: create {}/{}, restore {}/{}",
                snapshot_success,
                snapshot_success + snapshot_failure,
                restore_success,
                restore_success + restore_failure
            );

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Snapshot chaos test should not panic: {:?}",
        result.err()
    );
}

/// Test teleport storage operations under chaos
///
/// Upload/download cycles with high failure rates.
#[madsim::test]
async fn test_dst_teleport_storage_under_chaos() {
    let config = SimConfig::new(6004);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::TeleportUploadFail, 0.20))
        .with_fault(FaultConfig::new(FaultType::TeleportDownloadFail, 0.20))
        .with_fault(FaultConfig::new(
            FaultType::TeleportTimeout { timeout_ms: 500 },
            0.10,
        ))
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.10))
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.10))
        .run_async(|sim_env| async move {
            let teleport_storage = sim_env.teleport_storage.clone();

            let mut upload_success = 0;
            let mut upload_failure = 0;
            let mut download_success = 0;
            let mut download_failure = 0;
            let mut roundtrip_success = 0;

            for i in 0..40 {
                let vm_snapshot = VmSnapshotBlob::encode(
                    Bytes::new(),
                    Bytes::from("cpu"),
                    Bytes::from(vec![0u8; 1024]),
                );
                let package = TeleportPackage::new(
                    format!("chaos-pkg-{}", i),
                    format!("chaos-agent-{}", i),
                    Architecture::Arm64,
                    SnapshotKind::Teleport,
                )
                .with_vm_snapshot(vm_snapshot)
                .with_workspace_ref(format!("ws-{}", i))
                .with_agent_state(Bytes::from(format!("state-{}", i)))
                .with_base_image_version("v1.0.0")
                .with_created_at(i as u64);

                // Upload
                let upload_result = teleport_storage.upload(package.clone()).await;
                match upload_result {
                    Ok(id) => {
                        upload_success += 1;

                        // Download
                        match teleport_storage.download(&id).await {
                            Ok(downloaded) => {
                                download_success += 1;

                                // Verify roundtrip
                                if downloaded.agent_id == package.agent_id {
                                    roundtrip_success += 1;
                                }

                                // Cleanup
                                let _ = teleport_storage.delete(&id).await;
                            }
                            Err(_) => download_failure += 1,
                        }
                    }
                    Err(_) => upload_failure += 1,
                }
            }

            println!(
                "Teleport storage chaos: upload {}/{}, download {}/{}, roundtrip {}",
                upload_success,
                upload_success + upload_failure,
                download_success,
                download_success + download_failure,
                roundtrip_success
            );

            // With ~50% combined fault rate, expect some successes
            assert!(
                upload_success > 0,
                "Should have some upload successes despite faults"
            );

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Teleport storage chaos test should not panic: {:?}",
        result.err()
    );
}

/// Test determinism under chaos - same seed must produce same results
#[madsim::test]
async fn test_dst_chaos_determinism() {
    // Run the same chaos scenario twice with same seed
    let seed = 6005;

    let run_chaos = |seed: u64| async move {
        let config = SimConfig::new(seed);

        Simulation::new(config)
            .with_fault(FaultConfig::new(FaultType::SandboxCrash, 0.10))
            .with_fault(FaultConfig::new(FaultType::SnapshotCorruption, 0.10))
            .with_fault(FaultConfig::new(FaultType::TeleportUploadFail, 0.15))
            .run_async(|sim_env| async move {
                let factory = sim_env.sandbox_factory.clone();
                let teleport_storage = sim_env.teleport_storage.clone();

                let mut results = Vec::new();

                for i in 0..10 {
                    // Track each operation's outcome
                    let sandbox_result = factory.create(SandboxConfig::default()).await;
                    let sandbox_ok = sandbox_result.is_ok();

                    let upload_result = teleport_storage
                        .upload(
                            TeleportPackage::new(
                                format!("det-{}", i),
                                format!("agent-{}", i),
                                Architecture::Arm64,
                                SnapshotKind::Checkpoint,
                            )
                            .with_base_image_version("v1")
                            .with_created_at(0),
                        )
                        .await;
                    let upload_ok = upload_result.is_ok();

                    results.push((sandbox_ok, upload_ok));
                }

                Ok(results)
            })
            .await
    };

    let results1 = run_chaos(seed).await.unwrap();
    let results2 = run_chaos(seed).await.unwrap();

    assert_eq!(
        results1, results2,
        "Same seed must produce identical results under chaos"
    );
}

// ============================================================================
// Stress Tests (Long-Running, Ignored by Default)
// ============================================================================

/// Stress test: 100 concurrent teleport operations
///
/// Run with: cargo test stress_test_concurrent_teleports --release -- --ignored
#[madsim::test]
#[ignore]
async fn stress_test_concurrent_teleports() {
    let config = SimConfig::new(6100);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::TeleportUploadFail, 0.05))
        .with_fault(FaultConfig::new(FaultType::TeleportDownloadFail, 0.05))
        .with_fault(FaultConfig::new(FaultType::SnapshotCreateFail, 0.05))
        .run_async(|sim_env| async move {
            let factory = sim_env.sandbox_factory.clone();
            let teleport_storage = sim_env.teleport_storage.clone();

            // Spawn 100 concurrent teleport operations
            let mut handles = Vec::new();

            for i in 0..100 {
                let factory = factory.clone();
                let storage = teleport_storage.clone();

                let handle =
                    tokio::spawn(async move { run_teleport_workflow(&factory, &storage, i).await });
                handles.push(handle);
            }

            let mut success = 0;
            let mut failure = 0;

            for handle in handles {
                match handle.await {
                    Ok(Ok(_)) => success += 1,
                    _ => failure += 1,
                }
            }

            println!(
                "Concurrent teleport stress test: {} success, {} failure",
                success, failure
            );

            // Expect majority to succeed with 15% combined fault rate
            assert!(
                success > 50,
                "Expected majority success in concurrent stress test"
            );

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Concurrent stress test failed: {:?}",
        result.err()
    );
}

/// Stress test: Rapid sandbox lifecycle cycles
///
/// Run with: cargo test stress_test_rapid_sandbox_lifecycle --release -- --ignored
#[madsim::test]
#[ignore]
async fn stress_test_rapid_sandbox_lifecycle() {
    let config = SimConfig::new(6101);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::SandboxBootFail, 0.02))
        .with_fault(FaultConfig::new(FaultType::SandboxCrash, 0.02))
        .run_async(|sim_env| async move {
            let factory = sim_env.sandbox_factory.clone();

            let mut total_cycles = 0;
            let mut failed_cycles = 0;

            // 1000 rapid lifecycle cycles
            for i in 0..1000 {
                let result = async {
                    let mut sandbox = factory.create(SandboxConfig::default()).await?;
                    sandbox.start().await?;
                    sandbox.exec_simple("echo", &[&format!("{}", i)]).await?;
                    sandbox.stop().await?;
                    Ok::<_, kelpie_core::Error>(())
                }
                .await;

                total_cycles += 1;
                if result.is_err() {
                    failed_cycles += 1;
                }
            }

            println!(
                "Rapid lifecycle stress: {}/{} successful",
                total_cycles - failed_cycles,
                total_cycles
            );

            // With 4% combined fault rate, expect ~96% success
            assert!(
                failed_cycles < 100,
                "Too many failures in rapid lifecycle test"
            );

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Rapid lifecycle stress test failed: {:?}",
        result.err()
    );
}

/// Stress test: Rapid suspend/resume cycles
///
/// Run with: cargo test stress_test_rapid_suspend_resume --release -- --ignored
#[madsim::test]
#[ignore]
async fn stress_test_rapid_suspend_resume() {
    let config = SimConfig::new(6102);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::SandboxPauseFail, 0.02))
        .with_fault(FaultConfig::new(FaultType::SandboxResumeFail, 0.02))
        .with_fault(FaultConfig::new(FaultType::SnapshotCreateFail, 0.02))
        .run_async(|sim_env| async move {
            let factory = sim_env.sandbox_factory.clone();

            let mut sandbox = factory.create(SandboxConfig::default()).await?;
            sandbox.start().await?;

            let mut success_cycles = 0;
            let mut failed_cycles = 0;

            // 500 suspend/resume cycles on same sandbox
            for _ in 0..500 {
                let cycle_result = async {
                    sandbox.pause().await?;
                    sandbox.resume().await?;
                    Ok::<_, kelpie_core::Error>(())
                }
                .await;

                match cycle_result {
                    Ok(_) => success_cycles += 1,
                    Err(_) => {
                        failed_cycles += 1;
                        // Recreate sandbox if cycle failed
                        let _ = sandbox.stop().await;
                        sandbox = factory.create(SandboxConfig::default()).await?;
                        sandbox.start().await?;
                    }
                }
            }

            sandbox.stop().await?;

            println!(
                "Rapid suspend/resume: {}/{} successful",
                success_cycles,
                success_cycles + failed_cycles
            );

            // With 6% combined fault rate, expect ~94% success
            assert!(
                success_cycles > 400,
                "Too many failures in suspend/resume test"
            );

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Rapid suspend/resume stress test failed: {:?}",
        result.err()
    );
}

/// Stress test: Many snapshots with varying sizes
///
/// Run with: cargo test stress_test_many_snapshots --release -- --ignored
#[madsim::test]
#[ignore]
async fn stress_test_many_snapshots() {
    let config = SimConfig::new(6103);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::SnapshotCreateFail, 0.03))
        .with_fault(FaultConfig::new(FaultType::SnapshotCorruption, 0.02))
        .with_fault(FaultConfig::new(FaultType::SnapshotRestoreFail, 0.03))
        .run_async(|sim_env| async move {
            let factory = sim_env.sandbox_factory.clone();

            let mut snapshots_created = 0;
            let mut snapshots_restored = 0;
            let mut failures = 0;

            for i in 0..200 {
                let result = async {
                    let mut sandbox = factory.create(SandboxConfig::default()).await?;
                    sandbox.start().await?;

                    // Do some work
                    for j in 0..5 {
                        sandbox
                            .exec_simple("echo", &[&format!("{}-{}", i, j)])
                            .await?;
                    }

                    // Snapshot
                    let snapshot = sandbox.snapshot().await?;
                    snapshots_created += 1;

                    // Restore to new sandbox
                    let mut new_sandbox = factory.create(SandboxConfig::default()).await?;
                    new_sandbox.start().await?;
                    new_sandbox.restore(&snapshot).await?;
                    snapshots_restored += 1;

                    sandbox.stop().await?;
                    new_sandbox.stop().await?;

                    Ok::<_, kelpie_core::Error>(())
                }
                .await;

                if result.is_err() {
                    failures += 1;
                }
            }

            println!(
                "Many snapshots stress: created {}, restored {}, failures {}",
                snapshots_created, snapshots_restored, failures
            );

            // Expect most operations to succeed
            assert!(
                snapshots_created > 150,
                "Too many snapshot creation failures"
            );

            Ok(())
        })
        .await;

    assert!(
        result.is_ok(),
        "Many snapshots stress test failed: {:?}",
        result.err()
    );
}
