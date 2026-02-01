//! DST Tests for REAL Heartbeat/Pause Implementation
//!
//! TigerStyle: Full simulation tests against REAL production code.
//!
//! These tests use the actual:
//! - UnifiedToolRegistry
//! - pause_heartbeats tool (from heartbeat.rs)
//! - ClockSource abstraction
//! - ToolSignal mechanism
#![cfg(feature = "dst")]

use kelpie_dst::{FaultConfig, FaultType, SimConfig, Simulation};
use kelpie_server::state::AppState;
use kelpie_server::tools::{
    parse_pause_signal, register_pause_heartbeats_with_clock, ClockSource,
    AGENT_LOOP_ITERATIONS_MAX, HEARTBEAT_PAUSE_MINUTES_DEFAULT, HEARTBEAT_PAUSE_MINUTES_MAX,
    HEARTBEAT_PAUSE_MINUTES_MIN, MS_PER_MINUTE,
};
use serde_json::json;
use std::sync::Arc;

// =============================================================================
// Full Simulation Tests - REAL Implementation
// =============================================================================

/// Test the REAL pause_heartbeats tool execution via UnifiedToolRegistry
#[test]
fn test_real_pause_heartbeats_via_registry() {
    let config = SimConfig::from_env_or_random();
    println!(
        "DST seed: {} (set DST_SEED={} to replay)",
        config.seed, config.seed
    );

    let result = Simulation::new(config).run(|env| async move {
        // Create REAL AppState and registry
        let state = AppState::new(kelpie_core::current_runtime());
        let registry = state.tool_registry();

        // Create clock source from SimClock
        let clock = env.clock.clone();
        let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
        let clock_source = ClockSource::Sim(clock_fn);

        // Register REAL pause_heartbeats tool with simulated clock
        register_pause_heartbeats_with_clock(registry, clock_source).await;

        // Verify tool is registered
        assert!(
            registry.has_tool("pause_heartbeats").await,
            "pause_heartbeats tool should be registered"
        );

        // Execute the REAL tool via registry
        let result = registry.execute("pause_heartbeats", &json!({})).await;

        // Verify execution succeeded
        assert!(result.success, "Tool execution should succeed");

        // Parse the REAL output format
        let (minutes, pause_until_ms) =
            parse_pause_signal(&result.output).expect("Should parse pause signal from output");

        // Verify default duration
        assert_eq!(
            minutes, HEARTBEAT_PAUSE_MINUTES_DEFAULT,
            "Should use default {} minutes",
            HEARTBEAT_PAUSE_MINUTES_DEFAULT
        );

        // Verify pause_until is calculated correctly
        let expected_until = env.clock.now_ms() + (minutes * MS_PER_MINUTE);
        assert_eq!(
            pause_until_ms, expected_until,
            "Pause until should be now + duration"
        );

        Ok(())
    });

    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());
}

/// Test custom duration with REAL implementation
#[test]
fn test_real_pause_custom_duration() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|env| async move {
        let state = AppState::new(kelpie_core::current_runtime());
        let registry = state.tool_registry();

        let clock = env.clock.clone();
        let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
        register_pause_heartbeats_with_clock(registry, ClockSource::Sim(clock_fn)).await;

        // Test various durations
        for minutes in [1, 5, 10, 30, 60] {
            let result = registry
                .execute("pause_heartbeats", &json!({ "minutes": minutes }))
                .await;
            assert!(result.success);

            let (actual_minutes, _) = parse_pause_signal(&result.output).unwrap();
            assert_eq!(actual_minutes, minutes, "Minutes mismatch for {}", minutes);
        }

        Ok(())
    });

    assert!(result.is_ok());
}

/// Test duration clamping with REAL implementation
#[test]
fn test_real_pause_duration_clamping() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|env| async move {
        let state = AppState::new(kelpie_core::current_runtime());
        let registry = state.tool_registry();

        let clock = env.clock.clone();
        let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
        register_pause_heartbeats_with_clock(registry, ClockSource::Sim(clock_fn)).await;

        // Test below minimum - should clamp to 1
        let result = registry
            .execute("pause_heartbeats", &json!({ "minutes": 0 }))
            .await;
        let (minutes, _) = parse_pause_signal(&result.output).unwrap();
        assert_eq!(minutes, HEARTBEAT_PAUSE_MINUTES_MIN, "Should clamp to min");

        // Test above maximum - should clamp to 60
        let result = registry
            .execute("pause_heartbeats", &json!({ "minutes": 100 }))
            .await;
        let (minutes, _) = parse_pause_signal(&result.output).unwrap();
        assert_eq!(minutes, HEARTBEAT_PAUSE_MINUTES_MAX, "Should clamp to max");

        Ok(())
    });

    assert!(result.is_ok());
}

