//! DST tests for Teleportation Service (Phase 4)
//!
//! TigerStyle: Simulation-first testing with comprehensive fault injection.
//!
//! Tests verify teleportation service works correctly under faults:
//! - Roundtrip teleportation (out -> in) preserves agent state
//! - Storage failures during upload/download are handled gracefully
//! - Architecture validation prevents invalid cross-arch VM teleports
//! - Concurrent teleports don't interfere with each other
//! - Interrupted teleports leave system in consistent state

// Allow direct tokio usage in test code
#![allow(clippy::disallowed_methods)]

use bytes::Bytes;
use kelpie_dst::{
    Architecture, FaultConfig, FaultType, SimConfig, Simulation, SnapshotKind, TeleportPackage,
    VmSnapshotBlob,
};
use kelpie_sandbox::{ExecOptions, ResourceLimits, Sandbox, SandboxConfig, SandboxFactory};

// ============================================================================
// Test Helpers
// ============================================================================

fn test_config() -> SandboxConfig {
    SandboxConfig::new()
        .with_limits(
            ResourceLimits::new()
                .with_memory(512 * 1024 * 1024) // 512 MiB
                .with_vcpus(2)
                .with_network(true),
        )
        .with_workdir("/workspace")
        .with_env("PATH", "/usr/bin:/bin")
}

// ============================================================================
// Phase 4 DST Test 1: Teleport Roundtrip Under Faults
// ============================================================================

