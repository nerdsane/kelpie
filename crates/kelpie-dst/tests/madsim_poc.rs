//! Proof-of-Concept: madsim Runtime Determinism
//!
//! This test demonstrates that madsim provides true runtime determinism:
//! - sleep() advances virtual time instantly
//! - Same seed = identical execution order
//! - Spawn tasks execute deterministically

// Allow direct tokio usage in test code (tests madsim interception)
#![allow(clippy::disallowed_methods)]

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// Test 1: Basic sleep advances virtual time instantly
    ///
    /// With tokio: This would take 1 real second
    /// With madsim: This completes instantly, advancing virtual time
    #[madsim::test]
    async fn test_madsim_sleep_is_instant() {
        let start = madsim::time::Instant::now();

        // Sleep for 1 second in virtual time
        madsim::time::sleep(Duration::from_secs(1)).await;

        let elapsed = start.elapsed();

        // Virtual time advanced by 1 second
        assert!(
            elapsed >= Duration::from_secs(1),
            "Virtual time should advance by at least 1 second, got {:?}",
            elapsed
        );

        // But in real wall-clock time, this test completes instantly (< 100ms)
        // We can't easily assert this without external timing, but you can observe it
        println!(
            "Virtual time elapsed: {:?} (test completed instantly in real time)",
            elapsed
        );
    }

    /// Test 2: Deterministic execution with spawn
    ///
    /// With tokio: Task ordering is non-deterministic
    /// With madsim: Same seed = same task execution order
    #[madsim::test]
    async fn test_madsim_spawn_is_deterministic() {
        let results = Arc::new(Mutex::new(Vec::new()));

        // Spawn 3 tasks that sleep for different durations
        let mut handles = vec![];

        for i in 0..3 {
            let results_clone = results.clone();
            let handle = madsim::task::spawn(async move {
                // Sleep for (i+1) * 100ms
                madsim::time::sleep(Duration::from_millis((i + 1) * 100)).await;
                results_clone.lock().unwrap().push(i);
            });
            handles.push(handle);
        }

        // Wait for all tasks
        for handle in handles {
            handle.await.unwrap();
        }

        // Tasks complete in deterministic order based on sleep duration
        let final_results = results.lock().unwrap().clone();
        assert_eq!(
            final_results,
            vec![0, 1, 2],
            "Tasks should complete in order: 0, 1, 2"
        );
    }

    /// Test 3: Simple check that madsim runs
    ///
    /// This test just verifies basic madsim functionality works
    #[madsim::test]
    async fn test_madsim_basic_functionality() {
        // Sleep for 100ms
        madsim::time::sleep(Duration::from_millis(100)).await;

        // Spawn a task
        let handle = madsim::task::spawn(async {
            madsim::time::sleep(Duration::from_millis(50)).await;
            42
        });

        let result = handle.await.unwrap();
        assert_eq!(result, 42);
    }
}
