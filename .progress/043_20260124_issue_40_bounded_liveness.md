# Issue #40: DST - Implement Real Bounded Liveness Testing

**Created:** 2026-01-24
**Issue:** https://github.com/rita-aga/kelpie/issues/40
**Branch:** issue-40-bounded-liveness
**Worktree:** ../kelpie-issue-40-bounded-liveness

## Summary

Current liveness tests use polling-based verification (check condition repeatedly). The issue requests "real" bounded model checking that explores state space systematically.

## Current State Analysis

The existing code has:
- `BoundedLiveness` struct with `verify_eventually`, `verify_leads_to`, `verify_infinitely_often`
- 8 liveness tests in `liveness_dst.rs` that test various TLA+ properties
- State machines for activation, registry, lease, and WAL systems

**Gap:** The current approach is "polling" - it just advances time and checks if a condition holds. True bounded model checking would:
1. Explore all possible interleavings of actions
2. Verify the property holds across ALL explored paths
3. Report counterexamples when violations are found

## Options Considered

### Option 1: Enhance Existing Polling Approach (Chosen)
- Add `#[madsim::test]` to get deterministic task scheduling
- Add state-based `check_eventually` method that takes a state transition function
- Add counterexample reporting with trace
- **Pros:** Incremental improvement, leverages existing code
- **Cons:** Still not full model checking

### Option 2: Integrate Stateright Model Checker
- Use stateright crate for formal model checking
- Define state machines as stateright models
- **Pros:** Real model checking with formal guarantees
- **Cons:** Significant refactoring, different paradigm

### Option 3: Custom BFS/DFS State Space Exploration
- Implement explicit state space exploration
- Bound by step count, explore all paths up to bound
- **Pros:** Full control, true bounded model checking
- **Cons:** More implementation work

**Decision:** Option 1 + elements of Option 3. Enhance `BoundedLiveness` with:
1. State-based exploration methods
2. Trace/counterexample support
3. `#[madsim::test]` attribute for determinism

## Implementation Plan

### Phase 1: Add State-Based Exploration to BoundedLiveness
- Add `check_eventually_from_state<S>()` that takes initial state and transition fn
- Add `check_leads_to_from_state<S>()` for P ~> Q verification
- Add trace capture for counterexamples

### Phase 2: Add Counterexample Reporting
- `LivenessViolation` should include execution trace
- Show sequence of states leading to violation
- Document which TLA+ property was violated

### Phase 3: Update Tests to Use madsim::test
- Convert tests to use `#[madsim::test]` for determinism
- Ensure DST_SEED works for reproduction

### Phase 4: Add New Tests from Issue
- Test for SingleActivation bounded liveness
- Test for WAL recovery bounded liveness
- Test for failure detection bounded liveness

## What to Try

### Works Now
- `cargo test -p kelpie-dst liveness` - existing tests should pass

### Doesn't Work Yet
- State-based exploration (implementing now)
- Counterexample traces (implementing now)

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| Start | Enhance existing vs rewrite | Existing infra is solid, just needs state exploration | Less formal than stateright |
| Start | Add trace capture | Counterexamples are essential for debugging | More memory usage |

## Verification Checklist

- [x] `cargo test -p kelpie-dst liveness` passes (18 unit tests + 8 integration tests)
- [x] At least 3 tests verify real temporal properties (11 new StateExplorer tests)
- [x] Counterexamples show trace on failure (StateTrace with actions)
- [x] Tests document which TLA+ property they verify
- [x] `cargo clippy` passes
- [x] `cargo fmt --check` passes

## Implementation Summary

Added `StateExplorer` with true state-space exploration:

1. **`StateExplorer::check_eventually()`** - BFS exploration to find if property ever holds
2. **`StateExplorer::check_leads_to()`** - Verifies P ~> Q (leads-to property)
3. **`StateExplorer::check_infinitely_often()`** - Checks []<> (infinitely often)
4. **`StateTrace<S>`** - Captures counterexample traces with actions
5. **`StateLivenessViolation<S>`** - Error with full trace for debugging

New tests:
- `test_state_explorer_check_eventually_success` - Basic eventually property
- `test_state_explorer_check_eventually_immediate` - Property holds in initial state
- `test_state_explorer_check_eventually_failure` - Unreachable property
- `test_state_explorer_check_leads_to` - Claiming ~> Active ∨ Idle
- `test_state_explorer_check_leads_to_vacuous` - Vacuously true leads-to
- `test_state_explorer_check_infinitely_often` - []<> property
- `test_state_explorer_bounded_depth` - Depth limiting
- `test_state_explorer_two_node_eventual_activation` - Multi-node liveness
- `test_state_explorer_mutual_exclusion` - Safety property (negative test)
