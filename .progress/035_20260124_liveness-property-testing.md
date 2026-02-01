# Plan: DST Liveness Property Testing (Issue #19)

**Created:** 2026-01-24
**Status:** Complete
**Branch:** dst/liveness-testing

## Objective

Add liveness property testing to DST framework, enabling verification of temporal properties like `EventualActivation`, `EventualFailureDetection`, etc. that are defined in TLA+ specs but not currently tested.

## Background

**Safety vs Liveness:**
- **Safety**: "Bad things don't happen" (invariant checks)
- **Liveness**: "Good things eventually happen" (progress checks)

Current DST tests only verify safety. TLA+ specs define liveness properties that need runtime verification.

## Options Considered

### Option 1: Bounded Liveness Checks (CHOSEN)
- Use bounded time/steps to verify eventual properties
- Configurable timeout values
- Works well with deterministic simulation

**Pros:**
- Simple to implement
- Integrates with existing SimClock
- Deterministic behavior

**Cons:**
- Bounded checking (not infinite)
- Must choose appropriate bounds

### Option 2: State Machine Exploration
- Explore all reachable states
- More like TLC model checking

**Pros:**
- Complete state coverage

**Cons:**
- Exponential complexity
- Doesn't match DST paradigm

### Decision: Option 1 - bounded liveness with configurable timeouts

## Implementation Plan

### Phase 1: Core Liveness Module
Create `crates/kelpie-dst/src/liveness.rs`:
- [x] `LivenessViolation` error type
- [x] `BoundedLiveness` struct for timeout-based checks
- [x] `verify_eventually()` - <> operator (eventually)
- [x] `verify_leads_to()` - ~> operator (leads-to)
- [x] `verify_infinitely_often()` - []<> operator (for bounded checking)
- [x] Export from lib.rs

### Phase 2: Liveness Tests
Create `crates/kelpie-dst/tests/liveness_dst.rs`:
- [x] `EventualActivation` - Claims resolve to Active or Idle
- [x] `NoStuckClaims` - No node remains claiming forever
- [x] `EventualFailureDetection` - Dead nodes eventually detected
- [x] `EventualCacheInvalidation` - Stale caches eventually corrected
- [x] `EventualLeaseResolution` - Leases resolve to clean state
- [x] `EventualRecovery` - WAL entries eventually processed

### Phase 3: Fault Injection Integration
- [x] Tests run under fault injection (test_eventual_activation_with_faults, test_eventual_recovery_with_crash_faults)
- [x] Verify liveness holds even with faults
- [x] Document timeout values and their relationship to system timeouts

## Acceptance Criteria (from Issue)

- [x] New module `crates/kelpie-dst/src/liveness.rs`
- [x] `BoundedLiveness` struct for timeout-based checks
- [x] `verify_leads_to()` function for ~> operator
- [x] `verify_eventually()` function for <> operator
- [x] Tests for each TLA+ liveness property (6 total)
- [x] Tests run under fault injection
- [x] Document timeout values (constants at top of liveness_dst.rs)

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 2026-01-24 | Use bounded liveness | Matches DST paradigm, deterministic | Not infinite checking |
| 2026-01-24 | State capture via closure | Flexible, matches existing patterns | Slight indirection |

## What to Try

### Works Now
- `cargo test -p kelpie-dst --test liveness_dst` - Runs 8 liveness tests
- `cargo test -p kelpie-dst --lib liveness` - Runs 7 unit tests for liveness module
- All liveness properties from TLA+ specs are now tested:
  - EventualActivation
  - NoStuckClaims
  - EventualFailureDetection
  - EventualCacheInvalidation
  - EventualLeaseResolution
  - EventualRecovery
- Two fault injection tests verify liveness under adverse conditions

### Doesn't Work Yet
- Stress test is ignored by default (run with `cargo test -p kelpie-dst -- --ignored`)

### Known Limitations
- Bounded checking (not infinite state exploration)
- Must choose appropriate timeout bounds based on system parameters
- State machines in tests are simplified models of TLA+ specs (capture essential behavior)

## Files to Create/Modify

- `crates/kelpie-dst/src/liveness.rs` - NEW
- `crates/kelpie-dst/src/lib.rs` - Add export
- `crates/kelpie-dst/tests/liveness_dst.rs` - NEW

## References

- TLA+ temporal operators: `[]` (always), `<>` (eventually), `~>` (leads-to)
- `docs/tla/KelpieSingleActivation.tla` - EventualActivation, NoStuckClaims
- `docs/tla/KelpieRegistry.tla` - EventualFailureDetection, EventualCacheInvalidation
- `docs/tla/KelpieLease.tla` - EventualLeaseResolution
- `docs/tla/KelpieWAL.tla` - EventualRecovery
