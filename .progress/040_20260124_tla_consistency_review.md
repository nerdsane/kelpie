# TLA+ Consistency Review for DST Alignment

**Created**: 2026-01-24
**Status**: Phase 1 Complete - Buggy configs created, README updated

## Executive Summary

Reviewed all 10 TLA+ specs for consistency and DST alignment. Found:
- **3 specs missing buggy configs** (Registry, SingleActivation, WAL)
- **Inconsistent NULL sentinel naming** (NULL vs NONE vs NoHolder)
- **6 specs missing crash modeling** (critical for DST)
- **Inconsistent BUGGY mode patterns** across specs

## Detailed Analysis

### 1. BUGGY Mode Patterns

| Spec | Has CONSTANT BUGGY | Uses BUGGY | Has _Buggy.cfg | Bug Injection Method |
|------|-------------------|------------|----------------|---------------------|
| KelpieActorLifecycle | No | Yes | Yes | `BUGGY = TRUE` in cfg |
| KelpieActorState | No | Yes | Yes | `SafeMode = FALSE` |
| KelpieClusterMembership | **Yes** | Yes | Yes | `BUGGY` constant |
| KelpieFDBTransaction | No | No | Yes | Config skips conflict detection |
| KelpieLease | No | No | Yes | Race condition mode |
| KelpieMigration | No | Yes | Yes | `SkipTransfer = TRUE` |
| KelpieRegistry | No | No | **No** | N/A |
| KelpieSingleActivation | No | No | **No** | N/A |
| KelpieTeleport | No | No | Yes | Config allows cross-arch |
| KelpieWAL | No | No | **No** | N/A |

**Issue**: Inconsistent bug injection patterns. Only ClusterMembership uses formal CONSTANT BUGGY pattern.

### 2. NULL Sentinel Styles

| Spec | Sentinel Style | Usage |
|------|---------------|-------|
| KelpieRegistry | `NULL` | Placement sentinel |
| KelpieSingleActivation | `NONE` | Holder sentinel |
| KelpieLease | `NoHolder`, `NONE` | Lease holder sentinel |
| Others | (none) | N/A |

**Issue**: Three different styles for the same concept (no value/empty).

**Recommendation**: Standardize on `NONE` (most common TLA+ convention).

### 3. Crash Modeling

| Spec | Models Crashes | DST Fault Types Needed |
|------|---------------|------------------------|
| KelpieWAL | Yes | CrashBeforeWrite, CrashAfterWrite |
| KelpieRegistry | Yes | NodeCrash |
| KelpieMigration | Yes | CrashDuringMigration |
| KelpieClusterMembership | Yes | NodeCrash, PartitionCrash |
| KelpieSingleActivation | **No** | CrashDuringActivation |
| KelpieLease | **No** | CrashDuringLeaseAcquire |
| KelpieTeleport | **No** | CrashDuringSnapshot |
| KelpieFDBTransaction | **No** | CrashDuringTransaction |
| KelpieActorState | **No** | CrashDuringInvocation |
| KelpieActorLifecycle | **No** | CrashDuringActivation |

**Issue**: 6 specs don't model crash scenarios critical for DST alignment.

### 4. Liveness Properties

| Spec | Has Liveness | Liveness Properties |
|------|-------------|---------------------|
| KelpieWAL | Yes | WF_vars, EventualRecovery, EventualCompletion |
| KelpieRegistry | Yes | WF_vars, EventualFailureDetection |
| KelpieSingleActivation | Yes | WF_vars, EventualActivation |
| KelpieLease | Yes | WF_vars, EventualAcquisition |
| KelpieClusterMembership | Yes | WF_vars |
| KelpieActorState | Yes | WF_vars |
| KelpieMigration | Yes | WF_vars |
| KelpieTeleport | Yes | WF_vars |
| KelpieActorLifecycle | Yes | WF_vars |
| KelpieFDBTransaction | Yes | WF_vars |

**Status**: All specs have basic liveness (WF_vars). Some have explicit liveness properties.

### 5. FDB-Related Specs

Specs that model FoundationDB semantics:
- `KelpieFDBTransaction` - OCC conflict detection
- `KelpieSingleActivation` - FDB transaction for activation
- `KelpieLease` - FDB-backed leases

**Gaps**:
- KelpieFDBTransaction doesn't model transaction timeout or crash-during-commit
- KelpieSingleActivation doesn't model FDB unavailability
- KelpieLease doesn't model lease persistence crashes

