# Project Constraints

**Type:** STABLE
**Created:** 2026-01-12
**Last Updated:** 2026-01-12

---

## Purpose

This document defines non-negotiable rules for Kelpie development. These are outcomes we must achieve, not prescriptions for how to achieve them.

---

## Core Constraints

### 1. Simulation-First Development (MANDATORY)

Every feature MUST be developed and tested through the deterministic simulation harness BEFORE being considered complete.

**What Simulation-First IS:**
- Running the **full system** in a deterministic simulation harness
- Injecting faults (network failures, disk errors, timeouts, crashes, service unavailability)
- Stress testing (high concurrency, large data volumes, resource exhaustion)
- Storm testing (chaos scenarios, cascading failures)
- Finding bugs **before** production code is "done"
- Verifying determinism (same seed = same outcome, always)

**What Simulation-First is NOT:**
- Just adding unit tests (useful, but not the same)
- Creating mocks/fakes for testing (useful, but not the same)
- Integration tests against real services (useful, but not the same)
- "Good test coverage" (necessary, but not sufficient)

**The Rule:** If you cannot run the kelpie-dst simulation harness with fault injection against your feature, you are not doing simulation-first development.

**Harness Extension Rule:** If the feature requires fault types the harness doesn't support:
1. **STOP** implementation work
2. **Extend the harness** to support those faults FIRST
3. **THEN** implement the feature
4. **THEN** test through simulation with fault injection

**The Simulation-First Workflow:**

```
┌─────────────────────────────────────────────────────────────────┐
│  1. HARNESS CHECK                                               │
│     Does the simulation harness support the faults this         │
│     feature will encounter? (network, disk, timeouts, etc.)     │
│                                                                 │
│     NO → Extend harness FIRST, then continue                    │
│     YES → Continue                                              │
├─────────────────────────────────────────────────────────────────┤
│  2. WRITE SIMULATION TEST                                       │
│     Write the DST test that exercises the feature under:        │
│     - Normal conditions                                         │
│     - Fault injection (e.g., 30% failure rate)                  │
│     - Stress/storm conditions (high concurrency)                │
│     - Edge cases (empty inputs, max sizes, etc.)                │
│                                                                 │
│     This test will FAIL initially (feature doesn't exist)       │
├─────────────────────────────────────────────────────────────────┤
│  3. IMPLEMENT FEATURE                                           │
│     Write the actual production code                            │
├─────────────────────────────────────────────────────────────────┤
│  4. RUN SIMULATION                                              │
│     Execute with multiple seeds, find failures                  │
│     The simulation WILL find bugs you didn't anticipate         │
├─────────────────────────────────────────────────────────────────┤
│  5. FIX & ITERATE                                               │
│     Fix bugs found by simulation                                │
│     Re-run until deterministically passing                      │
├─────────────────────────────────────────────────────────────────┤
│  6. VERIFY DETERMINISM                                          │
│     Same seed = same behavior, always                           │
└─────────────────────────────────────────────────────────────────┘
```

**Example - Right vs Wrong:**

```rust
// WRONG: Unit test with mocks (useful, but NOT simulation-first)
#[test]
fn test_actor_state_persistence() {
    let mock_storage = MockStorage::new();
    let actor = Actor::new(mock_storage);
    actor.set_state("key", "value").unwrap();
    assert_eq!(actor.get_state("key"), Some("value"));
}

// RIGHT: DST harness with fault injection (this IS simulation-first)
#[test]
fn test_actor_state_persistence_with_storage_faults() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.1))
        .with_fault(FaultConfig::new(FaultType::CrashAfterWrite, 0.05))
        .run(|env| async move {
            let actor = Actor::new(env.actor_id, env.storage.clone());

            // Feature works EVEN when storage fails 10% of the time
            // and crashes happen 5% of the time
            actor.set_state("key", "value").await?;

            // Simulate crash and restart
            drop(actor);
            let actor = Actor::new(env.actor_id, env.storage.clone());

            // State persists through crashes
            let value = actor.get_state("key").await?;
            assert_eq!(value, Some("value".to_string()));

            Ok(())
        })
        .await;

    assert!(result.is_ok(), "Actor state must persist through faults");
}
```

**Enforcement Checklist:**
- [ ] Feature has DST test using simulation harness (see kelpie-dst crate)
- [ ] Test includes fault injection for relevant failure modes
- [ ] Test runs with multiple seeds (use `DST_SEED` env var)
- [ ] Test verifies behavior under stress
- [ ] Same seed produces identical results (determinism verified)

**Kelpie-Specific DST Commands:**
```bash
# Run DST tests with random seed
cargo test -p kelpie-dst

# Reproduce specific failure
DST_SEED=12345 cargo test -p kelpie-dst

# Stress test (longer, more iterations)
cargo test -p kelpie-dst stress --release -- --ignored
```

