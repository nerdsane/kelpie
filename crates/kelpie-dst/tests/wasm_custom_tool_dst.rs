//! DST tests for WASM runtime and custom tool execution with fault injection
//!
//! TigerStyle: Deterministic testing of WASM compilation, execution,
//! cache behavior, and custom tool sandbox execution under fault injection.

use kelpie_dst::fault::{FaultConfig, FaultInjectorBuilder, FaultType};
use kelpie_dst::rng::DeterministicRng;
use kelpie_dst::{SimConfig, Simulation};
use std::sync::Arc;

// =============================================================================
// FaultType Tests
// =============================================================================

#[test]
fn test_wasm_fault_type_names() {
    // WASM runtime faults
    assert_eq!(FaultType::WasmCompileFail.name(), "wasm_compile_fail");
    assert_eq!(
        FaultType::WasmInstantiateFail.name(),
        "wasm_instantiate_fail"
    );
    assert_eq!(FaultType::WasmExecFail.name(), "wasm_exec_fail");
    assert_eq!(
        FaultType::WasmExecTimeout { timeout_ms: 30_000 }.name(),
        "wasm_exec_timeout"
    );
    assert_eq!(FaultType::WasmCacheEvict.name(), "wasm_cache_evict");
}

#[test]
fn test_custom_tool_fault_type_names() {
    // Custom tool faults
    assert_eq!(
        FaultType::CustomToolExecFail.name(),
        "custom_tool_exec_fail"
    );
    assert_eq!(
        FaultType::CustomToolExecTimeout { timeout_ms: 30_000 }.name(),
        "custom_tool_exec_timeout"
    );
    assert_eq!(
        FaultType::CustomToolSandboxAcquireFail.name(),
        "custom_tool_sandbox_acquire_fail"
    );
}

// =============================================================================
// Fault Injector Builder Tests
// =============================================================================

#[test]
fn test_fault_injector_builder_wasm_faults() {
    let rng = DeterministicRng::new(42);
    let injector = FaultInjectorBuilder::new(rng).with_wasm_faults(0.1).build();

    let stats = injector.stats();
    // with_wasm_faults adds 5 faults: compile, instantiate, exec, timeout, cache_evict
    assert_eq!(stats.len(), 5);

    let fault_names: Vec<&str> = stats.iter().map(|s| s.fault_type.as_str()).collect();
    assert!(fault_names.contains(&"wasm_compile_fail"));
    assert!(fault_names.contains(&"wasm_instantiate_fail"));
    assert!(fault_names.contains(&"wasm_exec_fail"));
    assert!(fault_names.contains(&"wasm_exec_timeout"));
    assert!(fault_names.contains(&"wasm_cache_evict"));
}

#[test]
fn test_fault_injector_builder_custom_tool_faults() {
    let rng = DeterministicRng::new(42);
    let injector = FaultInjectorBuilder::new(rng)
        .with_custom_tool_faults(0.1)
        .build();

    let stats = injector.stats();
    // with_custom_tool_faults adds 3 faults: exec, timeout, sandbox_acquire
    assert_eq!(stats.len(), 3);

    let fault_names: Vec<&str> = stats.iter().map(|s| s.fault_type.as_str()).collect();
    assert!(fault_names.contains(&"custom_tool_exec_fail"));
    assert!(fault_names.contains(&"custom_tool_exec_timeout"));
    assert!(fault_names.contains(&"custom_tool_sandbox_acquire_fail"));
}

// =============================================================================
// WASM Fault Injection Determinism
// =============================================================================

#[test]
fn test_wasm_fault_injection_determinism() {
    let seed = 42;

    let run_test = || {
        let rng = DeterministicRng::new(seed);
        let injector = FaultInjectorBuilder::new(rng)
            .with_fault(FaultConfig::new(FaultType::WasmCompileFail, 0.5))
            .build();

        let mut results = Vec::new();
        for i in 0..20 {
            let fault = injector.should_inject(&format!("wasm_compile_{}", i));
            results.push(fault.is_some());
        }
        results
    };

    let results1 = run_test();
    let results2 = run_test();

    assert_eq!(
        results1, results2,
        "Fault injection should be deterministic with same seed"
    );
}

#[test]
fn test_custom_tool_fault_injection_determinism() {
    let seed = 12345;

    let run_test = || {
        let rng = DeterministicRng::new(seed);
        let injector = FaultInjectorBuilder::new(rng)
            .with_custom_tool_faults(0.3)
            .build();

        let mut results = Vec::new();
        for i in 0..30 {
            let fault = injector.should_inject(&format!("custom_tool_execute_{}", i));
            results.push(fault.map(|f| f.name().to_string()));
        }
        results
    };

    let results1 = run_test();
    let results2 = run_test();

    assert_eq!(
        results1, results2,
        "Custom tool fault injection should be deterministic with same seed"
    );
}

// =============================================================================
// Fault Injection with Operation Filters
// =============================================================================

#[test]
fn test_wasm_fault_with_operation_filter() {
    let rng = DeterministicRng::new(42);
    let injector = FaultInjectorBuilder::new(rng)
        .with_fault(FaultConfig::new(FaultType::WasmCompileFail, 1.0).with_filter("wasm_compile"))
        .build();

    // Should inject for compile operations
    let compile_fault = injector.should_inject("wasm_compile");
    assert!(compile_fault.is_some());
    assert!(matches!(compile_fault, Some(FaultType::WasmCompileFail)));

    // Should NOT inject for execute operations
    let exec_fault = injector.should_inject("wasm_execute");
    assert!(exec_fault.is_none());
}

