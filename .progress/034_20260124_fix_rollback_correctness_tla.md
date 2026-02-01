# Plan: Fix RollbackCorrectness Invariant in TLA+ Spec

**Issue:** GitHub #14
**Status:** Complete
**Created:** 2026-01-24
**Completed:** 2026-01-24

---

## Goal

Implement a real RollbackCorrectness invariant in the KelpieActorState.tla specification that verifies rollback restores pre-invocation state.

## Context

From ADR-008 (Transaction API):
- Transactions buffer writes until commit
- On abort: all buffered writes are discarded
- State must return to pre-invocation state on rollback

The TLA+ specification didn't exist - created from scratch with proper invariants.

## Options Considered

### Option 1: Track Pre-Invocation Snapshot
- Store `stateSnapshot` when invocation starts
- On rollback, verify `memory = stateSnapshot`
- **Pros:** Direct verification of rollback correctness
- **Cons:** Additional state variable

### Option 2: Track Dirty Flag Only
- Track whether buffer has uncommitted changes
- On rollback, verify buffer is cleared
- **Pros:** Simpler
- **Cons:** Doesn't verify memory state restoration

### Decision: Option 1
Tracking the snapshot is essential to verify the core property: rollback restores pre-invocation state.

## Phases

### Phase 1: Create TLA+ Directory and Files
- [x] Create docs/tla directory
- [x] Create KelpieActorState.tla specification
- [x] Create KelpieActorState.cfg configuration
- [x] Create KelpieActorState_Buggy.cfg configuration

### Phase 2: Implement TLA+ Specification
- [x] Model actor state (memory, buffer)
- [x] Model invocation lifecycle (Idle, Running, Committed, Aborted)
- [x] Implement real RollbackCorrectness invariant
- [x] Add liveness property: EventualCommitOrRollback
- [x] Create Buggy variant that violates RollbackCorrectness

### Phase 3: Run TLC Model Checker (MANDATORY)
- [x] Run Safe variant - verify all invariants pass
- [x] Run Buggy variant - verify RollbackCorrectness fails
- [x] Document state count and verification time

### Phase 4: Documentation
- [x] Create docs/tla/README.md
- [x] Document how to run TLC
- [x] Document invariants and their meaning

### Phase 5: Commit and PR
- [x] Commit changes
- [x] Create PR with 'Closes #14'

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 12:40 | Track stateSnapshot | Needed to verify rollback restores original state | Extra state variable |
| 12:40 | Use simple key-value model | Matches Kelpie storage semantics | Limited to single key for TLC efficiency |
| 12:44 | Bound buffer with MaxBufferLen | Unbounded sequences explode state space | Limited depth exploration |
| 12:45 | Buggy = direct memory writes | Need bug that manifests in model | More complex model |

## What to Try

### Works Now
1. Run TLC on Safe variant:
   ```bash
   cd docs/tla
   java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieActorState.cfg KelpieActorState.tla
   ```
   Expected: Model checking completed. No error has been found.

2. Run TLC on Buggy variant:
   ```bash
   java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieActorState_Buggy.cfg KelpieActorState.tla
   ```
   Expected: Error: Invariant RollbackCorrectness is violated.

### Known Limitations
- Model uses single key-value pair for state space efficiency
- MaxBufferLen = 2 limits exploration depth
- Liveness properties require `-liveness` flag

---

## TLC Verification Results

### Safe Variant (KelpieActorState.cfg)
- **Result:** PASS
- **States generated:** 136
- **Distinct states:** 60
- **Depth:** 9
- **Time:** <1 second

### Buggy Variant (KelpieActorState_Buggy.cfg)
- **Result:** FAIL (RollbackCorrectness violated)
- **States to error:** 12
- **Time:** <1 second

**Counterexample:**
```
State 1: Initial
  memory = "empty", snapshot = "empty", state = "Idle"

State 2: StartInvocation
  snapshot = "empty" (captured), state = "Running"

State 3: BufferWrite("v1")
  memory = "v1" (BUG: applied directly), snapshot = "empty"

State 4: Rollback
  memory = "v1" (BUG: not restored), snapshot = "empty"

  => RollbackCorrectness VIOLATED: memory ≠ stateSnapshot
```

---

## Implementation Notes

### RollbackCorrectness Invariant

```tla
RollbackCorrectness ==
    invocationState = "Aborted" =>
        /\ memory = stateSnapshot
        /\ buffer = <<>>
```

### Buggy Behavior

The buggy variant has TWO bugs:
1. `BufferWriteBuggy`: Writes directly to memory instead of buffering
2. `RollbackBuggy`: Does NOT restore memory to snapshot

This ensures the invariant catches the bug.

### Liveness Property

```tla
EventualCommitOrRollback ==
    [](invocationState = "Running" => <>(invocationState \in {"Committed", "Aborted"}))
```

## Files Created

- `docs/tla/KelpieActorState.tla` - Main TLA+ specification
- `docs/tla/KelpieActorState.cfg` - Safe mode configuration
- `docs/tla/KelpieActorState_Buggy.cfg` - Buggy mode configuration
- `docs/tla/README.md` - Documentation
