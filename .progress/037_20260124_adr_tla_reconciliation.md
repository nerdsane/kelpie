# ADR and TLA+ Reconciliation Report

**Created:** 2026-01-24
**Status:** ANALYSIS COMPLETE
**Goal:** Map all discovered issues to ADRs and TLA+ specs, identify gaps, prioritize fixes.

---

## Executive Summary

**27 issues found** during Phase 1 codebase investigation. Analysis reveals:

| Category | Count | Status |
|----------|-------|--------|
| Issues with ADR coverage | 18 | ADRs exist but may need updates |
| Issues without ADR coverage | 9 | Need new ADRs or ADR updates |
| Issues with TLA+ coverage | 8 | Have formal specs |
| Issues needing TLA+ specs | 12 | Should have formal specs |
| Issues not needing TLA+ | 7 | Implementation-only issues |

---

## Part 1: ADRs Needing Updates

### ADR-001: Virtual Actor Model

**Status:** Has implementation notes, but incomplete

**Current Claims vs Reality:**

| Claim | ADR Status | Reality | Action |
|-------|------------|---------|--------|
| Single activation guarantee | ⚠️ Partial | Local check only, TOCTOU race | ❌ ADR correctly notes this |
| Distributed single activation | ❌ Not Implemented | Correct | ✅ ADR accurate |
| Failure recovery | ❌ Not Implemented | Correct | ✅ ADR accurate |

**Related Issues:**
- [HIGH] Zombie actor risk (no heartbeat-lease coordination)
- [MEDIUM] Distributed mode TOCTOU race
- [MEDIUM] Stale registry entries on node crash

**ADR Update Needed:** None - ADR already updated in progress/035

---

### ADR-002: FoundationDB Integration

**Status:** Has implementation notes, needs update

**Current Claims vs Reality:**

| Claim | ADR Status | Reality | Action |
|-------|------------|---------|--------|
| FDB Backend storage | ✅ Complete | Works | ✅ Accurate |
| FDB Registry | ⚠️ Has Issues | TOCTOU race in lease check | ✅ ADR notes this |
| Transaction semantics | ✅ Complete | Auto-retry works | ✅ Accurate |
| FDB CI tests | ❌ Ignored | Tests require cluster | ⚠️ Need to note workaround |

**Related Issues:**
- [MEDIUM] try_claim_actor may be incomplete
- [LOW] FDB batch size limit implicit

**ADR Update Needed:** Add note about FDB test strategy

---

### ADR-004: Linearizability Guarantees

**Status:** Has implementation notes, but makes aspirational claims

**Current Claims vs Reality:**

| Claim | ADR Status | Reality | Action |
|-------|------------|---------|--------|
| Single activation (local) | ⚠️ Partial | Has TOCTOU race | ✅ ADR notes this |
| Lease-based ownership | 📋 Design Only | Code exists but has TOCTOU | ✅ ADR notes this |
| Failure detection | 🚧 Partial | Heartbeats exist, no recovery | ✅ ADR notes this |
| Automatic recovery | ❌ Not Implemented | Correct | ✅ Accurate |

**Related Issues:**
- [HIGH] Zombie actor risk
- [MEDIUM] Distributed mode TOCTOU race
- [MEDIUM] Stale registry entries

**ADR Update Needed:** None - ADR already has accurate implementation notes

---

### ADR-005: DST Framework

**Status:** Has implementation notes, needs update

**Current Claims vs Reality:**

| Claim | ADR Status | Reality | Action |
|-------|------------|---------|--------|
| SimClock, SimRng, etc. | ✅ Complete | Works | ✅ Accurate |
| 16+ fault types | ✅ Complete | Actually 40+ | ⚠️ Update count |
| Stateright integration | 🚧 Scaffolded | Pseudocode only | ✅ ADR notes this |
| 7 invariants verified | ⚠️ Partial | Tests run but weak assertions | ⚠️ Need update |

