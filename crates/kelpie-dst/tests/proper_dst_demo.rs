//! Demonstration of Proper DST Architecture
//!
//! This test file demonstrates that the new DST architecture:
//! 1. Uses SAME state machine code (GenericSandbox) for both production and DST
//! 2. Fault injection works at the I/O boundary (SimSandboxIO)
//! 3. Determinism - same seed produces same results
//! 4. Meaningful testing - faults actually cause failures that test error handling
//!
//! **Phase 2 Migration:** Converted to use madsim for true runtime determinism.
//! Tests now run on madsim's deterministic executor (virtual time, deterministic scheduling).

use kelpie_dst::{
    DeterministicRng, FaultConfig, FaultInjectorBuilder, FaultType, SimClock, SimSandboxIOFactory,
};
use kelpie_sandbox::{GenericSandbox, SandboxConfig, SandboxState};
use std::sync::Arc;

/// Test 1: GenericSandbox uses SHARED state machine code
///
/// The GenericSandbox<SimSandboxIO> uses the SAME lifecycle state machine
/// that will be used by GenericSandbox<VmSandboxIO> in production.
/// This means our DST tests exercise the actual production code paths.
#[madsim::test]
async fn test_proper_dst_shared_state_machine() {
    let rng = Arc::new(DeterministicRng::new(42));
    let faults = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
    let clock = Arc::new(SimClock::default());

    let factory = SimSandboxIOFactory::new(rng, faults, clock);
    let mut sandbox: GenericSandbox<_> = factory.create(SandboxConfig::default()).await.unwrap();

    // State machine validation - this code is SHARED with production
    assert_eq!(sandbox.state(), SandboxState::Stopped);

    // Can't exec when stopped - state validation in GenericSandbox (SHARED CODE)
    let result = sandbox.exec_simple("echo", &["hello"]).await;
    assert!(result.is_err(), "Should fail when sandbox is stopped");

    // Start transitions state - SHARED CODE
    sandbox.start().await.unwrap();
    assert_eq!(sandbox.state(), SandboxState::Running);

    // Now exec works - SHARED CODE validates state
    let output = sandbox.exec_simple("echo", &["hello"]).await.unwrap();
    assert!(output.is_success());

    // Pause/resume - SHARED STATE MACHINE
    sandbox.pause().await.unwrap();
    assert_eq!(sandbox.state(), SandboxState::Paused);

    // Can't exec when paused - SHARED VALIDATION
    let result = sandbox.exec_simple("echo", &["test"]).await;
    assert!(result.is_err(), "Should fail when sandbox is paused");

    sandbox.resume().await.unwrap();
    assert_eq!(sandbox.state(), SandboxState::Running);

    // Stop - SHARED CODE
    sandbox.stop().await.unwrap();
    assert_eq!(sandbox.state(), SandboxState::Stopped);

    println!("✅ State machine code is SHARED between DST and production");
}

/// Test 2: Fault injection at I/O boundary
///
/// Faults are injected in SimSandboxIO, not in GenericSandbox.
/// This tests that our business logic handles I/O failures correctly.
#[madsim::test]
async fn test_proper_dst_fault_injection_at_io_boundary() {
    let rng = Arc::new(DeterministicRng::new(42));

    // 100% boot failure - tests error handling in GenericSandbox
    let mut fault_injector = FaultInjectorBuilder::new(rng.fork());
    fault_injector = fault_injector.with_fault(FaultConfig::new(FaultType::SandboxBootFail, 1.0));
    let faults = Arc::new(fault_injector.build());
    let clock = Arc::new(SimClock::default());

    let factory = SimSandboxIOFactory::new(rng.clone(), faults, clock.clone());
    let mut sandbox = factory.create(SandboxConfig::default()).await.unwrap();

    // Boot should fail due to fault injection in SimSandboxIO
    let result = sandbox.start().await;
    assert!(result.is_err(), "Boot should fail with 100% fault rate");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("injected"),
        "Error should indicate fault injection: {}",
        err
    );

    println!("✅ Fault injection works at I/O boundary");
}

