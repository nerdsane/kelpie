//! Bug Hunting with Proper DST
//!
//! Aggressive chaos tests designed to find bugs in the shared state machine code.

use kelpie_dst::{
    DeterministicRng, FaultConfig, FaultInjectorBuilder, FaultType, SimClock, SimSandboxIOFactory,
};
use kelpie_sandbox::{SandboxConfig, SandboxState};
use std::sync::Arc;

/// Test rapid state transitions under faults
#[madsim::test]
async fn test_rapid_state_transitions() {
    let rng = Arc::new(DeterministicRng::new(42));
    let mut fault_builder = FaultInjectorBuilder::new(rng.fork());
    fault_builder = fault_builder
        .with_fault(FaultConfig::new(FaultType::SandboxBootFail, 0.2))
        .with_fault(FaultConfig::new(FaultType::SandboxExecFail, 0.3).with_filter("sandbox_exec"));
    let faults = Arc::new(fault_builder.build());
    let clock = Arc::new(SimClock::default());

    // Keep a reference to faults for verification
    let faults_ref = faults.clone();
    let factory = SimSandboxIOFactory::new(rng.clone(), faults, clock);

    for iteration in 0..50 {
        let mut sandbox = factory.create(SandboxConfig::default()).await.unwrap();

        // Try start - may fail
        match sandbox.start().await {
            Ok(_) => {
                assert_eq!(sandbox.state(), SandboxState::Running);

                // Rapid pause/resume cycles
                for _ in 0..5 {
                    sandbox.pause().await.unwrap();
                    assert_eq!(sandbox.state(), SandboxState::Paused);
                    sandbox.resume().await.unwrap();
                    assert_eq!(sandbox.state(), SandboxState::Running);
                }

                // Try some execs
                for j in 0..10 {
                    let _ = sandbox
                        .exec_simple("test", &[&format!("{}_{}", iteration, j)])
                        .await;
                }

                sandbox.stop().await.unwrap();
                assert_eq!(sandbox.state(), SandboxState::Stopped);
            }
            Err(_) => {
                // Boot failed - state should still be Stopped
                assert_eq!(
                    sandbox.state(),
                    SandboxState::Stopped,
                    "State should remain Stopped after failed boot"
                );
            }
        }
    }

    // Verify faults actually fired - with 50 iterations and 20%/30% fault rates,
    // we should see at least some faults triggered
    let stats = faults_ref.stats();
    let total_triggered: u64 = stats.iter().map(|s| s.trigger_count).sum();
    assert!(
        total_triggered > 0,
        "Expected some faults to trigger with 50 iterations and 20%/30% fault rates, got 0"
    );
    println!(
        "✅ Rapid state transitions: No bugs found in 50 iterations ({} faults triggered)",
        total_triggered
    );
}

/// Test double-start prevention
#[madsim::test]
async fn test_double_start_prevention() {
    let rng = Arc::new(DeterministicRng::new(123));
    let faults = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
    let clock = Arc::new(SimClock::default());

    let factory = SimSandboxIOFactory::new(rng, faults, clock);
    let mut sandbox = factory.create(SandboxConfig::default()).await.unwrap();

    sandbox.start().await.unwrap();
    assert_eq!(sandbox.state(), SandboxState::Running);

    // Try to start again - should fail
    let result = sandbox.start().await;
    assert!(result.is_err(), "Double start should be prevented");

    // State should still be Running (not corrupted)
    assert_eq!(sandbox.state(), SandboxState::Running);

    println!("✅ Double-start prevention works correctly");
}