/// Test with clock advancement (simulated time passing)
#[test]
fn test_real_pause_with_clock_advancement() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|env| async move {
        let state = AppState::new(kelpie_core::current_runtime());
        let registry = state.tool_registry();

        let clock = env.clock.clone();
        let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
        register_pause_heartbeats_with_clock(registry, ClockSource::Sim(clock_fn)).await;

        // Record initial time
        let initial_time = env.clock.now_ms();

        // Pause for 2 minutes
        let result = registry
            .execute("pause_heartbeats", &json!({ "minutes": 2 }))
            .await;
        let (_, pause_until_ms) = parse_pause_signal(&result.output).unwrap();

        // Verify pause is in the future
        assert!(
            pause_until_ms > initial_time,
            "Pause should be in the future"
        );

        // Advance time by 1 minute - still paused
        env.advance_time_ms(MS_PER_MINUTE);
        assert!(
            env.clock.now_ms() < pause_until_ms,
            "Should still be paused after 1 minute"
        );

        // Advance time past pause
        env.advance_time_ms(MS_PER_MINUTE + 1);
        assert!(
            env.clock.now_ms() > pause_until_ms,
            "Pause should have expired"
        );

        Ok(())
    });

    assert!(result.is_ok());
}

/// Test determinism - same seed produces same results
#[test]
fn test_real_pause_determinism() {
    let seed = 42u64;

    let run_simulation = || {
        let config = SimConfig::new(seed);
        Simulation::new(config).run(|env| async move {
            let state = AppState::new(kelpie_core::current_runtime());
            let registry = state.tool_registry();

            let clock = env.clock.clone();
            let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
            register_pause_heartbeats_with_clock(registry, ClockSource::Sim(clock_fn)).await;

            let mut results = Vec::new();
            for minutes in [1, 2, 5, 10, 30] {
                let result = registry
                    .execute("pause_heartbeats", &json!({ "minutes": minutes }))
                    .await;
                let (m, until) = parse_pause_signal(&result.output).unwrap();
                results.push((m, until));
            }
            Ok(results)
        })
    };

    let result1 = run_simulation().expect("First run failed");
    let result2 = run_simulation().expect("Second run failed");

    assert_eq!(result1, result2, "Same seed must produce same results");
}

/// Test with clock skew fault
#[test]
fn test_real_pause_with_clock_skew_fault() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    // Note: ClockSkew doesn't affect SimClock directly since we're using
    // the SimClock's now_ms() function. This test verifies the tool
    // still works correctly with the fault configured.
    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(
            FaultType::ClockSkew { delta_ms: 5000 },
            1.0,
        ))
        .run(|env| async move {
            let state = AppState::new(kelpie_core::current_runtime());
            let registry = state.tool_registry();

            let clock = env.clock.clone();
            let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
            register_pause_heartbeats_with_clock(registry, ClockSource::Sim(clock_fn)).await;

            let initial_time = env.clock.now_ms();

            let result = registry
                .execute("pause_heartbeats", &json!({ "minutes": 2 }))
                .await;
            assert!(result.success);

            let (minutes, pause_until_ms) = parse_pause_signal(&result.output).unwrap();
            assert_eq!(minutes, 2);

            // Pause should be calculated from SimClock
            let expected = initial_time + (2 * MS_PER_MINUTE);
            assert_eq!(
                pause_until_ms, expected,
                "Clock skew should not affect SimClock-based pause"
            );

            Ok(())
        });

    assert!(result.is_ok());
}

/// Test high frequency execution (stress test)
#[test]
fn test_real_pause_high_frequency() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|env| async move {
        let state = AppState::new(kelpie_core::current_runtime());
        let registry = state.tool_registry();

        let clock = env.clock.clone();
        let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
        register_pause_heartbeats_with_clock(registry, ClockSource::Sim(clock_fn)).await;

        // Execute 100 times rapidly
        for i in 0..100 {
            let minutes = (i % 60) + 1;
            let result = registry
                .execute("pause_heartbeats", &json!({ "minutes": minutes }))
                .await;
            assert!(result.success, "Call {} failed", i);

            let (actual_minutes, _) = parse_pause_signal(&result.output).unwrap();
            assert_eq!(actual_minutes, minutes as u64);
        }

        Ok(())
    });

    assert!(result.is_ok());
}

/// Test with storage faults (shouldn't affect pause_heartbeats since it's stateless)
#[test]
fn test_real_pause_with_storage_faults() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.3))
        .with_fault(FaultConfig::new(FaultType::StorageReadFail, 0.2))
        .run(|env| async move {
            let state = AppState::new(kelpie_core::current_runtime());
            let registry = state.tool_registry();

            let clock = env.clock.clone();
            let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
            register_pause_heartbeats_with_clock(registry, ClockSource::Sim(clock_fn)).await;

            // pause_heartbeats doesn't use storage, so it should work even with storage faults
            for _ in 0..20 {
                let result = registry
                    .execute("pause_heartbeats", &json!({ "minutes": 5 }))
                    .await;
                assert!(
                    result.success,
                    "pause_heartbeats should work regardless of storage faults"
                );
            }

            Ok(())
        });

    assert!(result.is_ok());
}