**Related Issues:**
- [HIGH] No invariant verification helpers → **NOW FIXED** (Phase 4)
- [HIGH] Stateright not integrated
- [MEDIUM] Missing fault types
- [MEDIUM] ClockSkew/ClockJump faults not injected

**ADR Update Needed:**
1. Update fault type count (40+)
2. Add reference to new invariant verification helpers (tests/common/invariants.rs)
3. Note TLA+ bug pattern tests added

---

### ADR-010: Heartbeat/Pause Mechanism

**Status:** No implementation status table

**Missing Information:**
- No implementation status table
- No notes on what's actually implemented

**ADR Update Needed:** Add implementation status table

---

## Part 2: Issues Without ADR Coverage (Need New ADRs)

### Issue: WAL has no replay mechanism

**Severity:** HIGH
**Component:** kelpie-storage
**Evidence:** wal.rs pending_entries() exists but no code calls it on startup

**ADR Gap:** No ADR describes WAL design or recovery procedure

**Action:** Create ADR-021: Write-Ahead Log Design
- Should document WAL purpose
- Should document recovery procedure
- Should document what happens on crash

---

### Issue: join_cluster() is stub

**Severity:** HIGH
**Component:** kelpie-cluster
**Evidence:** cluster.rs:423-435 iterates seeds but takes no action

**ADR Gap:** No ADR describes cluster join protocol

**Action:** Could be covered by future cluster ADR when implementation happens
**Note:** ADR-001 already notes "Cluster distribution: ❌ Not Implemented"

---

### Issue: Failure detection never executes migrations

**Severity:** HIGH
**Component:** kelpie-cluster
**Evidence:** cluster.rs:566 TODO(Phase 6)

**ADR Gap:** No ADR describes migration protocol

**Action:** Future work - migration is not implemented

---

### Issue: Firecracker state TOCTOU

**Severity:** HIGH
**Component:** kelpie-sandbox
**Evidence:** firecracker.rs:482-489 - state read then released then written

**ADR Gap:** ADR-017 (superseded) and ADR-020 don't mention this

**Action:** Add note to ADR-020 about Firecracker state management risks

---

### Issue: kelpie-memory not thread-safe

**Severity:** HIGH
**Component:** kelpie-memory
**Evidence:** CoreMemory/WorkingMemory are Clone but not Arc<Mutex>

**ADR Gap:** ADR-009 (Memory Tools Architecture) doesn't mention thread safety

**Action:** Update ADR-009 with thread safety requirements/limitations

---

### Issue: Memory transaction not atomic

**Severity:** MEDIUM
**Component:** kelpie-storage
**Evidence:** memory.rs:90-196 commit applies writes sequentially

**ADR Gap:** No ADR documents MemoryBackend limitations vs FDB

**Action:** Add note to ADR-002 about MemoryBackend being for testing only

---

### Issue: Checkpoint not atomic with state mutations

**Severity:** MEDIUM
**Component:** kelpie-memory
**Evidence:** No WAL visible in checkpoint.rs

**ADR Gap:** No ADR for checkpoint design

**Action:** Create ADR-022: Checkpoint and State Persistence
- Document checkpoint atomicity requirements
- Document relationship with WAL

---

## Part 3: TLA+ Coverage Analysis

### Existing TLA+ Specs (4)

| Spec | Invariants | Bug Patterns | Issues Covered |
|------|------------|--------------|----------------|
| KelpieSingleActivation.tla | SingleActivation, PlacementConsistency, LeaseValidityIfActive | TryClaimActor_Racy, LeaseExpires_Racy | Zombie actor, TOCTOU race |
| KelpieRegistry.tla | CapacityBounds, CapacityConsistency, HeartbeatStatusSync, LeaseExclusivity | RegisterActor_Racy, ReceiveHeartbeat_Racy | Concurrent registration |
| KelpieActorState.tla | StateConsistency, TransactionAtomicity, RollbackCorrectness | CommitTransaction_StateOnly | Partial commit |
| KelpieWAL.tla (**NEW**) | WAL_Durability, WAL_Monotonicity, WAL_IdempotencyGuarantee, WAL_RecoveryCompleteness, WAL_StatusConsistency, WAL_NoZombieComplete | Append_DuplicateIdempotency, Append_ReusedId, Complete_Premature, CompleteRecovery_Partial | WAL no replay, WAL durability |

