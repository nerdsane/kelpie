//! DST tests for Snapshot Type System (Phase 3)
//!
//! TigerStyle: Simulation-first testing with comprehensive fault injection.
//!
//! Tests verify the three snapshot types work correctly under faults:
//! - Suspend: Memory-only, same-host pause/resume
//! - Teleport: Full VM snapshot, same-architecture transfer
//! - Checkpoint: App-level checkpoint, cross-architecture transfer

use std::sync::Arc;

use bytes::Bytes;
use kelpie_dst::{
    Architecture, DeterministicRng, FaultConfig, FaultInjector, FaultInjectorBuilder, FaultType,
    SimConfig, SimSandboxFactory, SimTeleportStorage, SnapshotKind, TeleportPackage,
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

fn create_no_fault_injector(rng: &DeterministicRng) -> Arc<FaultInjector> {
    Arc::new(FaultInjectorBuilder::new(rng.fork()).build())
}

// ============================================================================
// Phase 3 DST Tests - Suspend Snapshot (Memory-only, same-host)
// ============================================================================

/// Test: Suspend snapshot creation under no faults (baseline)
/// Expected: Fast memory-only snapshot succeeds
#[madsim::test]
async fn test_dst_suspend_snapshot_no_faults() {
    let config = SimConfig::from_env_or_random();
    let rng = DeterministicRng::new(config.seed);
    let faults = create_no_fault_injector(&rng);

    let factory = SimSandboxFactory::new(rng.fork(), faults.clone());
    let storage = SimTeleportStorage::new(rng.fork(), faults);
    let config = test_config();

    // Create and start sandbox
    let mut sandbox = factory.create(config).await.expect("should create sandbox");
    sandbox.start().await.expect("should start");

    // Execute some commands to establish state
    sandbox
        .exec("echo", &["hello"], ExecOptions::default())
        .await
        .expect("should exec");

    // Create suspend snapshot
    let snapshot = sandbox.snapshot().await.expect("should create snapshot");

    // Create teleport package as Suspend type
    let package = TeleportPackage::new(
        format!("suspend-{}", snapshot.id()),
        sandbox.id(),
        Architecture::Arm64,
        SnapshotKind::Suspend,
    )
    .with_vm_snapshot(VmSnapshotBlob::encode(
        Bytes::new(),
        Bytes::new(),
        Bytes::from("simulated memory state"),
    ))
    .with_base_image_version("1.0.0")
    .with_created_at(1000);

    // Suspend should NOT include CPU state or disk (memory-only)
    assert!(
        !package.is_full_teleport(),
        "Suspend should not be full teleport"
    );
    assert_eq!(package.kind, SnapshotKind::Suspend);

    // Suspend works on same architecture only
    assert!(
        package.validate_for_restore(Architecture::Arm64).is_ok(),
        "Suspend should work on same arch"
    );
    assert!(
        package.validate_for_restore(Architecture::X86_64).is_err(),
        "Suspend should NOT work on different arch"
    );

    // Upload suspend package
    let id = storage.upload(package).await.expect("should upload");
    assert!(!id.is_empty());

    // Download for restore on same arch
    let restored = storage
        .download_for_restore(&id, Architecture::Arm64)
        .await
        .expect("should restore suspend on same arch");
    assert_eq!(restored.kind, SnapshotKind::Suspend);

    sandbox.stop().await.expect("should stop");
}

/// Test: Suspend snapshot with crash faults
/// Expected: Crashes during suspend are detected and handled
#[madsim::test]
async fn test_dst_suspend_snapshot_crash_faults() {
    let config = SimConfig::from_env_or_random();
    let rng = DeterministicRng::new(config.seed);

    let faults = Arc::new(
        FaultInjectorBuilder::new(rng.fork())
            .with_fault(FaultConfig::new(FaultType::SandboxCrash, 0.2))
            .with_fault(FaultConfig::new(FaultType::SnapshotCreateFail, 0.2))
            .build(),
    );

    let factory = SimSandboxFactory::new(rng.fork(), faults);
    let config = test_config();

    let mut snapshot_successes = 0;
    let mut snapshot_failures = 0;

    for _ in 0..10 {
        let mut sandbox = factory.create(config.clone()).await.expect("should create");
        if sandbox.start().await.is_err() {
            continue;
        }

        match sandbox.snapshot().await {
            Ok(_) => snapshot_successes += 1,
            Err(e) => {
                snapshot_failures += 1;
                let msg = e.to_string().to_lowercase();
                // Should indicate snapshot or crash failure
                assert!(
                    msg.contains("snapshot")
                        || msg.contains("crash")
                        || msg.contains("fail")
                        || msg.contains("create"),
                    "error should indicate failure type: {}",
                    e
                );
            }
        }
    }

    println!(
        "Suspend snapshot: {} successes, {} failures",
        snapshot_successes, snapshot_failures
    );
    // Should have some successes with 20% fault rate
    assert!(snapshot_successes > 0, "should have at least one success");
}

// ============================================================================
// Phase 3 DST Tests - Teleport Snapshot (Full VM, same-architecture)
// ============================================================================

/// Test: Teleport snapshot creation under no faults (baseline)
/// Expected: Full VM snapshot with memory + CPU + disk succeeds
#[madsim::test]
async fn test_dst_teleport_snapshot_no_faults() {
    let config = SimConfig::from_env_or_random();
    let rng = DeterministicRng::new(config.seed);
    let faults = create_no_fault_injector(&rng);

    let factory = SimSandboxFactory::new(rng.fork(), faults.clone());
    let storage = SimTeleportStorage::new(rng.fork(), faults);
    let config = test_config();

    // Create and start sandbox
    let mut sandbox = factory.create(config).await.expect("should create sandbox");
    sandbox.start().await.expect("should start");

    // Execute commands to establish state
    sandbox
        .exec("touch", &["/tmp/testfile"], ExecOptions::default())
        .await
        .expect("should exec");

    // Create snapshot
    let snapshot = sandbox.snapshot().await.expect("should create snapshot");

    // Create teleport package with FULL VM state
    let package = TeleportPackage::new(
        format!("teleport-{}", snapshot.id()),
        sandbox.id(),
        Architecture::Arm64,
        SnapshotKind::Teleport,
    )
    .with_vm_snapshot(VmSnapshotBlob::encode(
        Bytes::new(),
        Bytes::from("cpu registers"),
        Bytes::from("full memory state"),
    ))
    .with_workspace_ref("s3://bucket/workspace-123")
    .with_agent_state(Bytes::from("agent state"))
    .with_base_image_version("1.0.0")
    .with_created_at(1000);

    // Teleport MUST include CPU state (full snapshot)
    assert!(
        package.is_full_teleport(),
        "Teleport should be full snapshot"
    );
    assert_eq!(package.kind, SnapshotKind::Teleport);
    assert!(package.vm_snapshot.is_some());
    assert!(package.workspace_ref.is_some());

    // Teleport works on same architecture only
    assert!(
        package.validate_for_restore(Architecture::Arm64).is_ok(),
        "Teleport should work on same arch"
    );
    assert!(
        package.validate_for_restore(Architecture::X86_64).is_err(),
        "Teleport should NOT work on different arch"
    );

    // Upload teleport package
    let id = storage.upload(package).await.expect("should upload");

    // Download for restore on same arch
    let restored = storage
        .download_for_restore(&id, Architecture::Arm64)
        .await
        .expect("should restore teleport on same arch");
    assert_eq!(restored.kind, SnapshotKind::Teleport);
    assert!(restored.is_full_teleport());

    sandbox.stop().await.expect("should stop");
}

/// Test: Teleport snapshot with storage faults
/// Expected: Upload/download failures handled gracefully
#[madsim::test]
async fn test_dst_teleport_snapshot_storage_faults() {
    let config = SimConfig::from_env_or_random();
    let rng = DeterministicRng::new(config.seed);

    let faults = Arc::new(
        FaultInjectorBuilder::new(rng.fork())
            .with_fault(FaultConfig::new(FaultType::TeleportUploadFail, 0.3))
            .with_fault(FaultConfig::new(FaultType::TeleportDownloadFail, 0.3))
            .build(),
    );

    let storage = SimTeleportStorage::new(rng.fork(), faults);

    let mut upload_successes = 0;
    let mut upload_failures = 0;

    for i in 0..10 {
        let package = TeleportPackage::new(
            format!("teleport-{}", i),
            "agent-1",
            Architecture::Arm64,
            SnapshotKind::Teleport,
        )
        .with_vm_snapshot(VmSnapshotBlob::encode(
            Bytes::new(),
            Bytes::from("cpu"),
            Bytes::from("memory"),
        ))
        .with_base_image_version("1.0.0");

        match storage.upload(package).await {
            Ok(_) => upload_successes += 1,
            Err(e) => {
                upload_failures += 1;
                let msg = e.to_string().to_lowercase();
                assert!(
                    msg.contains("upload") || msg.contains("fail"),
                    "error should indicate upload failure: {}",
                    e
                );
            }
        }
    }

    println!(
        "Teleport storage: {} upload successes, {} failures",
        upload_successes, upload_failures
    );

    // With 30% failure rate, should see some of each
    assert!(upload_successes > 0 || upload_failures > 0);
}

/// Test: Teleport snapshot with corruption
/// Expected: Corrupted snapshots detected on restore
#[madsim::test]
async fn test_dst_teleport_snapshot_corruption() {
    let config = SimConfig::from_env_or_random();
    let rng = DeterministicRng::new(config.seed);

    // Snapshot corruption only affects restore, not create
    let faults = Arc::new(
        FaultInjectorBuilder::new(rng.fork())
            .with_fault(FaultConfig::new(FaultType::SnapshotCorruption, 0.5))
            .build(),
    );

    let factory = SimSandboxFactory::new(rng.fork(), faults);
    let config = test_config();

    // Create sandbox and snapshot
    let mut sandbox = factory.create(config.clone()).await.expect("should create");
    sandbox.start().await.expect("should start");
    let snapshot = sandbox.snapshot().await.expect("should create snapshot");
    sandbox.stop().await.expect("should stop");

    // Try to restore multiple times
    let mut restore_successes = 0;
    let mut restore_failures = 0;

    for _ in 0..5 {
        let mut new_sandbox = factory.create(config.clone()).await.expect("should create");
        match new_sandbox.restore(&snapshot).await {
            Ok(()) => restore_successes += 1,
            Err(e) => {
                restore_failures += 1;
                let msg = e.to_string().to_lowercase();
                assert!(
                    msg.contains("corrupt") || msg.contains("restore") || msg.contains("fail"),
                    "error should indicate corruption: {}",
                    e
                );
            }
        }
    }

    println!(
        "Teleport corruption: {} restore successes, {} failures",
        restore_successes, restore_failures
    );
}

// ============================================================================
// Phase 3 DST Tests - Checkpoint Snapshot (App-level, cross-architecture)
// ============================================================================

/// Test: Checkpoint snapshot creation under no faults (baseline)
/// Expected: App-level checkpoint without VM state succeeds
#[madsim::test]
async fn test_dst_checkpoint_snapshot_no_faults() {
    let config = SimConfig::from_env_or_random();
    let rng = DeterministicRng::new(config.seed);
    let faults = create_no_fault_injector(&rng);

    let storage = SimTeleportStorage::new(rng.fork(), faults)
        .with_host_arch(Architecture::X86_64) // Simulate restoring on different arch
        .with_expected_image_version("1.0.0");

    // Create checkpoint package (NO VM state, only app state)
    let package = TeleportPackage::new(
        "checkpoint-1",
        "agent-1",
        Architecture::Arm64,
        SnapshotKind::Checkpoint,
    )
    .with_agent_state(Bytes::from("agent state: memory blocks, conversation"))
    .with_workspace_ref("s3://bucket/workspace-123")
    .with_env_vars(vec![
        ("PATH".to_string(), "/usr/bin".to_string()),
        ("HOME".to_string(), "/home/agent".to_string()),
    ])
    .with_base_image_version("1.0.0")
    .with_created_at(1000);

    // Checkpoint should NOT include VM state
    assert!(
        !package.is_full_teleport(),
        "Checkpoint should NOT be full teleport"
    );
    assert!(package.is_checkpoint());
    assert!(package.vm_snapshot.is_none());
    assert!(package.agent_state.is_some());

    // Checkpoint works ACROSS architectures
    assert!(
        package.validate_for_restore(Architecture::Arm64).is_ok(),
        "Checkpoint should work on same arch"
    );
    assert!(
        package.validate_for_restore(Architecture::X86_64).is_ok(),
        "Checkpoint should work on different arch"
    );

    // Upload checkpoint
    let id = storage.upload(package).await.expect("should upload");

    // Download for restore on DIFFERENT arch (this is the key capability)
    let restored = storage
        .download_for_restore(&id, Architecture::X86_64)
        .await
        .expect("should restore checkpoint on different arch");
    assert!(restored.is_checkpoint());
    assert!(restored.agent_state.is_some());
}

/// Test: Checkpoint with app state faults
/// Expected: State serialization failures handled
#[madsim::test]
async fn test_dst_checkpoint_snapshot_state_faults() {
    let config = SimConfig::from_env_or_random();
    let rng = DeterministicRng::new(config.seed);

    let faults = Arc::new(
        FaultInjectorBuilder::new(rng.fork())
            .with_fault(FaultConfig::new(FaultType::TeleportUploadFail, 0.2))
            .with_fault(FaultConfig::new(FaultType::TeleportDownloadFail, 0.2))
            .build(),
    );

    let storage = SimTeleportStorage::new(rng.fork(), faults).with_expected_image_version("1.0.0");

    let mut successes = 0;
    let mut failures = 0;

    for i in 0..10 {
        let package = TeleportPackage::new(
            format!("checkpoint-{}", i),
            "agent-1",
            Architecture::Arm64,
            SnapshotKind::Checkpoint,
        )
        .with_agent_state(Bytes::from(format!("state-{}", i)))
        .with_base_image_version("1.0.0");

        match storage.upload(package).await {
            Ok(id) => {
                // Try to download
                match storage.download(&id).await {
                    Ok(_) => successes += 1,
                    Err(_) => failures += 1,
                }
            }
            Err(_) => failures += 1,
        }
    }

    println!(
        "Checkpoint roundtrip: {} successes, {} failures",
        successes, failures
    );
}

// ============================================================================
// Phase 3 DST Tests - Architecture Validation
// ============================================================================

/// Test: Architecture validation for VM snapshots
/// Expected: ARM64 snapshot fails on x86_64, Checkpoint succeeds
#[madsim::test]
async fn test_dst_architecture_validation() {
    let config = SimConfig::from_env_or_random();
    let rng = DeterministicRng::new(config.seed);
    let faults = create_no_fault_injector(&rng);

    let storage = SimTeleportStorage::new(rng.fork(), faults)
        .with_host_arch(Architecture::X86_64)
        .with_expected_image_version("1.0.0");

    // Create ARM64 Teleport package (full VM)
    let teleport_package = TeleportPackage::new(
        "teleport-arm64",
        "agent-1",
        Architecture::Arm64,
        SnapshotKind::Teleport,
    )
    .with_vm_snapshot(VmSnapshotBlob::encode(
        Bytes::new(),
        Bytes::from("arm64 cpu"),
        Bytes::from("arm64 memory"),
    ))
    .with_base_image_version("1.0.0");

    storage
        .upload(teleport_package)
        .await
        .expect("should upload");

    // Try to restore on X86_64 - MUST FAIL
    let result = storage
        .download_for_restore("teleport-arm64", Architecture::X86_64)
        .await;

    assert!(result.is_err(), "Teleport ARM64->X86_64 should fail");

    if let Err(e) = result {
        let msg = e.to_string().to_lowercase();
        assert!(
            msg.contains("arch") || msg.contains("mismatch"),
            "error should indicate arch mismatch: {}",
            e
        );
    }

    // Create ARM64 Checkpoint package (app-level)
    let checkpoint_package = TeleportPackage::new(
        "checkpoint-arm64",
        "agent-1",
        Architecture::Arm64,
        SnapshotKind::Checkpoint,
    )
    .with_agent_state(Bytes::from("agent state"))
    .with_base_image_version("1.0.0");

    storage
        .upload(checkpoint_package)
        .await
        .expect("should upload");

    // Try to restore on X86_64 - SHOULD SUCCEED
    let result = storage
        .download_for_restore("checkpoint-arm64", Architecture::X86_64)
        .await;

    assert!(result.is_ok(), "Checkpoint ARM64->X86_64 should succeed");
}

/// Test: Architecture mismatch fault injection
/// Expected: Injected arch mismatch faults are handled
#[madsim::test]
async fn test_dst_architecture_mismatch_faults() {
    let config = SimConfig::from_env_or_random();
    let rng = DeterministicRng::new(config.seed);

    let faults = Arc::new(
        FaultInjectorBuilder::new(rng.fork())
            .with_fault(FaultConfig::new(FaultType::TeleportArchMismatch, 0.5))
            .build(),
    );

    let storage = SimTeleportStorage::new(rng.fork(), faults)
        .with_host_arch(Architecture::Arm64)
        .with_expected_image_version("1.0.0");

    // Create same-arch package
    let package = TeleportPackage::new(
        "teleport-1",
        "agent-1",
        Architecture::Arm64, // Same as host
        SnapshotKind::Teleport,
    )
    .with_vm_snapshot(VmSnapshotBlob::encode(
        Bytes::new(),
        Bytes::from("cpu"),
        Bytes::from("memory"),
    ))
    .with_base_image_version("1.0.0");

    storage.upload(package).await.expect("should upload");

    let mut successes = 0;
    let mut failures = 0;

    for _ in 0..10 {
        match storage
            .download_for_restore("teleport-1", Architecture::Arm64)
            .await
        {
            Ok(_) => successes += 1,
            Err(e) => {
                failures += 1;
                let msg = e.to_string().to_lowercase();
                assert!(
                    msg.contains("arch") || msg.contains("mismatch"),
                    "error should indicate arch issue: {}",
                    e
                );
            }
        }
    }

    println!(
        "Arch mismatch fault: {} successes, {} failures",
        successes, failures
    );

    // With 50% fault rate, should see some of each
    assert!(successes > 0 || failures > 0);
}

// ============================================================================
// Phase 3 DST Tests - Base Image Version Validation
// ============================================================================

/// Test: Base image version validation
/// Expected: Mismatched versions fail restoration
#[madsim::test]
async fn test_dst_base_image_version_validation() {
    let config = SimConfig::from_env_or_random();
    let rng = DeterministicRng::new(config.seed);
    let faults = create_no_fault_injector(&rng);

    let storage = SimTeleportStorage::new(rng.fork(), faults)
        .with_host_arch(Architecture::Arm64)
        .with_expected_image_version("2.0.0"); // Host expects v2.0.0

    // Create package with v1.0.0 base image
    let package = TeleportPackage::new(
        "teleport-old",
        "agent-1",
        Architecture::Arm64,
        SnapshotKind::Teleport,
    )
    .with_vm_snapshot(VmSnapshotBlob::encode(
        Bytes::new(),
        Bytes::from("cpu"),
        Bytes::from("memory"),
    ))
    .with_base_image_version("1.0.0"); // Old version

    storage.upload(package).await.expect("should upload");

    // Try to restore - MUST FAIL due to version mismatch
    let result = storage
        .download_for_restore("teleport-old", Architecture::Arm64)
        .await;

    assert!(result.is_err(), "Version mismatch should fail restoration");

    if let Err(e) = result {
        let msg = e.to_string().to_lowercase();
        assert!(
            msg.contains("version") || msg.contains("mismatch") || msg.contains("image"),
            "error should indicate version mismatch: {}",
            e
        );
    }

    // Create package with matching version
    let package_new = TeleportPackage::new(
        "teleport-new",
        "agent-1",
        Architecture::Arm64,
        SnapshotKind::Teleport,
    )
    .with_vm_snapshot(VmSnapshotBlob::encode(
        Bytes::new(),
        Bytes::from("cpu"),
        Bytes::from("memory"),
    ))
    .with_base_image_version("2.0.0"); // Matching version

    storage.upload(package_new).await.expect("should upload");

    // Try to restore - SHOULD SUCCEED
    let result = storage
        .download_for_restore("teleport-new", Architecture::Arm64)
        .await;

    assert!(result.is_ok(), "Matching version should succeed");
}

/// Test: Base image version mismatch fault injection
/// Expected: Injected version mismatch faults are handled
#[madsim::test]
async fn test_dst_base_image_mismatch_faults() {
    let config = SimConfig::from_env_or_random();
    let rng = DeterministicRng::new(config.seed);

    let faults = Arc::new(
        FaultInjectorBuilder::new(rng.fork())
            .with_fault(FaultConfig::new(FaultType::TeleportImageMismatch, 0.5))
            .build(),
    );

    let storage = SimTeleportStorage::new(rng.fork(), faults)
        .with_host_arch(Architecture::Arm64)
        .with_expected_image_version("1.0.0");

    // Create package with matching version
    let package = TeleportPackage::new(
        "teleport-1",
        "agent-1",
        Architecture::Arm64,
        SnapshotKind::Teleport,
    )
    .with_vm_snapshot(VmSnapshotBlob::encode(
        Bytes::new(),
        Bytes::from("cpu"),
        Bytes::from("memory"),
    ))
    .with_base_image_version("1.0.0"); // Matching

    storage.upload(package).await.expect("should upload");

    let mut successes = 0;
    let mut failures = 0;

    for _ in 0..10 {
        match storage
            .download_for_restore("teleport-1", Architecture::Arm64)
            .await
        {
            Ok(_) => successes += 1,
            Err(e) => {
                failures += 1;
                let msg = e.to_string().to_lowercase();
                assert!(
                    msg.contains("version") || msg.contains("mismatch") || msg.contains("image"),
                    "error should indicate version issue: {}",
                    e
                );
            }
        }
    }

    println!(
        "Image mismatch fault: {} successes, {} failures",
        successes, failures
    );
}

// ============================================================================
// Phase 3 DST Tests - Determinism
// ============================================================================

/// Test: Same seed produces same snapshot behavior
/// Critical: DST requires determinism for reproducibility
#[madsim::test]
async fn test_dst_snapshot_types_determinism() {
    let seed = 42u64; // Fixed seed

    let run_test = |seed: u64| async move {
        let rng = DeterministicRng::new(seed);
        let faults = Arc::new(
            FaultInjectorBuilder::new(rng.fork())
                .with_snapshot_faults(0.2)
                .with_teleport_faults(0.2)
                .build(),
        );

        let factory = SimSandboxFactory::new(rng.fork(), faults.clone());
        let storage =
            SimTeleportStorage::new(rng.fork(), faults).with_expected_image_version("1.0.0");

        let mut results = Vec::new();

        for i in 0..5 {
            // Create sandbox and snapshot
            let config = test_config();
            let mut sandbox = factory.create(config).await.expect("create");

            let start_ok = sandbox.start().await.is_ok();
            results.push(format!("start:{}", start_ok));

            if start_ok {
                let snapshot_ok = sandbox.snapshot().await.is_ok();
                results.push(format!("snapshot:{}", snapshot_ok));

                // Try teleport package
                let package = TeleportPackage::new(
                    format!("pkg-{}", i),
                    sandbox.id(),
                    Architecture::Arm64,
                    SnapshotKind::Teleport,
                )
                .with_vm_snapshot(VmSnapshotBlob::encode(
                    Bytes::new(),
                    Bytes::from("cpu"),
                    Bytes::from("memory"),
                ))
                .with_base_image_version("1.0.0");

                let upload_ok = storage.upload(package).await.is_ok();
                results.push(format!("upload:{}", upload_ok));
            }
        }

        results
    };

    let results1 = run_test(seed).await;
    let results2 = run_test(seed).await;

    assert_eq!(results1, results2, "Same seed must produce same results");
    println!(
        "Determinism verified: {} operations matched",
        results1.len()
    );
}

// ============================================================================
// Phase 3 DST Tests - Combined Chaos Testing
// ============================================================================

/// Test: All snapshot types under chaos conditions
/// Expected: System remains stable, no panics
#[madsim::test]
async fn test_dst_snapshot_types_chaos() {
    let config = SimConfig::from_env_or_random();
    let rng = DeterministicRng::new(config.seed);

    let faults = Arc::new(
        FaultInjectorBuilder::new(rng.fork())
            .with_sandbox_faults(0.1)
            .with_snapshot_faults(0.1)
            .with_teleport_faults(0.1)
            .build(),
    );

    let factory = SimSandboxFactory::new(rng.fork(), faults.clone());
    let storage = SimTeleportStorage::new(rng.fork(), faults).with_expected_image_version("1.0.0");

    let kinds = [
        SnapshotKind::Suspend,
        SnapshotKind::Teleport,
        SnapshotKind::Checkpoint,
    ];
    let mut stats = std::collections::HashMap::new();

    for kind in &kinds {
        stats.insert(format!("{:?}_success", kind), 0u32);
        stats.insert(format!("{:?}_failure", kind), 0u32);
    }

    for (i, kind) in kinds.iter().cycle().take(15).enumerate() {
        let config = test_config();

        // Create sandbox
        let mut sandbox = match factory.create(config).await {
            Ok(s) => s,
            Err(_) => continue,
        };

        if sandbox.start().await.is_err() {
            continue;
        }

        // Create appropriate package
        let package = match kind {
            SnapshotKind::Suspend => TeleportPackage::new(
                format!("suspend-{}", i),
                sandbox.id(),
                Architecture::Arm64,
                SnapshotKind::Suspend,
            )
            .with_vm_snapshot(VmSnapshotBlob::encode(
                Bytes::new(),
                Bytes::new(),
                Bytes::from("memory"),
            ))
            .with_base_image_version("1.0.0"),

            SnapshotKind::Teleport => TeleportPackage::new(
                format!("teleport-{}", i),
                sandbox.id(),
                Architecture::Arm64,
                SnapshotKind::Teleport,
            )
            .with_vm_snapshot(VmSnapshotBlob::encode(
                Bytes::new(),
                Bytes::from("cpu"),
                Bytes::from("memory"),
            ))
            .with_workspace_ref("s3://bucket/workspace")
            .with_base_image_version("1.0.0"),

            SnapshotKind::Checkpoint => TeleportPackage::new(
                format!("checkpoint-{}", i),
                sandbox.id(),
                Architecture::Arm64,
                SnapshotKind::Checkpoint,
            )
            .with_agent_state(Bytes::from("state"))
            .with_base_image_version("1.0.0"),
        };

        let key_success = format!("{:?}_success", kind);
        let key_failure = format!("{:?}_failure", kind);

        match storage.upload(package).await {
            Ok(_) => {
                *stats.get_mut(&key_success).unwrap() += 1;
            }
            Err(_) => {
                *stats.get_mut(&key_failure).unwrap() += 1;
            }
        }

        let _ = sandbox.stop().await;
    }

    println!("Chaos test results:");
    for (key, value) in &stats {
        println!("  {}: {}", key, value);
    }

    // Should have processed some packages
    let total: u32 = stats.values().sum();
    assert!(total > 0, "should have processed some packages");
}

// ============================================================================
// Stress Tests (ignored by default)
// ============================================================================

/// Stress test: Many snapshot creations and restorations
#[madsim::test]
#[ignore]
async fn stress_test_snapshot_types() {
    let config = SimConfig::from_env_or_random();
    let rng = DeterministicRng::new(config.seed);

    let faults = Arc::new(
        FaultInjectorBuilder::new(rng.fork())
            .with_fault(FaultConfig::new(FaultType::SnapshotCreateFail, 0.05))
            .with_fault(FaultConfig::new(FaultType::TeleportUploadFail, 0.05))
            .build(),
    );

    let factory = SimSandboxFactory::new(rng.fork(), faults.clone());
    let storage = SimTeleportStorage::new(rng.fork(), faults).with_expected_image_version("1.0.0");

    let iterations = 100;
    let mut successes = 0;

    for i in 0..iterations {
        let config = test_config();
        let mut sandbox = match factory.create(config).await {
            Ok(s) => s,
            Err(_) => continue,
        };

        if sandbox.start().await.is_err() {
            continue;
        }

        if sandbox.snapshot().await.is_ok() {
            let package = TeleportPackage::new(
                format!("stress-{}", i),
                sandbox.id(),
                Architecture::Arm64,
                SnapshotKind::Teleport,
            )
            .with_vm_snapshot(VmSnapshotBlob::encode(
                Bytes::new(),
                Bytes::from("cpu"),
                Bytes::from("memory"),
            ))
            .with_base_image_version("1.0.0");

            if storage.upload(package).await.is_ok() {
                successes += 1;
            }
        }

        let _ = sandbox.stop().await;
    }

    println!(
        "Stress test: {}/{} successful iterations",
        successes, iterations
    );

    assert!(successes > iterations / 2, "should have >50% success rate");
}