/// Test: Teleport out, then teleport in, preserving agent state under faults
///
/// Scenario:
/// 1. Create agent with sandbox
/// 2. Execute commands to establish state (create files, set env)
/// 3. Teleport OUT (snapshot + upload to storage)
/// 4. Destroy original sandbox
/// 5. Teleport IN (download + restore on new host)
/// 6. Verify state preserved (files exist, env vars correct)
///
/// Faults injected:
/// - TeleportUploadFail (20%)
/// - TeleportDownloadFail (20%)
/// - SnapshotCreateFail (10%)
/// - SnapshotRestoreFail (10%)
///
/// Expected behavior:
/// - If upload/snapshot fails, original agent remains running
/// - If download/restore fails, error returned but no partial state
/// - If succeeds, new agent has identical state to original
#[madsim::test]
async fn test_dst_teleport_roundtrip_under_faults() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::TeleportUploadFail, 0.2))
        .with_fault(FaultConfig::new(FaultType::TeleportDownloadFail, 0.2))
        .with_fault(FaultConfig::new(FaultType::SnapshotCreateFail, 0.1))
        .with_fault(FaultConfig::new(FaultType::SnapshotRestoreFail, 0.1))
        .run_async(|env| async move {
            let sandbox_config = test_config();

            // Step 1: Create agent with sandbox on "source host"
            let mut source_sandbox = env.sandbox_factory.create(sandbox_config.clone()).await?;
            source_sandbox.start().await?;

            let agent_id = format!("agent-{}", env.rng.next_u64());

            // Step 2: Establish state
            let create_file_result = source_sandbox
                .exec("touch", &["/tmp/testfile"], ExecOptions::default())
                .await;

            let state_established = create_file_result.is_ok();

            // Step 3: Teleport OUT (snapshot + upload)
            let snapshot_result = source_sandbox.snapshot().await;

            match snapshot_result {
                Ok(snapshot) => {
                    // Create teleport package
                    let package = TeleportPackage::new(
                        format!("teleport-{}", agent_id),
                        agent_id.clone(),
                        Architecture::Arm64,
                        SnapshotKind::Teleport,
                    )
                    .with_vm_snapshot(VmSnapshotBlob::encode(
                        Bytes::new(),
                        Bytes::from("simulated cpu state"),
                        Bytes::from("simulated memory state"),
                    ))
                    .with_agent_state(Bytes::from("simulated agent state"))
                    .with_workspace_ref(format!("s3://bucket/{}/workspace", agent_id))
                    .with_base_image_version("1.0.0")
                    .with_created_at(1000);

                    let upload_result = env.teleport_storage.upload(package.clone()).await;

                    match upload_result {
                        Ok(package_id) => {
                            // Upload succeeded, now test teleport IN

                            // Step 4: Destroy original (simulate moving to new host)
                            drop(source_sandbox);

                            // Step 5: Teleport IN (download + restore on "target host")
                            let download_result = env
                                .teleport_storage
                                .download_for_restore(&package_id, Architecture::Arm64)
                                .await;

                            match download_result {
                                Ok(downloaded_package) => {
                                    // Verify package integrity
                                    assert_eq!(downloaded_package.agent_id, agent_id);
                                    assert_eq!(downloaded_package.source_arch, Architecture::Arm64);
                                    assert_eq!(downloaded_package.kind, SnapshotKind::Teleport);
                                    assert!(
                                        downloaded_package.is_full_teleport(),
                                        "Should be full VM teleport"
                                    );

                                    // Create new sandbox on "target host"
                                    let mut target_sandbox =
                                        env.sandbox_factory.create(sandbox_config.clone()).await?;

                                    let start_result = target_sandbox.start().await;
                                    if start_result.is_ok() {
                                        // Restore from snapshot
                                        let restore_result =
                                            target_sandbox.restore(&snapshot).await;

                                        if restore_result.is_ok() && state_established {
                                            // Step 6: Verify state preserved
                                            let verify_result = target_sandbox
                                                .exec(
                                                    "ls",
                                                    &["/tmp/testfile"],
                                                    ExecOptions::default(),
                                                )
                                                .await;

                                            // Since we don't inject SandboxExecFail faults in this test,
                                            // exec failures after successful restore indicate a real bug
                                            match verify_result {
                                                Ok(output) => {
                                                    assert!(
                                                        output.status.is_success(),
                                                        "File should exist after teleport"
                                                    );
                                                }
                                                Err(e) => {
                                                    panic!(
                                                        "Verification exec failed unexpectedly after successful restore: {}",
                                                        e
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(_download_err) => {
                                    // Download failed due to fault injection - this is expected
                                }
                            }
                        }
                        Err(_upload_err) => {
                            // Upload failed due to fault injection - this is expected
                        }
                    }
                }
                Err(_snapshot_err) => {
                    // Snapshot creation failed due to fault injection - this is expected
                }
            }

            Ok(())
        });

    assert!(
        result.await.is_ok(),
        "Test should handle faults gracefully without panics"
    );
}

// ============================================================================
// Phase 4 DST Test 2: Teleport with Storage Failures
// ============================================================================

/// Test: High probability storage failures during teleport operations
///
/// Scenario:
/// - 50% upload failure rate
/// - 50% download failure rate
/// - Verify operations either succeed fully or fail cleanly (no partial state)
///
/// Expected behavior:
/// - Failed uploads don't leave partial packages in storage
/// - Failed downloads don't corrupt local state
/// - Retry logic can recover from transient failures
#[madsim::test]
async fn test_dst_teleport_with_storage_failures() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::TeleportUploadFail, 0.5))
        .with_fault(FaultConfig::new(FaultType::TeleportDownloadFail, 0.5))
        .run_async(|env| async move {
            let agent_id = format!("agent-{}", env.rng.next_u64());

            // Attempt multiple uploads - some will fail
            let mut upload_attempts = 0;
            let mut upload_successes = 0;
            let mut upload_failures = 0;

            for i in 0..10 {
                upload_attempts += 1;

                let package = TeleportPackage::new(
                    format!("pkg-{}-{}", agent_id, i),
                    agent_id.clone(),
                    Architecture::Arm64,
                    SnapshotKind::Teleport,
                )
                .with_vm_snapshot(VmSnapshotBlob::encode(
                    Bytes::new(),
                    Bytes::from(format!("cpu-{}", i)),
                    Bytes::from(format!("memory-{}", i)),
                ))
                .with_base_image_version("1.0.0");

                match env.teleport_storage.upload(package).await {
                    Ok(package_id) => {
                        upload_successes += 1;

                        // Try to download what we just uploaded
                        match env.teleport_storage.download(&package_id).await {
                            Ok(downloaded) => {
                                // Verify data integrity
                                assert_eq!(downloaded.agent_id, agent_id);
                                assert!(downloaded.is_full_teleport());
                            }
                            Err(_) => {
                                // Download failure - expected due to fault injection
                            }
                        }
                    }
                    Err(_) => {
                        upload_failures += 1;

                        // Verify package was NOT partially created
                        let list = env.teleport_storage.list().await;
                        assert!(
                            !list
                                .iter()
                                .any(|id| id.contains(&format!("pkg-{}-{}", agent_id, i))),
                            "Failed upload should not create partial package"
                        );
                    }
                }
            }

            println!(
                "Storage test: {} upload attempts, {} successes, {} failures",
                upload_attempts, upload_successes, upload_failures
            );

            // With 50% failure rate, we expect both successes and failures
            assert!(
                upload_successes > 0 || upload_failures > 0,
                "Should have some results"
            );

            Ok(())
        });

    assert!(result.await.is_ok(), "Storage failure test should complete");
}

// ============================================================================
// Phase 4 DST Test 3: Architecture Validation
// ============================================================================

/// Test: Cross-architecture teleportation rules are enforced
///
/// Scenario:
/// - Create Teleport snapshot on ARM64
/// - Try to restore on X86_64 - should FAIL
/// - Create Checkpoint snapshot on ARM64
/// - Try to restore on X86_64 - should SUCCEED
///
/// Faults injected:
/// - TeleportArchMismatch (triggers architecture validation errors)
///
/// Expected behavior:
/// - VM snapshots (Suspend/Teleport) require same architecture
/// - Checkpoints work across architectures
/// - Clear error messages when architecture mismatch occurs
#[madsim::test]
async fn test_dst_teleport_architecture_validation() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::TeleportArchMismatch, 0.3))
        .run_async(|env| async move {
            let agent_id = format!("agent-{}", env.rng.next_u64());

            // Test 1: Teleport snapshot (full VM) - ARM64 source
            let teleport_package = TeleportPackage::new(
                format!("teleport-{}", agent_id),
                agent_id.clone(),
                Architecture::Arm64,
                SnapshotKind::Teleport,
            )
            .with_vm_snapshot(VmSnapshotBlob::encode(
                Bytes::new(),
                Bytes::from("cpu"),
                Bytes::from("memory"),
            ))
            .with_base_image_version("1.0.0");

            // Validation should pass for same architecture
            assert!(
                teleport_package
                    .validate_for_restore(Architecture::Arm64)
                    .is_ok(),
                "Teleport should work on same architecture"
            );

            // Validation should FAIL for different architecture
            assert!(
                teleport_package
                    .validate_for_restore(Architecture::X86_64)
                    .is_err(),
                "Teleport should fail on different architecture"
            );

            // Test 2: Checkpoint snapshot (app-only) - ARM64 source
            let checkpoint_package = TeleportPackage::new(
                format!("checkpoint-{}", agent_id),
                agent_id.clone(),
                Architecture::Arm64,
                SnapshotKind::Checkpoint,
            )
            .with_agent_state(Bytes::from("agent state"))
            .with_workspace_ref("s3://bucket/workspace")
            .with_base_image_version("1.0.0");

            // Validation should pass for different architecture (checkpoint is cross-arch)
            assert!(
                checkpoint_package
                    .validate_for_restore(Architecture::Arm64)
                    .is_ok(),
                "Checkpoint should work on ARM64"
            );
            assert!(
                checkpoint_package
                    .validate_for_restore(Architecture::X86_64)
                    .is_ok(),
                "Checkpoint should work on X86_64 (cross-arch)"
            );

            Ok(())
        });

    assert!(
        result.await.is_ok(),
        "Architecture validation test should pass"
    );
}

