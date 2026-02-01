# GitHub Issue #34: DST Cleanup

**Created:** 2026-01-24
**Status:** IN_PROGRESS
**Issue:** https://github.com/kelpie-project/kelpie/issues/34

## Summary

Clean up garbage code, false claims, and bugs in the DST test framework.

## Options & Decisions

### liveness_dst.rs `#![allow(dead_code)]` directive

**Options:**
1. Remove directive and delete unused methods - cleanest, but may lose spec-alignment
2. Remove directive and add `#[allow(dead_code)]` to specific unused methods with justification
3. Exercise the unused methods in tests

**Decision:** Option 2 - Keep methods that align with TLA+ specs but mark them individually with comments explaining they exist for spec completeness.

**Rationale:** The methods exist to model TLA+ state machines. Deleting them would lose the spec alignment.

### TOCTOU race in fault.rs

**Options:**
1. Use `compare_exchange` loop - standard atomic pattern
2. Use `fetch_update` - cleaner Rust API
3. Add mutex protection - simpler but heavier

**Decision:** Option 1 - `compare_exchange` loop is the idiomatic atomic pattern.

### Silent test failures in teleport_service_dst.rs

**Options:**
1. Add explicit assertions for Err cases
2. Convert to Result-returning tests that surface errors
3. Keep current pattern but add logging

**Decision:** Option 1 - Add assertions. The test should fail loudly if verification fails.

### Fragile error checking in agent_integration_dst.rs

**Options:**
1. Use `matches!` with error variants
2. Add error kind enum and check against it
3. Keep string checking but document why

**Decision:** Option 1 - Use `matches!` macro for type-safe error checking.

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 17:30 | Keep spec-aligned dead code with annotations | TLA+ model fidelity | Minor code bloat |
| 17:31 | compare_exchange for TOCTOU fix | Standard atomic pattern | Slightly more code |
| 17:32 | Assertions over silent failures | Fail loudly | None |
| 17:33 | matches! over string checking | Type safety | None |

## Tasks

### Phase 1: Fix TOCTOU race in fault.rs (HIGH) ✅
- [x] Fix TOCTOU race with compare_exchange loop

### Phase 2: Fix liveness_dst.rs dead code (HIGH) ✅
- [x] Remove `#![allow(dead_code)]` module-level attribute
- [x] Add targeted `#[allow(dead_code)]` with comments for spec-aligned methods
- [x] Annotated: `Failed` variant in WalEntryStatus, `release()`, `renew_lease()`, `release_lease()`, `complete()`, `fail()` methods
- [x] Annotated: `clock` fields in ActivationProtocol, RegistrySystem, WalSystem

### Phase 3: Fix silent test failures (MEDIUM) ✅
- [x] Add assertion in teleport_service_dst.rs for verify_result Err case
- [x] Changed from `if let Ok(...)` to `match` with explicit panic on unexpected Err

### Phase 4: Add fault occurrence verification (MEDIUM) ✅
- [x] Added fault stats verification in bug_hunting_dst.rs test_rapid_state_transitions
- [x] Added fault stats verification in bug_hunting_dst.rs test_stress_many_sandboxes_high_faults

### Phase 5: Fix fragile error checking (LOW) ✅
- [x] Updated agent_integration_dst.rs to use `matches!` macro instead of string checking
- [x] Now checks for `Error::OperationTimedOut` or `Error::Internal` types

### Phase 6: Fix/remove ignored tests (LOW) ✅
- [x] Reviewed stress_test_teleport_operations() - no block_on issue
- [x] Fixed block_on usage in test_dst_teleport_interrupted_midway - converted to proper async/await

### Phase 7: Verification ✅
- [x] Run cargo test -p kelpie-dst - All tests pass
- [x] Run cargo clippy -p kelpie-dst - No warnings
- [x] Commit and push

## What to Try

### Works Now
- All DST tests pass with fault injection
- Clippy reports no warnings
- TOCTOU race in fault injection is fixed
- Unused TLA+ spec methods are properly documented

### Doesn't Work Yet
- N/A - All fixes complete

### Known Limitations
- Some TLA+ spec methods are kept but annotated as dead code for spec completeness

## Implementation Notes

### TOCTOU Fix
Changed from check-then-act pattern to compare_exchange loop:
```rust
// Before (TOCTOU race):
let trigger_count = fault_state.trigger_count.load(...);
if trigger_count >= max { continue; }
fault_state.trigger_count.fetch_add(1, ...);

// After (atomic):
loop {
    let current = fault_state.trigger_count.load(...);
    if current >= max { break; }
    match fault_state.trigger_count.compare_exchange(current, current + 1, ...) {
        Ok(_) => return Some(fault_type.clone()),
        Err(_) => continue,
    }
}
```

### Dead Code Annotations
Methods kept for TLA+ spec alignment:
- `ActivationProtocol::release()` - TLA+ Release action
- `LeaseSystem::renew_lease()` - TLA+ RenewLease action
- `LeaseSystem::release_lease()` - TLA+ ReleaseLease action
- `WalSystem::complete()` - TLA+ CompleteEntry action
- `WalSystem::fail()` - TLA+ FailEntry action
- `WalEntryStatus::Failed` - TLA+ spec state

### Silent Failure Fix
Changed from silently ignoring errors to explicit handling:
```rust
// Before:
if let Ok(output) = verify_result { ... }

// After:
match verify_result {
    Ok(output) => { assert!(...); }
    Err(e) => { panic!("Unexpected failure: {}", e); }
}
```

### Type-Safe Error Checking
Changed from string matching to type matching:
```rust
// Before:
assert!(err_str.contains("timed out") || err_str.contains("Internal"))

// After:
assert!(matches!(e, Error::OperationTimedOut { .. } | Error::Internal { .. }))
```
