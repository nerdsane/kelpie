# Plan: TLA+ Invariant Verification Framework

**Status:** Complete
**Issue:** #17
**Created:** 2026-01-24
**Branch:** dst/invariant-framework

## Summary

Build a TLA+ invariant verification framework in `crates/kelpie-dst/src/invariants.rs` that allows DST tests to verify TLA+ invariants hold after each simulation step.

## Options & Decisions

### 1. Invariant Trait Design

**Options:**
- A) Single `check()` method returning `Result<(), InvariantViolation>`
- B) Separate methods for `name()`, `description()`, and `check()`
- C) Builder pattern with chainable methods

**Decision:** Option B - Separate methods are clearer for debugging and logging. The `name()` method enables identification without running `check()`.

**Trade-off:** Slightly more boilerplate per invariant, but better error messages and debuggability.

### 2. SystemState Abstraction

**Options:**
- A) Concrete struct with all fields for nodes, actors, placements
- B) Trait-based with multiple implementations for different test scenarios
- C) Generic struct with type parameters for different state types

**Decision:** Option A - Start with concrete struct. The invariants are specific to Kelpie's domain model, so a concrete type is clearer.

**Trade-off:** Less flexible for testing non-Kelpie systems, but simpler implementation.

### 3. Integration with Simulation

**Options:**
- A) `Simulation::run_checked()` - automatic checking after each await point
- B) Explicit `InvariantChecker::verify()` calls in test code
- C) Both - provide `run_checked()` for convenience, allow manual verification

**Decision:** Option C - Both. `run_checked()` for simple cases, manual for complex scenarios where timing matters.

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 2026-01-24 | Use `thiserror` for InvariantViolation | Consistent with kelpie-core error handling | None |
| 2026-01-24 | Implement 6 core invariants first | Match issue requirements | Can add more later |
| 2026-01-24 | Put invariants in submodules | Keep main module clean | More files to navigate |

## Phases

### Phase 1: Core Framework ✅
- [x] Create `invariants.rs` module with `Invariant` trait
- [x] Create `InvariantViolation` error type
- [x] Create `InvariantChecker` struct
- [x] Create `SystemState` abstraction

### Phase 2: Implement Invariants ✅
- [x] `SingleActivation` (from KelpieSingleActivation.tla)
- [x] `ConsistentHolder` (from KelpieSingleActivation.tla)
- [x] `PlacementConsistency` (from KelpieRegistry.tla)
- [x] `LeaseUniqueness` (from KelpieLease.tla)
- [x] `Durability` (from KelpieWAL.tla)
- [x] `AtomicVisibility` (from KelpieWAL.tla)

### Phase 3: Integration ✅
- [x] Add `with_invariants()` to `Simulation`
- [x] Add `run_checked()` to `Simulation`
- [x] Update `lib.rs` to export invariants module

### Phase 4: Testing ✅
- [x] Example test demonstrating invariant checking
- [x] Test that violations are properly detected
- [x] Verify all tests pass

## What to Try

### Works Now
- `cargo test -p kelpie-dst` runs all DST tests including new invariant tests
- `InvariantChecker::new()` creates a checker
- `.with_invariant()` adds invariants to checker
- `.verify_all()` checks all invariants against state
- `.verify_all_collect()` collects all violations
- `SystemState` captures node states, actor placements, leases, WAL entries
- All 6 TLA+ invariants are implemented:
  - `SingleActivation` - at most one active node per actor
  - `ConsistentHolder` - active node matches FDB holder
  - `PlacementConsistency` - no actors on failed nodes
  - `LeaseUniqueness` - at most one lease holder per actor
  - `Durability` - completed WAL entries visible in storage
  - `AtomicVisibility` - WAL entries either fully applied or not

### Doesn't Work Yet
- `Simulation::run_checked()` is stubbed (no automatic checking after await points)
- State capture is manual (must call `SystemState::capture()` explicitly)

### Known Limitations
- `run_checked()` requires manual state capture - automatic interception of async boundaries would require significant runtime changes
- SystemState is a simplified model - real system has more complex state

## Files Created/Modified

- `crates/kelpie-dst/src/invariants.rs` (NEW) - Core framework + all invariants
- `crates/kelpie-dst/src/simulation.rs` (MODIFIED) - Added `with_invariants()`, `run_checked()`
- `crates/kelpie-dst/src/lib.rs` (MODIFIED) - Export invariants module

## Verification

```bash
cargo test -p kelpie-dst
cargo clippy --all-targets --all-features
cargo fmt --check
```

## References

- `docs/tla/KelpieSingleActivation.tla` - SingleActivation, ConsistentHolder
- `docs/tla/KelpieRegistry.tla` - PlacementConsistency
- `docs/tla/KelpieLease.tla` - LeaseUniqueness
- `docs/tla/KelpieWAL.tla` - Durability, AtomicVisibility