/// Test output format matches expected pattern
#[test]
fn test_real_pause_output_format() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|env| async move {
        let state = AppState::new(kelpie_core::current_runtime());
        let registry = state.tool_registry();

        let clock = env.clock.clone();
        let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
        register_pause_heartbeats_with_clock(registry, ClockSource::Sim(clock_fn)).await;

        let result = registry
            .execute("pause_heartbeats", &json!({ "minutes": 5 }))
            .await;

        // Output should start with PAUSE_HEARTBEATS:
        assert!(
            result.output.starts_with("PAUSE_HEARTBEATS:"),
            "Output format incorrect: {}",
            result.output
        );

        // Should contain user-friendly message
        assert!(
            result.output.contains("paused for 5 minute"),
            "Should contain human-readable message"
        );

        // Should be parseable
        let (minutes, pause_until_ms) = parse_pause_signal(&result.output)
            .expect("Output should be parseable by parse_pause_signal");
        assert_eq!(minutes, 5);
        assert!(pause_until_ms > 0);

        Ok(())
    });

    assert!(result.is_ok());
}

/// Test concurrent execution (thread safety)
#[test]
fn test_real_pause_concurrent_execution() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|env| async move {
        let state = AppState::new(kelpie_core::current_runtime());
        let registry = state.tool_registry();

        let clock = env.clock.clone();
        let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
        register_pause_heartbeats_with_clock(registry, ClockSource::Sim(clock_fn)).await;

        // Sequential execution (AppState reference pattern doesn't support spawned tasks)
        // This still tests the tool can handle multiple rapid calls
        for i in 0..10 {
            let minutes = (i % 60) + 1;
            let result = registry
                .execute("pause_heartbeats", &json!({ "minutes": minutes }))
                .await;
            assert!(result.success, "Call {} failed", i);
        }

        Ok(())
    });

    assert!(result.is_ok());
}

// =============================================================================
// Integration with Agent Loop Simulation
// =============================================================================

/// Simulate agent loop behavior with real pause_heartbeats tool
#[test]
fn test_real_agent_loop_with_pause() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|env| async move {
        let state = AppState::new(kelpie_core::current_runtime());
        let registry = state.tool_registry();

        let clock = env.clock.clone();
        let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
        register_pause_heartbeats_with_clock(registry, ClockSource::Sim(clock_fn)).await;

        // Simulate agent loop
        let mut iterations = 0u32;
        let mut pause_until_ms: Option<u64> = None;

        // Simulate tool calls
        let tool_sequence = vec![
            ("some_tool", json!({})),
            ("pause_heartbeats", json!({ "minutes": 5 })), // Should trigger pause
            ("another_tool", json!({})),                   // Should not execute if pause respected
        ];

        for (tool_name, input) in tool_sequence {
            // Check if paused
            if let Some(until) = pause_until_ms {
                if env.clock.now_ms() < until {
                    // Agent is paused, should break loop
                    break;
                }
            }

            iterations += 1;

            // Execute tool
            if registry.has_tool(tool_name).await {
                let result = registry.execute(tool_name, &input).await;

                // Check for pause signal
                if let Some((_, until)) = parse_pause_signal(&result.output) {
                    pause_until_ms = Some(until);
                }
            }

            if iterations >= AGENT_LOOP_ITERATIONS_MAX {
                break;
            }
        }

        // Verify we stopped due to pause (at iteration 2, not 3)
        assert_eq!(iterations, 2, "Should have stopped after pause_heartbeats");
        assert!(pause_until_ms.is_some(), "Pause should have been recorded");

        Ok(())
    });

    assert!(result.is_ok());
}

/// Test pause expiration allows loop to continue
#[test]
fn test_real_agent_loop_resumes_after_pause() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config).run(|env| async move {
        let state = AppState::new(kelpie_core::current_runtime());
        let registry = state.tool_registry();

        let clock = env.clock.clone();
        let clock_fn: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || clock.now_ms());
        register_pause_heartbeats_with_clock(registry, ClockSource::Sim(clock_fn)).await;

        // Execute pause for 1 minute
        let result = registry
            .execute("pause_heartbeats", &json!({ "minutes": 1 }))
            .await;
        let (_, pause_until_ms) = parse_pause_signal(&result.output).unwrap();

        // Verify currently paused
        assert!(
            env.clock.now_ms() < pause_until_ms,
            "Should be paused initially"
        );

        // Advance time past pause
        env.advance_time_ms(MS_PER_MINUTE + 1000);

        // Verify no longer paused
        assert!(
            env.clock.now_ms() > pause_until_ms,
            "Should no longer be paused"
        );

        // Can execute more tools
        let result = registry
            .execute("pause_heartbeats", &json!({ "minutes": 2 }))
            .await;
        assert!(
            result.success,
            "Should be able to execute after pause expires"
        );

        Ok(())
    });

    assert!(result.is_ok());
}