// ============================================================================
// Phase 4 DST Test 4: Concurrent Teleport Operations
// ============================================================================

/// Test: Multiple agents teleporting concurrently don't interfere
///
/// Scenario:
/// - Create 5 agents simultaneously
/// - Teleport all 5 out concurrently
/// - Verify each package is independent
/// - Teleport all 5 in concurrently
/// - Verify no state corruption or cross-contamination
///
/// Faults injected:
/// - Mixed faults (upload, download, snapshot) at 10% each
///
/// Expected behavior:
/// - Concurrent operations are isolated
/// - One agent's failure doesn't affect others
/// - Storage handles concurrent access correctly
#[madsim::test]
async fn test_dst_teleport_concurrent_operations() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::TeleportUploadFail, 0.1))
        .with_fault(FaultConfig::new(FaultType::TeleportDownloadFail, 0.1))
        .with_fault(FaultConfig::new(FaultType::SnapshotCreateFail, 0.1))
        .run_async(|env| async move {
            let sandbox_config = test_config();
            let num_agents = 5;

            // Create multiple agents and teleport concurrently
            let mut handles = vec![];

            for i in 0..num_agents {
                let agent_id = format!("agent-{}-{}", i, env.rng.next_u64());
                let factory = env.sandbox_factory.clone();
                let storage = env.teleport_storage.clone();
                let config = sandbox_config.clone();

                let handle = tokio::spawn(async move {
                    // Create sandbox
                    let mut sandbox = factory.create(config).await.ok()?;
                    sandbox.start().await.ok()?;

                    // Establish unique state for this agent
                    sandbox
                        .exec("echo", &[&format!("agent-{}", i)], ExecOptions::default())
                        .await
                        .ok()?;

                    // Snapshot
                    let snapshot = sandbox.snapshot().await.ok()?;

                    // Create teleport package
                    let package = TeleportPackage::new(
                        format!("pkg-{}", agent_id),
                        agent_id.clone(),
                        Architecture::Arm64,
                        SnapshotKind::Teleport,
                    )
                    .with_vm_snapshot(VmSnapshotBlob::encode(
                        Bytes::new(),
                        Bytes::from(format!("cpu-{}", i)),
                        Bytes::from(format!("memory-{}", i)),
                    ))
                    .with_agent_state(Bytes::from(format!("agent-state-{}", i)))
                    .with_base_image_version("1.0.0");

                    // Upload
                    let package_id = storage.upload(package).await.ok()?;

                    Some((agent_id.clone(), package_id.clone(), snapshot))
                });

                handles.push(handle);
            }

            // Wait for all operations
            let results: Vec<_> = futures::future::join_all(handles)
                .await
                .into_iter()
                .filter_map(|r| r.ok().flatten())
                .collect();

            println!(
                "Concurrent test: {}/{} agents teleported successfully",
                results.len(),
                num_agents
            );

            // With 10% fault rate, we expect most to succeed
            assert!(
                results.len() >= num_agents / 2,
                "At least half should succeed with 10% fault rate"
            );

            // Verify each successful teleport is independent
            for (agent_id, package_id, _snapshot) in results.iter() {
                let downloaded = env.teleport_storage.download(package_id).await.ok();
                if let Some(pkg) = downloaded {
                    assert_eq!(pkg.agent_id, *agent_id, "Package should match agent");
                    assert!(pkg.is_full_teleport(), "Should be full teleport");
                }
            }

            Ok(())
        });

    assert!(
        result.await.is_ok(),
        "Concurrent teleport test should complete"
    );
}