---

### 2. Supplementary Testing (Encouraged)

Other forms of testing are valuable and encouraged as **supplements** to simulation-first:

- **Unit tests** - Fast, focused tests for individual functions/modules
- **Mocks/fakes** - Useful for isolating components in unit tests
- **Integration tests** - Test real workflows with actual services
- **Property-based tests** - Discover edge cases through randomized inputs
- **End-to-end tests** - Verify complete user journeys

These forms of testing provide value and should be used where appropriate. They are **not replacements** for simulation-first - they are **complements** to it.

**The Testing Hierarchy:**
1. **Simulation-first (mandatory)** - Full system, fault injection, deterministic
2. **Integration tests (encouraged)** - Real workflows, real services
3. **Unit tests (encouraged)** - Fast feedback, isolated components
4. **Property-based tests (encouraged)** - Edge case discovery
5. **E2E tests (encouraged)** - Complete user journeys

---

### 3. TigerStyle Safety (MANDATORY)

Following TigerBeetle's principle: **Safety > Performance > Developer Experience**

**Explicit Constants with Units:**
```rust
// Good - unit in name, explicit limit
pub const ACTOR_INVOCATION_TIMEOUT_MS_MAX: u64 = 30_000;
pub const ACTOR_STATE_SIZE_BYTES_MAX: usize = 10 * 1024 * 1024;

// Bad - unclear units
pub const TIMEOUT: u64 = 30000;
```

**Assertions (2+ per Function):**
```rust
pub fn set_timeout(&mut self, timeout_ms: u64) {
    assert!(timeout_ms > 0, "timeout must be positive");
    assert!(timeout_ms <= TIMEOUT_MS_MAX, "timeout exceeds maximum");
    self.timeout_ms = timeout_ms;
    assert!(self.timeout_ms > 0);
}
```

**No Silent Truncation:**
```rust
// Good - explicit conversion with assertion
let size: u64 = data.len() as u64;
assert!(size <= u32::MAX as u64, "size too large for u32");
let size_u32: u32 = size as u32;

// Bad - silent truncation
let size: u32 = data.len() as u32;
```

---

### 4. No Placeholders in Production

Code that gets committed should be complete, not placeholder.

- No TODO, FIXME, HACK, XXX left unaddressed
- No commented-out code blocks
- No stub functions that don't actually work
- No silent failures (errors caught but not handled)

**Verification:** Run `/no-cap` before marking work complete.

---

### 5. Explicit Over Implicit

Make the code's behavior clear and obvious.

- Handle errors explicitly, don't swallow them
- Use clear naming that reveals intent
- Document non-obvious decisions
- Avoid magic numbers and strings

---

### 6. Quality Over Speed

It's better to ship less, done well, than more with issues.

- Don't cut corners on error handling
- Don't skip tests to meet deadlines
- Fix root causes, not symptoms

---

### 7. Changes Are Traceable

Every significant change should be traceable to a decision.

- Document why, not just what
- Link commits to plans or issues
- Keep decision log in plan files (`.progress/`)

---

## Development Process Constraints

### Planning Required for Non-Trivial Work

Before starting work that touches multiple files or takes more than a few minutes:
1. Create a plan in `.progress/`
2. Document options considered
3. Note trade-offs of chosen approach

### Verification Before Completion

Before marking work complete:
1. Run `cargo test` - all tests must pass
2. Run `cargo clippy` - fix all warnings
3. Check for placeholders (`/no-cap`)
4. Verify alignment with these constraints
5. Update plan status

---

## Kelpie-Specific Constraints

### Available Fault Types in kelpie-dst

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

### Critical Paths Requiring DST Coverage

- [ ] Actor activation/deactivation
- [ ] State persistence and recovery
- [ ] Cross-actor invocation
- [ ] Failure detection and recovery
- [ ] Migration correctness (when implemented)
- [ ] Memory tier operations (core, working, archival)
- [ ] Tool execution with sandbox isolation
- [ ] MCP server communication

---

## Anti-Patterns (What NOT to Do)

| Anti-Pattern | Why It's Bad |
|--------------|--------------|
| "Works on my machine" | Ship tested, verified code |
| Quick hacks | They become permanent debt |
| Skipping DST tests | You'll regret it later |
| Copy-paste coding | Creates maintenance burden |
| Ignoring errors | Silent failures are the worst |
| .unwrap() in production | Causes panics instead of graceful handling |

---

## Enforcement

These constraints are enforced through:
- Pre-commit hooks (check for placeholders, debug statements)
- CI pipeline (run tests, clippy, format checks)
- `/no-cap` verification command
- Plan review before implementation
- DST test coverage requirements

---

*Note: This document should be STABLE - change it rarely and deliberately.*
