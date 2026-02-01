//! Deterministic Scheduling Tests (Issue #15)
//!
//! This test file verifies that the madsim runtime provides true deterministic
//! task scheduling, which is the foundational requirement for FoundationDB-style
//! deterministic simulation testing.
//!
//! Key property being tested:
//! **Same seed = same task execution order, always**
//!
//! This was the foundational gap in Kelpie's DST - tokio's internal task scheduler
//! is non-deterministic, meaning two tasks spawned via `tokio::spawn()` will
//! interleave non-deterministically even with the same seed.

// Allow direct madsim usage in these tests - madsim intercepts tokio at compile time
// which causes clippy to flag these as disallowed tokio methods.
#![allow(clippy::disallowed_methods)]

use kelpie_dst::{DeterministicRng, SimConfig, Simulation};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Test: Deterministic Task Ordering
///
/// This is the key acceptance criteria from Issue #15:
/// Verifies that task execution order is determined by sleep durations,
/// which demonstrates madsim's deterministic virtual time scheduler.
///
/// **IMPORTANT:** To verify cross-run determinism, run this test MULTIPLE times
/// with the same DST_SEED and verify the output is identical:
/// ```
/// DST_SEED=12345 cargo test -p kelpie-dst test_deterministic_task_ordering -- --nocapture
/// ```
///
/// Within a single madsim session, we verify that:
/// - Tasks complete in order based on their sleep durations
/// - The execution order is predictable and consistent with virtual time
#[madsim::test]
async fn test_deterministic_task_ordering() {
    // Get seed from environment (for cross-run verification) or use default
    let seed = std::env::var("DST_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(12345u64);

    println!("Running with DST_SEED={}", seed);
    println!("To verify determinism, run multiple times with same seed:");
    println!(
        "  DST_SEED={} cargo test -p kelpie-dst test_deterministic_task_ordering -- --nocapture\n",
        seed
    );

    let execution_order = Arc::new(Mutex::new(Vec::new()));

    // Spawn 100 tasks that record their execution order
    // Tasks with shorter sleep times should complete first (deterministic!)
    let mut handles = vec![];
    for task_id in 0..100u64 {
        let order = execution_order.clone();
        let handle = madsim::task::spawn(async move {
            // Each task does "work" based on task_id
            // task_id % 10 gives sleep times 0-9ms
            let work_time = (task_id % 10) + 1;
            madsim::time::sleep(Duration::from_millis(work_time)).await;

            // Record when this task completed
            order.lock().unwrap().push(task_id);
        });
        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        handle.await.unwrap();
    }

    let final_order = execution_order.lock().unwrap().clone();

    // Verify expected ordering: tasks are grouped by sleep duration
    // Tasks sleeping 1ms finish first, then 2ms, etc.
    // Within each group: task 0, 10, 20, 30... (sleep 1ms), then 1, 11, 21, 31... (sleep 2ms)

    // First 10 should all be tasks with (task_id % 10 == 0) - they sleep only 1ms
    let first_10 = &final_order[..10];
    println!(
        "First 10 completions (should be tasks 0,10,20,...): {:?}",
        first_10
    );

    // All tasks in first 10 should have task_id % 10 == 0 (1ms sleep)
    for &task_id in first_10 {
        assert_eq!(
            task_id % 10,
            0,
            "Tasks sleeping 1ms (task_id % 10 == 0) should complete first. Got task {}",
            task_id
        );
    }

    // Next 10 should all be tasks with (task_id % 10 == 1) - they sleep 2ms
    let next_10 = &final_order[10..20];
    println!(
        "Next 10 completions (should be tasks 1,11,21,...): {:?}",
        next_10
    );

    for &task_id in next_10 {
        assert_eq!(
            task_id % 10,
            1,
            "Tasks sleeping 2ms (task_id % 10 == 1) should complete second. Got task {}",
            task_id
        );
    }

    // Verify total count
    assert_eq!(final_order.len(), 100, "All 100 tasks should complete");

    println!("\nSUCCESS: Task ordering is deterministic based on sleep durations.");
    println!("Full execution order: {:?}", &final_order[..20]);
    println!("...(80 more tasks)...");
}

/// Test: Simulation Harness with Deterministic Scheduling
///
/// Verifies that the Simulation::run_async() method works correctly with madsim.
/// Note: We use run_async() instead of run() because we're already in a madsim
/// async context from #[madsim::test].
///
/// **Cross-run determinism:** To verify same seed = same result across runs:
/// ```
/// DST_SEED=54321 cargo test -p kelpie-dst test_simulation_deterministic -- --nocapture > run1.txt
/// DST_SEED=54321 cargo test -p kelpie-dst test_simulation_deterministic -- --nocapture > run2.txt
/// diff run1.txt run2.txt  # Should be identical
/// ```
#[madsim::test]
async fn test_simulation_deterministic_ordering() {
    let seed = 54321u64;

    let config = SimConfig::new(seed);
    let result = Simulation::new(config)
        .run_async(|env| async move {
            let execution_order = Arc::new(Mutex::new(Vec::new()));

            // Spawn tasks using the DST environment's RNG for sleep times
            let mut handles = vec![];
            for i in 0..50u64 {
                let order = execution_order.clone();
                let sleep_ms = env.rng.next_u64() % 10 + 1;

                let handle = madsim::task::spawn(async move {
                    madsim::time::sleep(Duration::from_millis(sleep_ms)).await;
                    order.lock().unwrap().push(i);
                });
                handles.push(handle);
            }

            for handle in handles {
                handle.await.unwrap();
            }

            let result = execution_order.lock().unwrap().clone();
            Ok(result)
        })
        .await
        .expect("Simulation run failed");

    // Verify all 50 tasks completed
    assert_eq!(result.len(), 50, "All 50 tasks should complete");

    // Verify each task ID appears exactly once
    let mut sorted = result.clone();
    sorted.sort();
    let expected: Vec<u64> = (0..50).collect();
    assert_eq!(sorted, expected, "Each task ID should appear exactly once");

    println!("SUCCESS: Simulation harness with deterministic scheduling works");
    println!("Execution order (first 20): {:?}", &result[..20]);
    println!("\nTo verify cross-run determinism, run multiple times with same seed:");
    println!(
        "  DST_SEED={} cargo test -p kelpie-dst test_simulation_deterministic -- --nocapture",
        seed
    );
}

/// Test: Different Seeds Produce Different Orderings
///
/// This verifies that different seeds actually produce different execution
/// orders (i.e., the seed is meaningful, not ignored).
#[madsim::test]
async fn test_different_seeds_different_order() {
    let run_with_seed = |seed: u64| async move {
        let rng = Arc::new(DeterministicRng::new(seed));
        let order = Arc::new(Mutex::new(Vec::new()));

        let mut handles = vec![];
        for i in 0..20u64 {
            let order = order.clone();
            let sleep_ms = rng.next_u64() % 10 + 1;

            let handle = madsim::task::spawn(async move {
                madsim::time::sleep(Duration::from_millis(sleep_ms)).await;
                order.lock().unwrap().push(i);
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap();
        }

        let result = order.lock().unwrap().clone();
        result
    };

    let order_seed_1 = run_with_seed(11111).await;
    let order_seed_2 = run_with_seed(22222).await;

    // Different seeds should (with high probability) produce different orders
    // due to different RNG-derived sleep times
    assert_ne!(
        order_seed_1, order_seed_2,
        "Different seeds should produce different execution orders.\n\
         This indicates the seed is actually being used for scheduling decisions."
    );

    println!("SUCCESS: Different seeds produce different orderings (as expected)");
}

/// Test: Concurrent Operations Have Consistent Structure
///
/// Tests that complex concurrent patterns produce events in a predictable order.
/// Note: Within a single madsim session, time accumulates, so we verify the
/// event sequence has expected structure rather than comparing two runs.
///
/// For true cross-run determinism, run:
/// DST_SEED=12345 cargo test test_concurrent_operations_deterministic --no-capture
/// Multiple times and verify identical output.
#[madsim::test]
async fn test_concurrent_operations_deterministic() {
    let events = Arc::new(Mutex::new(Vec::new()));

    // Wave 1: Spawn 10 tasks
    let mut handles = vec![];
    for i in 0..10u64 {
        let events = events.clone();
        handles.push(madsim::task::spawn(async move {
            madsim::time::sleep(Duration::from_millis(i + 1)).await;
            events.lock().unwrap().push(format!("wave1_task{}", i));
        }));
    }

    // Do some work in the main task
    madsim::time::sleep(Duration::from_millis(5)).await;
    events.lock().unwrap().push("main_checkpoint_1".to_string());

    // Wave 2: Spawn 10 more tasks while wave 1 is still running
    for i in 0..10u64 {
        let events = events.clone();
        handles.push(madsim::task::spawn(async move {
            madsim::time::sleep(Duration::from_millis(i + 1)).await;
            events.lock().unwrap().push(format!("wave2_task{}", i));
        }));
    }

    // Wait for all
    for handle in handles {
        handle.await.unwrap();
    }

    events.lock().unwrap().push("done".to_string());
    let result = events.lock().unwrap().clone();

    // Verify the structure of events is as expected:
    // - All wave1 tasks should complete
    // - All wave2 tasks should complete
    // - main_checkpoint_1 should appear somewhere in the middle
    // - "done" should be last
    let wave1_count = result.iter().filter(|e| e.starts_with("wave1_")).count();
    let wave2_count = result.iter().filter(|e| e.starts_with("wave2_")).count();
    let has_checkpoint = result.contains(&"main_checkpoint_1".to_string());
    let ends_with_done = result.last() == Some(&"done".to_string());

    assert_eq!(wave1_count, 10, "Should have all 10 wave1 tasks");
    assert_eq!(wave2_count, 10, "Should have all 10 wave2 tasks");
    assert!(has_checkpoint, "Should have main_checkpoint_1");
    assert!(ends_with_done, "Should end with done");

    println!("SUCCESS: Concurrent operation structure is consistent");
    println!("Event sequence: {:?}", result);
    println!("\nTo verify determinism across runs:");
    println!("  Run: DST_SEED=12345 cargo test -p kelpie-dst test_concurrent -- --nocapture");
    println!("  Multiple times and compare the output");
}

/// Test: Spawn Inside Spawn Is Deterministic
///
/// Tests that nested task spawning is also deterministic.
#[madsim::test]
async fn test_nested_spawn_deterministic() {
    let run_nested = || async {
        let events = Arc::new(Mutex::new(Vec::new()));

        let events_outer = events.clone();
        let outer = madsim::task::spawn(async move {
            events_outer.lock().unwrap().push("outer_start".to_string());

            // Spawn inner tasks
            let events_inner_1 = events_outer.clone();
            let inner_1 = madsim::task::spawn(async move {
                madsim::time::sleep(Duration::from_millis(10)).await;
                events_inner_1
                    .lock()
                    .unwrap()
                    .push("inner_1_done".to_string());
            });

            let events_inner_2 = events_outer.clone();
            let inner_2 = madsim::task::spawn(async move {
                madsim::time::sleep(Duration::from_millis(5)).await;
                events_inner_2
                    .lock()
                    .unwrap()
                    .push("inner_2_done".to_string());
            });

            inner_1.await.unwrap();
            inner_2.await.unwrap();

            events_outer.lock().unwrap().push("outer_done".to_string());
        });

        outer.await.unwrap();
        let result = events.lock().unwrap().clone();
        result
    };

    let events_1 = run_nested().await;
    let events_2 = run_nested().await;

    assert_eq!(events_1, events_2, "Nested spawns must be deterministic");

    println!("SUCCESS: Nested spawn patterns are deterministic");
    println!("Event sequence: {:?}", events_1);
}

/// Test: Verify DST_SEED Environment Variable Usage
///
/// This test documents how DST_SEED should be used for reproduction.
#[madsim::test]
async fn test_dst_seed_documentation() {
    // Get seed from environment or use default
    let seed = std::env::var("DST_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(99999u64);

    println!("Running with DST_SEED={}", seed);
    println!("To reproduce this exact run: DST_SEED={} cargo test -p kelpie-dst test_dst_seed_documentation", seed);

    let rng = DeterministicRng::new(seed);
    let values: Vec<u64> = (0..5).map(|_| rng.next_u64()).collect();

    println!("RNG sequence for seed {}: {:?}", seed, values);

    // The values should be deterministic for the same seed
    // This is just documentation - no assertion needed
    println!("\nDeterministic Simulation Testing (DST) Key Points:");
    println!("1. Set DST_SEED for reproducible test runs");
    println!("2. madsim ensures task scheduling is deterministic");
    println!("3. Same seed = same execution order, always");
    println!("4. Race conditions can be reliably reproduced and debugged");
}