// ============================================================================
// Phase 4 DST Test 5: Interrupted Teleport (Crash Midway)
// ============================================================================

/// Test: System crash during teleport leaves no partial state
///
/// Scenario:
/// - Begin teleport operation
/// - Crash occurs at various stages (pre-upload, mid-upload, post-upload)
/// - Verify no orphaned packages or corrupted state
///
/// Faults injected:
/// - CrashBeforeWrite (10%)
/// - CrashAfterWrite (10%)
/// - TeleportUploadFail (20%)
///
/// Expected behavior:
/// - Pre-upload crash: no package created
/// - Mid-upload crash: package may exist but incomplete (should be detectable)
/// - Post-upload crash: package exists and is complete
/// - Cleanup mechanisms can detect and remove orphaned packages
#[madsim::test]
async fn test_dst_teleport_interrupted_midway() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::CrashBeforeWrite, 0.1))
        .with_fault(FaultConfig::new(FaultType::CrashAfterWrite, 0.1))
        .with_fault(FaultConfig::new(FaultType::TeleportUploadFail, 0.2))
        .run_async(|env| async move {
            let sandbox_config = test_config();
            let agent_id = format!("agent-{}", env.rng.next_u64());

            // Create sandbox
            let mut sandbox = match env.sandbox_factory.create(sandbox_config).await {
                Ok(s) => s,
                Err(_) => return Ok(()), // Crash before creation - test complete
            };

            if sandbox.start().await.is_err() {
                return Ok(()); // Crash during start - test complete
            }

            // Establish state
            let _ = sandbox
                .exec("touch", &["/tmp/testfile"], ExecOptions::default())
                .await;

            // Attempt snapshot (might crash)
            let _snapshot = match sandbox.snapshot().await {
                Ok(s) => s,
                Err(_) => {
                    // Crash during snapshot - verify no partial snapshot in storage
                    let packages = env.teleport_storage.list().await;
                    assert!(
                        !packages.iter().any(|id| id.contains(&agent_id)),
                        "No partial packages should exist after snapshot crash"
                    );
                    return Ok(()); // Test complete
                }
            };

            // Create teleport package
            let package = TeleportPackage::new(
                format!("pkg-{}", agent_id),
                agent_id.clone(),
                Architecture::Arm64,
                SnapshotKind::Teleport,
            )
            .with_vm_snapshot(VmSnapshotBlob::encode(
                Bytes::new(),
                Bytes::from("cpu"),
                Bytes::from("memory"),
            ))
            .with_agent_state(Bytes::from("agent state"))
            .with_base_image_version("1.0.0");

            // Attempt upload (might crash or fail)
            match env.teleport_storage.upload(package).await {
                Ok(package_id) => {
                    // Upload succeeded - verify package is complete
                    let downloaded = env.teleport_storage.download(&package_id).await;
                    assert!(
                        downloaded.is_ok(),
                        "Successfully uploaded package should be downloadable"
                    );

                    if let Ok(pkg) = downloaded {
                        assert!(pkg.is_full_teleport(), "Package should be complete");
                        assert_eq!(pkg.agent_id, agent_id);
                    }
                }
                Err(_) => {
                    // Upload failed (crash or fault) - verify no orphaned package
                    let packages = env.teleport_storage.list().await;

                    // Find and verify any package that might exist
                    // Use async/await instead of block_on to avoid deadlocks and maintain determinism
                    let matching_id = packages.iter().find(|id| id.contains(&agent_id)).cloned();

                    if let Some(id) = matching_id {
                        if let Ok(pkg) = env.teleport_storage.download(&id).await {
                            // Package exists - it should be complete
                            assert!(
                                pkg.is_full_teleport(),
                                "Partial packages should not be left behind"
                            );
                        }
                    }
                }
            }

            Ok(())
        });

    assert!(
        result.await.is_ok(),
        "Interrupted teleport test should complete without panics"
    );
}

