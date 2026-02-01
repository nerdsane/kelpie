# DST (Deterministic Simulation Testing)

## Core Principles

1. **All randomness flows from a single seed** - set `DST_SEED` to reproduce
2. **Simulated time** - `SimClock` replaces wall clock
3. **Explicit fault injection** - 16+ fault types with configurable probability
4. **Deterministic network** - `SimNetwork` with partitions, delays, reordering
5. **Deterministic task scheduling** - madsim provides consistent task interleaving
6. **All I/O through injected providers** - See below

## Deterministic Task Scheduling (Issue #15)

The `madsim` feature is **enabled by default** for `kelpie-dst`, ensuring true deterministic
task scheduling. Unlike tokio's scheduler (which is non-deterministic), madsim guarantees:

- **Same seed = same task interleaving order**
- Race conditions can be reliably reproduced
- `DST_SEED=12345 cargo test -p kelpie-dst` produces identical results every time

## I/O Abstraction Requirements (MANDATORY)

**All time and random operations MUST use injected providers, not direct calls.**

```rust
// ❌ FORBIDDEN - Breaks DST determinism
tokio::time::sleep(Duration::from_secs(1)).await;
let now = std::time::SystemTime::now();
let random_val = rand::random::<u64>();

// ✅ CORRECT - Uses injected providers
time_provider.sleep_ms(1000).await;
let now = time_provider.now_ms();
let random_val = rng_provider.next_u64();
```

**Forbidden Patterns:**

| Pattern | Use Instead |
|---------|-------------|
| `tokio::time::sleep(dur)` | `time_provider.sleep_ms(ms)` |
| `std::thread::sleep(dur)` | `time_provider.sleep_ms(ms)` |
| `SystemTime::now()` | `time_provider.now_ms()` |
| `Instant::now()` | `time_provider.monotonic_ms()` |
| `rand::random()` | `rng_provider.next_u64()` |
| `thread_rng()` | `rng_provider` |

**CI Enforcement:**

The `scripts/check-determinism.sh` script scans for these patterns and fails CI on violations.

```bash
# Run locally before committing
./scripts/check-determinism.sh

# Warn-only mode (doesn't fail)
./scripts/check-determinism.sh --warn-only
```

**Allowed Exceptions:**

- `kelpie-core/src/io.rs` - Production TimeProvider/RngProvider implementations
- `kelpie-core/src/runtime.rs` - Production runtime
- `kelpie-dst/` - DST framework (needs real time for comparison)
- `kelpie-vm/`, `kelpie-sandbox/` - Real VM interactions
- `kelpie-cli/`, `kelpie-tools/` - CLI tools run in production
- `kelpie-cluster/` - Cluster heartbeats/gossip
- Test files (`*_test.rs`, `tests/*.rs`, `#[cfg(test)]` blocks)

**See:** `crates/kelpie-core/src/io.rs` for `TimeProvider` and `RngProvider` traits.

## Running DST Tests

```bash
# Run with random seed (logged for reproduction)
cargo test -p kelpie-dst

# Reproduce specific run
DST_SEED=12345 cargo test -p kelpie-dst

# Verify determinism across runs
DST_SEED=12345 cargo test -p kelpie-dst test_name -- --nocapture > run1.txt
DST_SEED=12345 cargo test -p kelpie-dst test_name -- --nocapture > run2.txt
diff run1.txt run2.txt  # Should be identical

# Stress test (longer, more iterations)
cargo test -p kelpie-dst stress --release -- --ignored
```

## Writing DST Tests

**Recommended pattern: Use `#[madsim::test]` for deterministic scheduling:**