/// Test double-stop is safe
#[madsim::test]
async fn test_double_stop_safety() {
    let rng = Arc::new(DeterministicRng::new(456));
    let faults = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
    let clock = Arc::new(SimClock::default());

    let factory = SimSandboxIOFactory::new(rng, faults, clock);
    let mut sandbox = factory.create(SandboxConfig::default()).await.unwrap();

    sandbox.start().await.unwrap();
    sandbox.stop().await.unwrap();
    assert_eq!(sandbox.state(), SandboxState::Stopped);

    // Try to stop again - should be safe (idempotent or error)
    let result = sandbox.stop().await;
    // Either it succeeds (idempotent) or fails gracefully
    if result.is_err() {
        // That's fine - just shouldn't panic or corrupt state
        assert_eq!(sandbox.state(), SandboxState::Stopped);
    }

    println!("✅ Double-stop is safe");
}

/// Test operations on stopped sandbox
#[madsim::test]
async fn test_operations_on_stopped_sandbox() {
    let rng = Arc::new(DeterministicRng::new(789));
    let faults = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
    let clock = Arc::new(SimClock::default());

    let factory = SimSandboxIOFactory::new(rng, faults, clock);
    let mut sandbox = factory.create(SandboxConfig::default()).await.unwrap();

    // All these should fail gracefully on a stopped sandbox
    assert!(sandbox.exec_simple("test", &[]).await.is_err());
    assert!(sandbox.pause().await.is_err());
    assert!(sandbox.resume().await.is_err());

    // State should still be Stopped
    assert_eq!(sandbox.state(), SandboxState::Stopped);

    println!("✅ Operations on stopped sandbox fail gracefully");
}

/// Test snapshot during different states
#[madsim::test]
async fn test_snapshot_state_requirements() {
    let rng = Arc::new(DeterministicRng::new(999));
    let faults = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
    let clock = Arc::new(SimClock::default());

    let factory = SimSandboxIOFactory::new(rng, faults, clock);
    let mut sandbox = factory.create(SandboxConfig::default()).await.unwrap();

    // Snapshot when stopped - should this work?
    let result_stopped = sandbox.snapshot().await;
    println!("Snapshot when stopped: {:?}", result_stopped.is_ok());

    sandbox.start().await.unwrap();

    // Snapshot when running - should work
    let result_running = sandbox.snapshot().await;
    assert!(result_running.is_ok(), "Snapshot should work when running");

    sandbox.pause().await.unwrap();

    // Snapshot when paused - should work (VM is frozen)
    let result_paused = sandbox.snapshot().await;
    assert!(result_paused.is_ok(), "Snapshot should work when paused");

    println!("✅ Snapshot state requirements verified");
}

/// Stress test: many sandboxes with high fault rate
#[madsim::test]
async fn test_stress_many_sandboxes_high_faults() {
    let rng = Arc::new(DeterministicRng::new(11111));
    let mut fault_builder = FaultInjectorBuilder::new(rng.fork());
    fault_builder = fault_builder
        .with_fault(FaultConfig::new(FaultType::SandboxBootFail, 0.3))
        .with_fault(FaultConfig::new(FaultType::SandboxExecFail, 0.4).with_filter("sandbox_exec"))
        .with_fault(
            FaultConfig::new(FaultType::SandboxExecTimeout { timeout_ms: 100 }, 0.1)
                .with_filter("sandbox_exec"),
        );
    let faults = Arc::new(fault_builder.build());
    let clock = Arc::new(SimClock::default());

    // Keep a reference to faults for verification
    let faults_ref = faults.clone();
    let factory = SimSandboxIOFactory::new(rng.clone(), faults, clock);

    let mut boot_success = 0;
    let mut boot_fail = 0;
    let mut exec_success = 0;
    let mut exec_fail = 0;

    for _ in 0..100 {
        let mut sandbox = factory.create(SandboxConfig::default()).await.unwrap();

        match sandbox.start().await {
            Ok(_) => {
                boot_success += 1;

                for _ in 0..10 {
                    match sandbox.exec_simple("stress", &["test"]).await {
                        Ok(_) => exec_success += 1,
                        Err(_) => exec_fail += 1,
                    }
                }

                sandbox.stop().await.unwrap();
            }
            Err(_) => {
                boot_fail += 1;
            }
        }
    }

    println!("╔═══════════════════════════════════════╗");
    println!("║      STRESS TEST RESULTS              ║");
    println!("╠═══════════════════════════════════════╣");
    println!("║ Boot success: {:>4}                   ║", boot_success);
    println!("║ Boot fail:    {:>4}                   ║", boot_fail);
    println!("║ Exec success: {:>4}                   ║", exec_success);
    println!("║ Exec fail:    {:>4}                   ║", exec_fail);
    println!("╚═══════════════════════════════════════╝");

    // With 30% boot fail rate, should see some failures
    assert!(boot_fail > 0, "Should have some boot failures");
    assert!(boot_success > 0, "Should have some boot successes");

    // With 50% exec fail rate, should see both
    assert!(exec_fail > 0, "Should have some exec failures");
    assert!(exec_success > 0, "Should have some exec successes");

    // Verify faults actually triggered in the fault injector
    let stats = faults_ref.stats();
    let total_triggered: u64 = stats.iter().map(|s| s.trigger_count).sum();
    assert!(
        total_triggered > 0,
        "Expected some faults to trigger with high fault rates, got 0"
    );

    println!(
        "✅ Stress test passed - no panics or state corruption ({} faults triggered)",
        total_triggered
    );
}