### Issues Covered by TLA+ (10)

| Issue | TLA+ Spec | Bug Pattern | DST Test Status |
|-------|-----------|-------------|-----------------|
| [HIGH] Zombie actor risk | KelpieSingleActivation | LeaseExpires_Racy | ✅ test_zombie_actor_reclaim_race |
| [HIGH] WAL no replay | KelpieWAL | CompleteRecovery_Partial | ⚠️ Needs DST test |
| [MEDIUM] Distributed TOCTOU race | KelpieSingleActivation | TryClaimActor_Racy | ✅ test_toctou_race_dual_activation |
| [MEDIUM] try_claim_actor incomplete | KelpieSingleActivation | TryClaimActor_Racy | ✅ test_toctou_race_dual_activation |
| [MEDIUM] Memory transaction not atomic | KelpieActorState | CommitTransaction_StateOnly | ✅ test_partial_commit_detected |
| [MEDIUM] WAL idempotency bypass | KelpieWAL | Append_DuplicateIdempotency | ⚠️ Needs DST test |
| [LOW] Sequential lock acquisition | KelpieRegistry | RegisterActor_Racy | ✅ test_concurrent_registration_race |
| [MEDIUM] Stale registry entries | KelpieRegistry | HeartbeatStatusSync | ⚠️ Needs test |
| [MEDIUM] Shutdown race | KelpieActorState | - | ⚠️ Needs test |
| [LOW] BUG-001/BUG-002 patterns | All specs | Various | ⚠️ Needs tests |

### Issues Needing TLA+ Specs (11 remaining)

| Issue | Priority | Proposed Spec | Invariants Needed |
|-------|----------|---------------|-------------------|
| ~~[HIGH] WAL no replay~~ | ~~HIGH~~ | ~~KelpieWAL.tla~~ | ✅ **COMPLETE** (2026-01-24) |
| [HIGH] Firecracker state TOCTOU | MEDIUM | KelpieSandbox.tla | SandboxStateConsistency |
| [HIGH] kelpie-memory not thread-safe | MEDIUM | - (impl fix, not TLA+) | N/A |
| [HIGH] Stateright not integrated | LOW | - (tooling, not TLA+) | N/A |
| [HIGH] join_cluster() stub | LOW | KelpieCluster.tla (aspirational) | ClusterMembership |
| [HIGH] Failure detection no migration | LOW | KelpieCluster.tla (aspirational) | MigrationAtomicity |
| [MEDIUM] Async I/O without atomicity | MEDIUM | KelpieSandbox.tla | ConfigurationAtomicity |
| [MEDIUM] Checkpoint not atomic | HIGH | KelpieCheckpoint.tla | CheckpointAtomicity |
| [MEDIUM] ClockSkew/Jump not injected | LOW | - (tooling, not TLA+) | N/A |
| [MEDIUM] Missing fault types | LOW | - (tooling, not TLA+) | N/A |
| [MEDIUM] Expired entries capacity | LOW | - (impl fix, not TLA+) | N/A |
| [MEDIUM] TcpTransport incomplete | LOW | KelpieCluster.tla | N/A |

### Issues NOT Needing TLA+ (7)

These are implementation quality issues, not correctness issues:

| Issue | Reason |
|-------|--------|
| [MEDIUM] No consensus algorithm | Design choice, uses FDB |
| [LOW] FDB batch size implicit | Validation issue |
| [LOW] StorageBackend validation | Runtime vs compile-time check |
| [LOW] Process cleanup race | Error handling issue |
| [LOW] Snapshot checksum weak | Security improvement |
| [LOW] No auto-restart dispatcher | Operational issue |
| [MEDIUM] LeaseRenewalTask silent failures | Logging issue |