## Proposed Fixes

### Priority 1: Add Missing Buggy Configs (High)

Create `_Buggy.cfg` files for:
1. **KelpieRegistry_Buggy.cfg** - Disable placement consistency check
2. **KelpieSingleActivation_Buggy.cfg** - Allow dual activation
3. **KelpieWAL_Buggy.cfg** - Skip durability guarantee

### Priority 2: Standardize NULL Sentinels (Medium)

Replace all sentinel values with consistent `NONE`:
- KelpieRegistry: `NULL` → `NONE`
- KelpieLease: `NoHolder` → `NONE`

### Priority 3: Add Crash Modeling (High - DST Alignment)

Add crash actions to specs without them:
- KelpieSingleActivation: `CrashDuringActivation`
- KelpieLease: `CrashDuringAcquire`, `CrashDuringRenew`
- KelpieTeleport: `CrashDuringSnapshot`, `CrashDuringRestore`
- KelpieFDBTransaction: `CrashDuringCommit`
- KelpieActorState: `CrashDuringInvocation`
- KelpieActorLifecycle: `CrashDuringActivation`

### Priority 4: Standardize BUGGY Mode Pattern (Low)

Adopt ClusterMembership's pattern across all specs:
```tla
CONSTANT BUGGY  \* TRUE enables buggy behavior for testing

BuggyAction == IF BUGGY THEN ... ELSE ...
```

## DST Alignment Matrix

| TLA+ Spec | DST Fault Types | Alignment Status |
|-----------|-----------------|------------------|
| KelpieWAL | StorageWriteFail, CrashBeforeWrite, CrashAfterWrite | **Aligned** |
| KelpieRegistry | NetworkPartition, NodeCrash | **Aligned** |
| KelpieMigration | NetworkPartition, CrashDuringMigration | **Aligned** |
| KelpieClusterMembership | NetworkPartition, NodeCrash | **Aligned** |
| KelpieFDBTransaction | TransactionConflict | **Partial** (missing crash) |
| KelpieSingleActivation | TransactionConflict | **Partial** (missing crash) |
| KelpieLease | - | **Needs Work** |
| KelpieTeleport | - | **Needs Work** |
| KelpieActorState | - | **Needs Work** |
| KelpieActorLifecycle | - | **Needs Work** |

## Verification Commands

Run all safe configs (should PASS):
```bash
cd docs/tla
for spec in KelpieLease KelpieActorLifecycle KelpieMigration KelpieActorState \
            KelpieFDBTransaction KelpieTeleport KelpieSingleActivation \
            KelpieRegistry KelpieWAL KelpieClusterMembership; do
    echo "=== $spec ==="
    java -XX:+UseParallelGC -Xmx4g -jar ~/tla2tools.jar -deadlock -config ${spec}.cfg ${spec}.tla
done
```

Run buggy configs (should FAIL):
```bash
cd docs/tla
for spec in KelpieLease KelpieActorLifecycle KelpieMigration KelpieActorState \
            KelpieFDBTransaction KelpieTeleport KelpieClusterMembership; do
    echo "=== ${spec}_Buggy ==="
    java -XX:+UseParallelGC -Xmx4g -jar ~/tla2tools.jar -deadlock -config ${spec}_Buggy.cfg ${spec}.tla
done
```

## Next Steps

1. [x] Create missing _Buggy.cfg files (3) - DONE
2. [ ] Standardize NULL sentinel naming (requires .tla edits)
3. [ ] Add crash modeling to 6 specs (requires .tla edits)
4. [x] Update README.md with consistency notes - DONE
5. [x] Run TLC on safe configs to verify - DONE (Registry, SingleActivation, WAL pass)
6. [ ] Add BUGGY constant to Registry, SingleActivation, WAL specs

## Instance Log

| Time | Instance | Action |
|------|----------|--------|
| 2026-01-24 | claude-1 | Initial consistency analysis |
| 2026-01-24 | claude-1 | Created KelpieRegistry_Buggy.cfg, KelpieSingleActivation_Buggy.cfg, KelpieWAL_Buggy.cfg |
| 2026-01-24 | claude-1 | Updated README.md with consistency notes and DST alignment table |
| 2026-01-24 | claude-1 | Verified TLC passes on Registry, SingleActivation, WAL safe configs |