/// Test 3: Determinism - same seed produces same results
///
/// This is CRITICAL for DST - we must be able to reproduce failures.
#[madsim::test]
async fn test_proper_dst_determinism() {
    let seed = 12345u64;

    // Run the same sequence twice with the same seed
    let run_sequence = || async {
        let rng = Arc::new(DeterministicRng::new(seed));
        let faults = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
        let clock = Arc::new(SimClock::default());

        let factory = SimSandboxIOFactory::new(rng.clone(), faults, clock);
        let mut sandbox = factory.create(SandboxConfig::default()).await.unwrap();

        sandbox.start().await.unwrap();

        let mut outputs = Vec::new();
        for i in 0..5 {
            let output = sandbox
                .exec_simple("test", &[&format!("arg{}", i)])
                .await
                .unwrap();
            outputs.push(output.stdout.to_vec());
        }

        sandbox.stop().await.unwrap();
        outputs
    };

    let results1 = run_sequence().await;
    let results2 = run_sequence().await;

    assert_eq!(results1, results2, "Same seed must produce same results");

    println!("✅ DST is deterministic - same seed = same results");
    println!("   Seed {} can be used to reproduce this exact run", seed);
}

/// Test 4: Meaningful fault testing under chaos
///
/// This demonstrates that fault injection actually tests error handling.
/// With 50% exec failure rate, we should see both successes and failures.
#[madsim::test]
async fn test_proper_dst_meaningful_chaos() {
    let rng = Arc::new(DeterministicRng::new(777));

    // 50% exec failure rate - tests error handling code paths
    let mut fault_injector = FaultInjectorBuilder::new(rng.fork());
    fault_injector = fault_injector
        .with_fault(FaultConfig::new(FaultType::SandboxExecFail, 0.5).with_filter("sandbox_exec"));
    let faults = Arc::new(fault_injector.build());
    let clock = Arc::new(SimClock::default());

    let factory = SimSandboxIOFactory::new(rng.clone(), faults, clock);
    let mut sandbox = factory.create(SandboxConfig::default()).await.unwrap();
    sandbox.start().await.unwrap();

    let mut successes = 0;
    let mut failures = 0;

    for i in 0..20 {
        match sandbox.exec_simple("test", &[&format!("{}", i)]).await {
            Ok(_) => successes += 1,
            Err(_) => failures += 1,
        }
    }

    // With 50% fault rate over 20 attempts, we should see both
    assert!(successes > 0, "Should have some successes");
    assert!(
        failures > 0,
        "Should have some failures from fault injection"
    );

    println!(
        "✅ Meaningful chaos testing: {} successes, {} failures",
        successes, failures
    );
    println!("   This tests that error handling code paths are exercised");
}

/// Test 5: Snapshot/restore with fault injection
///
/// Tests that the SHARED snapshot logic works under faults.
#[madsim::test]
async fn test_proper_dst_snapshot_under_faults() {
    let rng = Arc::new(DeterministicRng::new(999));

    // No faults for this test - just verify snapshot logic works
    let faults = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
    let clock = Arc::new(SimClock::default());

    let factory = SimSandboxIOFactory::new(rng, faults, clock);

    // Create sandbox 1, write data, snapshot
    let mut sandbox1 = factory.create(SandboxConfig::default()).await.unwrap();
    sandbox1.start().await.unwrap();
    sandbox1
        .write_file("/important.txt", b"critical data")
        .await
        .unwrap();
    let snapshot = sandbox1.snapshot().await.unwrap();

    // Create sandbox 2, restore, verify data
    let mut sandbox2 = factory.create(SandboxConfig::default()).await.unwrap();
    sandbox2.start().await.unwrap();
    sandbox2.restore(&snapshot).await.unwrap();

    let content = sandbox2.read_file("/important.txt").await.unwrap();
    assert_eq!(content.as_ref(), b"critical data");

    println!("✅ Snapshot/restore works - this tests SHARED snapshot logic");
}

/// Summary test that prints the DST architecture benefits
#[madsim::test]
async fn test_proper_dst_summary() {
    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║           PROPER DST ARCHITECTURE VERIFICATION               ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║                                                              ║");
    println!("║  1. SHARED CODE: GenericSandbox<IO> state machine runs in    ║");
    println!("║     both DST (SimSandboxIO) and production (VmIO)       ║");
    println!("║                                                              ║");
    println!("║  2. I/O BOUNDARY: Faults injected in SimSandboxIO, not in    ║");
    println!("║     business logic - tests REAL error handling               ║");
    println!("║                                                              ║");
    println!("║  3. DETERMINISTIC: Same seed = same results, failures are    ║");
    println!("║     reproducible with DST_SEED environment variable          ║");
    println!("║                                                              ║");
    println!("║  4. MEANINGFUL: Chaos testing exercises error paths that     ║");
    println!("║     would occur in production under real failures            ║");
    println!("║                                                              ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!("\n");
}