---

## Part 4: Prioritized Action Items

### Priority 1: TLA+ Specs to Create (HIGH VALUE)

| Spec | Why | Invariants | Estimated Effort |
|------|-----|------------|------------------|
| KelpieWAL.tla | WAL has no replay mechanism | WALDurability, WALMonotonicity, RecoveryCompleteness | Medium |
| KelpieCheckpoint.tla | Checkpoint not atomic | CheckpointAtomicity, StateConsistency | Medium |

### Priority 2: ADRs to Update

| ADR | Update Needed | Effort |
|-----|---------------|--------|
| ADR-005 | Add invariant verification helpers reference, update fault count | Low |
| ADR-009 | Add thread safety requirements | Low |
| ADR-002 | Add note about MemoryBackend limitations | Low |
| ADR-010 | Add implementation status table | Low |

### Priority 3: ADRs to Create

| ADR | Topic | Why |
|-----|-------|-----|
| ADR-021 | Write-Ahead Log Design | Document WAL and recovery |
| ADR-022 | Checkpoint and State Persistence | Document checkpoint design |

### Priority 4: DST Tests to Add

| Test | TLA+ Pattern | Target |
|------|--------------|--------|
| test_heartbeat_status_sync | HeartbeatStatusSync | registry consistency |
| test_shutdown_race_atomicity | StateConsistency | server shutdown |

### Priority 5: Implementation Fixes (After TLA+ Verification)

| Issue | Fix Location | Complexity |
|-------|--------------|------------|
| WAL replay on startup | kelpie-storage/src/wal.rs | High |
| Memory thread safety | kelpie-memory/src/core.rs | Medium |
| Firecracker state atomicity | kelpie-sandbox/src/firecracker.rs | Medium |
| Checkpoint atomicity | kelpie-memory/src/checkpoint.rs | High |

---

## Part 5: Complete Issue → ADR → TLA+ Mapping

### HIGH Severity Issues (8)

| # | Issue | Component | ADR | TLA+ | Action |
|---|-------|-----------|-----|------|--------|
| 1 | Zombie actor risk | kelpie-registry | ADR-001, ADR-004 ✅ | KelpieSingleActivation ✅ | DST test exists ✅ |
| 2 | WAL no replay | kelpie-storage | ❌ None | ❌ None | Create ADR-021, KelpieWAL.tla |
| 3 | No invariant helpers | kelpie-dst | ADR-005 | N/A | **FIXED in Phase 4** ✅ |
| 4 | Stateright not integrated | kelpie-dst | ADR-005 ✅ | N/A | Future work |
| 5 | join_cluster() stub | kelpie-cluster | ADR-001 ✅ | ❌ None (aspirational) | Future work |
| 6 | No migration execution | kelpie-cluster | ❌ None | ❌ None (aspirational) | Future work |
| 7 | Firecracker TOCTOU | kelpie-sandbox | ADR-020 ⚠️ | ❌ None | Update ADR, consider TLA+ |
| 8 | Memory not thread-safe | kelpie-memory | ADR-009 ⚠️ | ❌ None | Update ADR, impl fix |

### MEDIUM Severity Issues (12)