#[test]
fn test_custom_tool_fault_with_max_triggers() {
    let rng = DeterministicRng::new(42);
    let injector = FaultInjectorBuilder::new(rng)
        .with_fault(FaultConfig::new(FaultType::CustomToolExecFail, 1.0).max_triggers(3))
        .build();

    // First 3 should trigger
    assert!(injector.should_inject("custom_tool").is_some());
    assert!(injector.should_inject("custom_tool").is_some());
    assert!(injector.should_inject("custom_tool").is_some());

    // Fourth should NOT trigger (max_triggers reached)
    assert!(injector.should_inject("custom_tool").is_none());
}

// =============================================================================
// Combined Fault Scenarios
// =============================================================================

#[test]
fn test_combined_wasm_and_custom_tool_faults() {
    let rng = DeterministicRng::new(42);
    let injector = FaultInjectorBuilder::new(rng)
        .with_wasm_faults(0.1)
        .with_custom_tool_faults(0.1)
        .build();

    let stats = injector.stats();
    // 5 WASM faults + 3 custom tool faults = 8 total
    assert_eq!(stats.len(), 8);
}

#[test]
fn test_fault_injection_stats_tracking() {
    let rng = DeterministicRng::new(42);
    let injector = FaultInjectorBuilder::new(rng)
        .with_fault(FaultConfig::new(FaultType::WasmExecFail, 1.0))
        .build();

    // Initial stats
    let stats = injector.stats();
    assert_eq!(stats[0].trigger_count, 0);

    // Trigger faults
    for i in 0..5 {
        injector.should_inject(&format!("wasm_exec_{}", i));
    }

    // Check updated stats
    let stats = injector.stats();
    assert_eq!(stats[0].trigger_count, 5);
}

// =============================================================================
// DST Simulation Tests
// =============================================================================

#[test]
fn test_dst_wasm_fault_injection_in_simulation() {
    let sim_config = SimConfig::new(42);

    let result = Simulation::new(sim_config).run(|_env| async move {
        let rng = DeterministicRng::new(42);
        let injector = Arc::new(FaultInjectorBuilder::new(rng).with_wasm_faults(0.2).build());

        let mut compile_failures = 0;
        let mut exec_failures = 0;
        let mut total_operations = 0;

        for i in 0..100 {
            total_operations += 1;
            if let Some(fault) = injector.should_inject(&format!("wasm_operation_{}", i)) {
                match fault {
                    FaultType::WasmCompileFail => compile_failures += 1,
                    FaultType::WasmExecFail => exec_failures += 1,
                    _ => {}
                }
            }
        }

        // With 0.2 probability and 5 fault types, we expect some failures
        assert!(
            compile_failures > 0 || exec_failures > 0,
            "Expected some WASM faults to be injected"
        );
        assert!(
            total_operations == 100,
            "All operations should be processed"
        );

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

#[test]
fn test_dst_custom_tool_fault_injection_in_simulation() {
    let sim_config = SimConfig::new(12345);

    let result = Simulation::new(sim_config).run(|_env| async move {
        let rng = DeterministicRng::new(12345);
        let injector = Arc::new(
            FaultInjectorBuilder::new(rng)
                .with_custom_tool_faults(0.3)
                .build(),
        );

        let mut exec_failures = 0;
        let mut timeout_failures = 0;
        let mut sandbox_failures = 0;

        for i in 0..100 {
            if let Some(fault) = injector.should_inject(&format!("custom_tool_execute_{}", i)) {
                match fault {
                    FaultType::CustomToolExecFail => exec_failures += 1,
                    FaultType::CustomToolExecTimeout { .. } => timeout_failures += 1,
                    FaultType::CustomToolSandboxAcquireFail => sandbox_failures += 1,
                    _ => {}
                }
            }
        }

        // With 0.3 probability and 3 fault types, we expect some failures
        let total_failures = exec_failures + timeout_failures + sandbox_failures;
        assert!(
            total_failures > 0,
            "Expected some custom tool faults to be injected"
        );

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}

// =============================================================================
// Stress Test
// =============================================================================

#[test]
fn test_dst_high_load_fault_injection() {
    let sim_config = SimConfig::new(99999);

    let result = Simulation::new(sim_config).run(|_env| async move {
        let rng = DeterministicRng::new(99999);
        let injector = Arc::new(
            FaultInjectorBuilder::new(rng)
                .with_wasm_faults(0.05)
                .with_custom_tool_faults(0.05)
                .build(),
        );

        let mut fault_counts: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();

        // Simulate high-load scenario
        for i in 0..1000 {
            // WASM operations
            if let Some(fault) = injector.should_inject(&format!("wasm_op_{}", i)) {
                *fault_counts.entry(fault.name().to_string()).or_insert(0) += 1;
            }

            // Custom tool operations
            if let Some(fault) = injector.should_inject(&format!("custom_tool_op_{}", i)) {
                *fault_counts.entry(fault.name().to_string()).or_insert(0) += 1;
            }
        }

        // Verify operation count
        assert_eq!(injector.operation_count(), 2000); // 1000 wasm + 1000 custom

        // With 0.05 probability across 8 fault types (some with reduced probability),
        // total probability per operation is approximately:
        // WASM: 0.05*3 + 0.025 + 0.017 = 0.192
        // Custom: 0.05 + 0.025 + 0.017 = 0.092
        // Combined: ~0.23 per operation => ~460 faults over 2000 operations
        let total_faults: u64 = fault_counts.values().sum();
        assert!(
            total_faults > 300 && total_faults < 700,
            "Expected ~460 faults, got {}",
            total_faults
        );

        Ok(())
    });

    assert!(result.is_ok(), "Test failed: {:?}", result.err());
}