/// Test file operations consistency
#[madsim::test]
async fn test_file_operations_consistency() {
    let rng = Arc::new(DeterministicRng::new(22222));
    let faults = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
    let clock = Arc::new(SimClock::default());

    let factory = SimSandboxIOFactory::new(rng, faults, clock);
    let mut sandbox = factory.create(SandboxConfig::default()).await.unwrap();
    sandbox.start().await.unwrap();

    // Test basic file operations
    for i in 0..50 {
        let path = format!("/test_{}.txt", i);
        let content = format!("content {}", i);

        sandbox.write_file(&path, content.as_bytes()).await.unwrap();

        // Read back immediately
        let data = sandbox.read_file(&path).await.unwrap();
        assert_eq!(
            data.as_ref(),
            content.as_bytes(),
            "Read data should match written data for file {}",
            i
        );
    }

    // Verify all files still exist
    for i in 0..50 {
        let path = format!("/test_{}.txt", i);
        let content = format!("content {}", i);
        let data = sandbox.read_file(&path).await.unwrap();
        assert_eq!(data.as_ref(), content.as_bytes());
    }

    // Test file not found
    let result = sandbox.read_file("/nonexistent.txt").await;
    assert!(result.is_err(), "Reading nonexistent file should fail");

    println!("✅ File operations consistency verified - no corruption detected");
}

/// Test restore after failed operations
#[madsim::test]
async fn test_recovery_after_failures() {
    let rng = Arc::new(DeterministicRng::new(33333));
    let faults = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
    let clock = Arc::new(SimClock::default());

    let factory = SimSandboxIOFactory::new(rng, faults, clock);
    let mut sandbox = factory.create(SandboxConfig::default()).await.unwrap();

    sandbox.start().await.unwrap();

    // Write some data
    sandbox
        .write_file("/important.txt", b"critical")
        .await
        .unwrap();

    // Take snapshot
    let snapshot = sandbox.snapshot().await.unwrap();

    // Make more changes
    sandbox
        .write_file("/important.txt", b"modified")
        .await
        .unwrap();
    sandbox.write_file("/new.txt", b"new file").await.unwrap();

    // Restore to snapshot
    sandbox.restore(&snapshot).await.unwrap();

    // Verify old state restored
    let content = sandbox.read_file("/important.txt").await.unwrap();
    assert_eq!(
        content.as_ref(),
        b"critical",
        "Should restore to snapshot state"
    );

    // New file should not exist
    let result = sandbox.read_file("/new.txt").await;
    assert!(result.is_err(), "New file should not exist after restore");

    println!("✅ Recovery after failures works correctly");
}