```rust
use std::time::Duration;

#[madsim::test]
async fn test_concurrent_operations() {
    // Spawn tasks - ordering is deterministic based on sleep durations!
    let handle1 = madsim::task::spawn(async {
        madsim::time::sleep(Duration::from_millis(10)).await;
        "task1"
    });

    let handle2 = madsim::task::spawn(async {
        madsim::time::sleep(Duration::from_millis(5)).await;
        "task2"
    });

    // task2 completes first (deterministically!) due to shorter sleep
    let result2 = handle2.await.unwrap();
    let result1 = handle1.await.unwrap();

    assert_eq!(result2, "task2");
    assert_eq!(result1, "task1");
}
```

**Using the Simulation harness with fault injection:**

```rust
use kelpie_dst::{Simulation, SimConfig, FaultConfig, FaultType};

#[madsim::test]
async fn test_actor_under_faults() {
    let config = SimConfig::from_env_or_random();

    // Use run_async() when inside #[madsim::test]
    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.1))
        .with_fault(FaultConfig::new(FaultType::NetworkPacketLoss, 0.05))
        .run_async(|env| async move {
            // Test logic using env.storage, env.network, env.clock
            env.storage.write(b"key", b"value").await?;

            // Advance simulated time
            env.advance_time_ms(1000);

            // Verify invariants
            let value = env.storage.read(b"key").await?;
            assert_eq!(value, Some(Bytes::from("value")));

            Ok(())
        })
        .await;

    assert!(result.is_ok());
}
```

## Fault Types

| Category | Fault Types |
|----------|-------------|
| Storage | `StorageWriteFail`, `StorageReadFail`, `StorageCorruption`, `StorageLatency`, `DiskFull` |
| Storage Semantics (FDB-critical) | `StorageMisdirectedWrite`, `StoragePartialWrite`, `StorageFsyncFail`, `StorageUnflushedLoss` |
| Crash | `CrashBeforeWrite`, `CrashAfterWrite`, `CrashDuringTransaction` |
| Network | `NetworkPartition`, `NetworkDelay`, `NetworkPacketLoss`, `NetworkMessageReorder` |
| Network Infrastructure (FDB-critical) | `NetworkPacketCorruption`, `NetworkJitter`, `NetworkConnectionExhaustion` |
| Time | `ClockSkew`, `ClockJump` |
| Resource | `OutOfMemory`, `CPUStarvation`, `ResourceFdExhaustion` |
| Distributed Coordination (FDB-critical) | `ClusterSplitBrain`, `ReplicationLag`, `QuorumLoss` |

## Test Categories

Kelpie has two types of tests with distinct purposes and characteristics:

### True DST Tests (`*_dst.rs`)

**Characteristics:**
- Fully deterministic (same seed = same result)
- Use `Simulation` harness or DST components (SimStorage, SimClock, DeterministicRng)
- No external dependencies or uncontrolled systems
- Instant execution (virtual time, no real I/O)
- Reproducible with `DST_SEED` environment variable

**Examples:**
- `actor_lifecycle_dst.rs` - Actor state machine tests
- `memory_dst.rs` - Memory system tests
- `integration_chaos_dst.rs` - Many faults simultaneously (still deterministic!)

**When to use:** Testing distributed system logic, fault handling, race conditions, state machines

### Chaos Tests (`*_chaos.rs`)

**Characteristics:**
- Non-deterministic (depend on external system state)
- Integrate with uncontrolled external systems
- Real I/O (slower)
- Harder to reproduce (external dependencies)
- Provide value for integration testing

**Examples:**
- `vm_backend_firecracker_chaos.rs` - Real Firecracker VM integration
- Tests using real network calls to external APIs
- Tests spawning external processes (git, shell, etc.)

**When to use:** Integration testing with real external systems that can't be fully mocked

**Note:** "Chaos" in test names like `integration_chaos_dst.rs` refers to **chaos engineering** (many simultaneous faults), not non-deterministic execution. These are still DST tests!

**Rule of thumb:** If it uses `Simulation` or DST components (SimStorage, SimClock, etc.), it's a DST test. If it requires real Firecracker, real network, or real external binaries, it's a Chaos test.
