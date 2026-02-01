# Issue #94: Complete KelpieSingleActivation.tla Specification

**Status:** COMPLETED
**Date:** 2026-01-29

## Investigation Summary

### Issue Claims (from #94)
The issue claimed:
1. "Ends at 'NEXT STATE RELATION' comment without defining Next or Spec"
2. "Missing Next disjunction"
3. "Missing Spec formula"
4. "No INVARIANT declarations"
5. "~150 lines"
6. "Create .cfg file" needed
7. "Run TLC" needed

### Actual State (at investigation time)
All original claims were **incorrect**. The specification was already complete:

| Element | Line(s) | Status |
|---------|---------|--------|
| `Next` action | 152-157 | Present |
| `Spec` formula | 223 | Present |
| `SafetySpec` formula | 226 | Present |
| `SingleActivation` invariant | 180-181 | Present |
| `ConsistentHolder` invariant | 185-187 | Present |
| `TypeOK` invariant | 67-71 | Present |
| `Fairness` condition | 170-172 | Present |
| `EventualActivation` liveness | 209-211 | Present |
| `NoStuckClaims` liveness | 214-216 | Present |
| `StateConstraint` | 235 | Present |
| `.cfg` file | KelpieSingleActivation.cfg | Exists |
| `_Buggy.cfg` file | KelpieSingleActivation_Buggy.cfg | Exists |

### What Was Actually Missing

The README.md noted that `KelpieSingleActivation` needed the `BUGGY` constant added to enable the buggy configuration to actually trigger a violation.

## Changes Made

### 1. Added BUGGY constant to KelpieSingleActivation.tla

**Location:** docs/tla/KelpieSingleActivation.tla

Added:
```tla
CONSTANT
    Nodes,
    NONE,
    BUGGY           \* TRUE: skip version check in CommitClaim (violates SingleActivation)

ASSUME BUGGY \in BOOLEAN
```

### 2. Modified CommitClaim action for BUGGY mode

In BUGGY mode, the action:
- Uses stale read-time information
- Skips the OCC version check at commit time
- Allows multiple nodes to commit successfully, causing split-brain

Bug pattern modeled: TOCTOU (Time-Of-Check-Time-Of-Use) race condition

### 3. Updated KelpieSingleActivation.cfg

Added `BUGGY = FALSE` to enable safe mode verification.

### 4. Updated docs/tla/README.md

- Added feature documentation for BUGGY mode
- Moved KelpieSingleActivation from "Needs BUGGY added" to "CONSTANT BUGGY" category
- Updated DST alignment status to "Aligned"

## TLC Verification Results

### Safe Config (BUGGY=FALSE)
```
$ java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieSingleActivation.cfg KelpieSingleActivation.tla

Model checking completed. No error has been found.
1429 states generated, 714 distinct states found, 0 states left on queue.
The depth of the complete state graph search is 27.
```
**Result:** PASS - All invariants hold, all liveness properties satisfied

### Buggy Config (BUGGY=TRUE)
```
$ java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieSingleActivation_Buggy.cfg KelpieSingleActivation.tla

Error: Invariant SingleActivation is violated.
State 7: <CommitClaim(n2)>
/\ node_state = (n1 :> "Active" @@ n2 :> "Active")  <- VIOLATION!
/\ fdb_holder = n2
```
**Result:** FAIL - SingleActivation violated as expected (both n1 and n2 are Active)

### Error Trace (Buggy)
1. `Init`: Both nodes Idle, holder=NONE, version=0
2. `StartClaim(n1)`: n1 enters Reading
3. `ReadFDB(n1)`: n1 reads version=0, enters Committing
4. `StartClaim(n2)`: n2 enters Reading
5. `ReadFDB(n2)`: n2 reads version=0, enters Committing
6. `CommitClaim(n1)`: n1 commits, becomes Active, version=1
7. `CommitClaim(n2)`: n2 commits (BUGGY: ignores version=1), becomes Active, version=2
8. **VIOLATION**: Both n1 and n2 are Active!

## Git History
- `212f00a7` - feat(tla): Add liveness properties to KelpieSingleActivation.tla (#7)
- `e491df0c` - feat(dst): Add SingleActivation invariant DST tests (#16)
- `aa9c746c` - feat(dst): Strengthen ADR->TLA+->DST pipeline (Fixes #35) (#44)

## Files Changed
1. `docs/tla/KelpieSingleActivation.tla` - Added BUGGY constant and conditional CommitClaim
2. `docs/tla/KelpieSingleActivation.cfg` - Added `BUGGY = FALSE`
3. `docs/tla/README.md` - Updated documentation