| # | Issue | Component | ADR | TLA+ | Action |
|---|-------|-----------|-----|------|--------|
| 1 | Shutdown race | kelpie-server | ADR-004 ⚠️ | KelpieActorState ⚠️ | Add DST test |
| 2 | Distributed TOCTOU | kelpie-runtime | ADR-001, ADR-004 ✅ | KelpieSingleActivation ✅ | DST test exists ✅ |
| 3 | Stale registry entries | kelpie-runtime | ADR-004 ✅ | KelpieRegistry ⚠️ | Add DST test |
| 4 | try_claim_actor incomplete | kelpie-registry | ADR-002 ✅ | KelpieSingleActivation ✅ | DST test exists ✅ |
| 5 | Memory txn not atomic | kelpie-storage | ADR-002 ⚠️ | KelpieActorState ✅ | Note in ADR |
| 6 | Missing fault types | kelpie-dst | ADR-005 ⚠️ | N/A | Update ADR |
| 7 | Clock faults not injected | kelpie-dst | ADR-005 ⚠️ | N/A | Impl work |
| 8 | No consensus | kelpie-cluster | ADR-002 ✅ | N/A | By design |
| 9 | TcpTransport incomplete | kelpie-cluster | ❌ None | ❌ None | Future work |
| 10 | Async I/O atomicity | kelpie-sandbox | ADR-020 ⚠️ | ❌ None | Consider TLA+ |
| 11 | Checkpoint not atomic | kelpie-memory | ❌ None | ❌ None | Create ADR-022, TLA+ |
| 12 | Expired entries capacity | kelpie-memory | ADR-009 ⚠️ | N/A | Impl fix |

### LOW Severity Issues (7)

| # | Issue | Component | ADR | TLA+ | Action |
|---|-------|-----------|-----|------|--------|
| 1 | BUG patterns not DST verified | kelpie-server | ADR-005 ⚠️ | All ⚠️ | Add DST tests |
| 2 | No auto-restart dispatcher | kelpie-runtime | ❌ None | N/A | Consider impl |
| 3 | Sequential lock stale state | kelpie-registry | ADR-002 ✅ | KelpieRegistry ✅ | DST test exists ✅ |
| 4 | FDB batch size implicit | kelpie-storage | ADR-002 ⚠️ | N/A | Add validation |
| 5 | StorageBackend runtime check | kelpie-core | ❌ None | N/A | Consider compile-time |
| 6 | Process cleanup race | kelpie-sandbox | ADR-020 ⚠️ | N/A | Error handling |
| 7 | Snapshot CRC32 weak | kelpie-vm | ❌ None | N/A | Security improvement |

---

## Summary: What to Do Next

### Immediate (This Session)

1. ✅ Phase 4 complete - invariant verification helpers exist
2. Update ADR-005 with reference to new tests

### Short Term (Next Session)

1. Create KelpieWAL.tla spec
2. Create KelpieCheckpoint.tla spec
3. Update ADRs 002, 005, 009, 010

### Medium Term

1. Create ADR-021 (WAL Design)
2. Create ADR-022 (Checkpoint Design)
3. Add remaining DST tests for HeartbeatStatusSync, shutdown race

### Long Term (When Implementing)

1. Fix WAL replay mechanism
2. Fix memory thread safety
3. Fix Firecracker state atomicity
4. Fix checkpoint atomicity

---

## Quick Reference: ADR Status Summary

| ADR | Title | Accuracy | Update Needed |
|-----|-------|----------|---------------|
| 001 | Virtual Actor Model | ✅ Accurate | None |
| 002 | FoundationDB Integration | ✅ Accurate | Add MemoryBackend note |
| 004 | Linearizability Guarantees | ✅ Accurate | None |
| 005 | DST Framework | ⚠️ Stale | Add invariant helpers ref |
| 009 | Memory Tools Architecture | ⚠️ Missing | Add thread safety |
| 010 | Heartbeat/Pause | ⚠️ Missing | Add impl status |
| 020 | Consolidated VM Crate | ⚠️ Missing | Add Firecracker TOCTOU note |

## Quick Reference: TLA+ Status Summary

| Spec | Exists | DST Tests | Gaps |
|------|--------|-----------|------|
| KelpieSingleActivation | ✅ | ✅ 2 tests | None |
| KelpieRegistry | ✅ | ⚠️ 1 test | HeartbeatStatusSync test |
| KelpieActorState | ✅ | ✅ 1 test | Shutdown race test |
| KelpieWAL | ❌ | N/A | **CREATE** |
| KelpieCheckpoint | ❌ | N/A | **CREATE** |
| KelpieCluster | ❌ | N/A | Aspirational only |