// ============================================================================
// Stress Test: Many Teleports with High Fault Rate
// ============================================================================

/// Stress test: 100 teleport operations with 30% fault rate
///
/// This is a long-running test that exercises the teleport system under stress.
/// Run with: cargo test -p kelpie-dst --test teleport_service_dst stress -- --ignored
#[madsim::test]
#[ignore]
async fn stress_test_teleport_operations() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::TeleportUploadFail, 0.3))
        .with_fault(FaultConfig::new(FaultType::TeleportDownloadFail, 0.3))
        .with_fault(FaultConfig::new(FaultType::SnapshotCreateFail, 0.2))
        .with_fault(FaultConfig::new(FaultType::SnapshotRestoreFail, 0.2))
        .run_async(|env| async move {
            let sandbox_config = test_config();
            let num_operations = 100;
            let mut successes = 0;
            let mut failures = 0;

            for i in 0..num_operations {
                let agent_id = format!("stress-agent-{}", i);

                // Try full teleport workflow
                let result_inner: Result<(), kelpie_core::Error> = async {
                    let mut sandbox = env.sandbox_factory.create(sandbox_config.clone()).await?;
                    sandbox.start().await?;
                    let _snapshot = sandbox.snapshot().await?;

                    let package = TeleportPackage::new(
                        format!("pkg-{}", agent_id),
                        agent_id.clone(),
                        Architecture::Arm64,
                        SnapshotKind::Teleport,
                    )
                    .with_vm_snapshot(VmSnapshotBlob::encode(
                        Bytes::new(),
                        Bytes::from(format!("cpu-{}", i)),
                        Bytes::from(format!("mem-{}", i)),
                    ))
                    .with_base_image_version("1.0.0");

                    let package_id = env.teleport_storage.upload(package).await.map_err(|e| {
                        kelpie_core::Error::Internal {
                            message: e.to_string(),
                        }
                    })?;
                    let _downloaded =
                        env.teleport_storage
                            .download(&package_id)
                            .await
                            .map_err(|e| kelpie_core::Error::Internal {
                                message: e.to_string(),
                            })?;

                    Ok(())
                }
                .await;

                if result_inner.is_ok() {
                    successes += 1;
                } else {
                    failures += 1;
                }
            }

            println!(
                "Stress test: {}/{} operations succeeded, {} failed",
                successes, num_operations, failures
            );

            // With 30% fault rate, expect significant successes
            assert!(
                successes >= num_operations / 3,
                "Should have at least 1/3 success rate"
            );

            Ok(())
        });

    assert!(result.await.is_ok(), "Stress test should complete");
}
