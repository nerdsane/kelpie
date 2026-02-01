# Plan: Create KelpieFDBTransaction.tla Spec

**Issue:** #9 - Create KelpieFDBTransaction.tla spec
**Created:** 2026-01-24
**Status:** Complete

## Objective

Create a TLA+ specification that models FDB transaction semantics for Kelpie, including conflict detection, serializable isolation, and atomic commit guarantees.

## Context

From the ADRs and FDB implementation:
- ADR-002 (G2.4): Requires transaction conflict detection with automatic retry
- ADR-004 (G4.1): Operations must appear atomic in sequential order
- Current code ASSUMES FDB atomicity - need to MODEL it formally
- FDB provides serializable isolation via optimistic concurrency control

## Options Considered

### Option 1: Model FDB internals fully (resolvers, proxies, storage servers)
- Pros: Most accurate model of real FDB behavior
- Cons: Too complex, not relevant for Kelpie's guarantees
- **REJECTED**: Over-engineering

### Option 2: Abstract transaction model with conflict detection (CHOSEN)
- Pros: Captures the guarantees Kelpie relies on without internal details
- Cons: Abstracts away some FDB implementation details
- **CHOSEN**: Right level of abstraction for verifying Kelpie invariants

### Option 3: Simple atomic operations without conflicts
- Pros: Simplest model
- Cons: Misses the key behavior we want to verify (conflict detection)
- **REJECTED**: Too simple, doesn't verify G2.4

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| Start | Use 2 transactions, 2 keys | Minimal state space to demonstrate conflicts | May miss edge cases with more actors |
| Start | Separate Safe/Buggy .cfg files | Same spec, different configurations | Need two config files |
| Start | Model read set tracking | FDB detects read-write conflicts | Increases state space |
| During | Use NoValue constant | Need sentinel for unset writeBuffer entries | Extra constant in config |
| During | ConflictDetection invariant refinement | Initial version too strict, refined to match FDB semantics | Multiple iterations needed |

## Implementation Plan

### Phase 1: Create TLA+ Spec Structure [COMPLETE]
- [x] Create docs/tla/ directory
- [x] Create KelpieFDBTransaction.tla with core model
- [x] Model states: IDLE, RUNNING, COMMITTED, ABORTED

### Phase 2: Model Transaction Operations [COMPLETE]
- [x] begin(txn) - start transaction, initialize read/write sets
- [x] read(txn, key) - add to read set, return value
- [x] write(txn, key, value) - add to write set, buffer value
- [x] commit(txn) - detect conflicts, commit or abort
- [x] abort(txn) - rollback transaction

### Phase 3: Define Invariants [COMPLETE]
- [x] SerializableIsolation - concurrent txns appear to execute in serial order
- [x] ConflictDetection - conflicting writes cause one txn to abort
- [x] AtomicCommit - commit is all-or-nothing
- [x] ReadYourWrites - txn sees its own uncommitted writes

### Phase 4: Create Buggy Variant [COMPLETE]
- [x] Create variant that skips conflict detection (EnableConflictDetection = FALSE)
- [x] Verify model checker catches the bug (SerializableIsolation violated)

### Phase 5: Run TLC Model Checker [COMPLETE]
- [x] Run Safe configuration - PASSED (0 errors)
- [x] Run Buggy configuration - FAILED (SerializableIsolation violated)
- [x] Document state counts and verification time

### Phase 6: Documentation [COMPLETE]
- [x] Create docs/tla/README.md
- [x] Document how to run the specs
- [ ] Commit and create PR

## What to Try

### Works Now
- **Safe configuration passes**: Run `java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieFDBTransaction.cfg KelpieFDBTransaction.tla`
  - Expected: "Model checking completed. No error has been found."
  - States: 308,867 generated, 56,193 distinct
  - Time: ~14 seconds

- **Buggy configuration fails**: Run `java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieFDBTransaction_Buggy.cfg KelpieFDBTransaction.tla`
  - Expected: "Error: Invariant SerializableIsolation is violated."
  - Counterexample shows read-write conflict not being detected

### Doesn't Work Yet
- N/A - all deliverables complete

### Known Limitations
- Model uses 2 transactions and 2 keys for tractable state space
- Does not model FDB internals (resolvers, storage servers, etc.)
- Does not model network failures (separate spec would be needed)
- Write-write conflicts without reads are allowed (FDB semantics - last writer wins)

## Verification Results

### Safe Configuration (EnableConflictDetection = TRUE)
```
States generated: 308,867
Distinct states: 56,193
Depth: 13
Time: 14 seconds
Result: PASS - All invariants hold
```

### Buggy Configuration (EnableConflictDetection = FALSE)
```
States generated: 6,536 (before finding error)
Distinct states: 2,237
Depth: 7
Time: 1 second
Result: FAIL - SerializableIsolation violated

Counterexample:
1. Txn1 reads k1 (sees v0 from snapshot)
2. Txn2 writes k1 = v1
3. Txn2 commits (kvStore[k1] = v1)
4. Txn1 commits WITHOUT detecting conflict (BUG!)
5. Txn1 read stale value v0 when committed value was v1
```

## Deliverables

1. [x] `docs/tla/KelpieFDBTransaction.tla` - Main TLA+ specification
2. [x] `docs/tla/KelpieFDBTransaction.cfg` - Safe configuration
3. [x] `docs/tla/KelpieFDBTransaction_Buggy.cfg` - Buggy configuration
4. [x] `docs/tla/README.md` - Documentation with run instructions
5. [ ] PR to master with 'Closes #9'

## Verification Checklist
- [x] TLC passes on Safe configuration (0 errors)
- [x] TLC fails on Buggy configuration (finds counterexample)
- [x] State count documented (56,193 distinct states)
- [ ] PR created with 'Closes #9'
