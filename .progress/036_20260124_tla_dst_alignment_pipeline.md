# Plan: TLA+ to DST Alignment Pipeline

**Created:** 2026-01-24
**Status:** IN_PROGRESS
**Goal:** Build comprehensive knowledge graph of implementation vs ADRs vs TLA+ specs, then use TLA+ to drive TigerStyle DST that verifies and fixes implementations.

---

## Vision Alignment

**Vision files read:** CONSTRAINTS.md, progress/035

**Relevant constraints:**
- Simulation-first development (CONSTRAINTS.md §1)
- TigerStyle safety (CONSTRAINTS.md §3)
- No placeholders in production (CONSTRAINTS.md §4)
- Changes are traceable (CONSTRAINTS.md §7)

**Prerequisite status:** Option B cleanup complete (progress/035). Honest baseline established.

---

## Phase Overview

```
Phase 1: Deep Investigation (Codebase Map + RLM)
    → Build knowledge graph of what's ACTUALLY implemented
    → Map every module, function, invariant assumption

Phase 2: ADR Reconciliation
    → Compare implementation graph to ADRs
    → Find: missing ADRs, inaccurate ADRs, outdated ADRs

Phase 3: TLA+ Gap Analysis
    → Compare implementation graph to TLA+ specs
    → Find: what needs TLA+ specs, what invariants are implicit
    → Generate missing TLA+ specs

Phase 4: DST-TLA+ Alignment (TigerStyle)
    → For each TLA+ invariant: create DST verification
    → For each TLA+ bug pattern: create DST test case

Phase 5: Implementation Fixes
    → Run DST, find violations
    → Fix implementations to satisfy TLA+ invariants
```

---

## Quick Decision Log [REQUIRED]

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 2026-01-24 | Use exam_start with RLM for knowledge graph | EVI tools are designed for this | Learning curve |
| 2026-01-24 | Examined all 14 crates in scope | Complete coverage | Time investment |
| 2026-01-24 | Parallel sub_llm for deep analysis | Efficient token usage | 30s timeout per call |

---

## Phase 1: Deep Investigation

### 1.1 Codebase Map Skill Execution

**Status:** ✅ COMPLETE

**Approach:**
1. Used exam_start with scope=["all"] for full crate examination
2. Ran RLM parallel analysis for invariants, state machines, TOCTOU risks
3. Generated knowledge graph in `.kelpie-index/understanding/20260124_152350_*/`

**Output:**
- `.kelpie-index/understanding/20260124_152350_build-comprehensive-knowledge-graph-for-tla-to-dst/MAP.md`
- `.kelpie-index/understanding/20260124_152350_build-comprehensive-knowledge-graph-for-tla-to-dst/ISSUES.md`
- 14 component files in `components/` subdirectory

### 1.2 RLM Deep Analysis

**Status:** ✅ COMPLETE

**Key Findings:**

| Component | State Machines | Invariants Found | TOCTOU Risks | Issues |
|-----------|---------------|------------------|--------------|--------|
| kelpie-server | AppState lifecycle, Request lifecycle | 7 invariants | 3 | 2 |
| kelpie-runtime | ActivationState (4 states) | Single activation | 1 (distributed) | 3 |
| kelpie-registry | Node status, Actor placement | Lease validity | Zombie actor risk | 3 |
| kelpie-storage | Transaction state | WAL monotonicity | Memory txn not atomic | 3 |
| kelpie-dst | Simulation harness | 41 fault types | None | 4 |
| kelpie-cluster | Cluster state (4 states) | None | None | 4 |
| kelpie-core | None | Compile-time constants | None | 1 |
| kelpie-sandbox | Sandbox state (5 states) | State pre-conditions | Firecracker TOCTOU | 3 |
| kelpie-memory | Core/Working tiers | Capacity bounds | Thread safety | 3 |
| kelpie-vm | VM state (5 states) | Checksum verification | None | 1 |

### 1.3 Knowledge Graph Generation

**Status:** ✅ COMPLETE

**Summary Statistics:**
- **14 components examined**
- **27 issues found** (8 HIGH, 12 MEDIUM, 7 LOW)
- **3 existing TLA+ specs** (SingleActivation, Registry, ActorState)

**Critical HIGH Issues:**
1. kelpie-registry: Zombie actor risk - no heartbeat-lease coordination
2. kelpie-storage: WAL has no replay mechanism
3. kelpie-dst: No invariant verification helpers (weak assertions)
4. kelpie-dst: Stateright not integrated
5. kelpie-cluster: join_cluster() is stub
6. kelpie-cluster: Failure detection never executes migrations
7. kelpie-sandbox: State TOCTOU in Firecracker
8. kelpie-memory: No thread safety

---

## Phase 2: ADR Reconciliation

### 2.1 ADR Audit

**Status:** IN_PROGRESS

**ADRs to audit:** 001, 002, 004, 005 (already updated in progress/035)
**Additional ADRs:** 003-020 (need verification)

### 2.2 Gap Identification

**Status:** PENDING

---

## Phase 3: TLA+ Gap Analysis

### 3.1 Existing TLA+ Specs

**Status:** ✅ ANALYZED

**Existing specs:**
1. **KelpieSingleActivation.tla** - Single activation guarantee
   - Invariants: SingleActivation, PlacementConsistency, LeaseValidityIfActive
   - Bug patterns: TryClaimActor_Racy (TOCTOU), LeaseExpires_Racy (zombie)

2. **KelpieRegistry.tla** - Registry operations
   - Invariants: CapacityBounds, CapacityConsistency, HeartbeatStatusSync, LeaseExclusivity
   - Bug patterns: RegisterActor_Racy, ReceiveHeartbeat_Racy

3. **KelpieActorState.tla** - Transaction atomicity
   - Invariants: StateConsistency, TransactionAtomicity, RollbackCorrectness
   - Bug patterns: CommitTransaction_StateOnly (partial commit)

### 3.2 TLA+ Coverage Gap

**Covered by TLA+:**
- Single activation guarantee
- Lease-based placement
- Transaction atomicity
- Heartbeat-based failure detection

**NOT covered by TLA+ (gaps):**
- WAL replay mechanism
- Memory tier operations
- Sandbox state machine
- VM snapshot/restore
- Cluster join/leave protocol (aspirational)

### 3.3 TLA+ Spec Generation

**Status:** IN_PROGRESS

**Specs created:**
- ✅ KelpieWAL.tla - WAL durability and replay (2026-01-24)
  - TLC verified: 53,121 states explored, no errors (SpecSafe)
  - TLC verified: WAL_IdempotencyGuarantee violation detected (SpecBuggy)

**Specs remaining:**
- KelpieCluster.tla - Cluster membership (aspirational)

**KelpieWAL.tla Invariants:**
| Invariant | Purpose |
|-----------|---------|
| WAL_Durability | Every acknowledged operation has a WAL entry |
| WAL_Monotonicity | Entry IDs are strictly increasing |
| WAL_IdempotencyGuarantee | Same idempotency key returns same entry |
| WAL_RecoveryCompleteness | All pending entries replayed on recovery |
| WAL_StatusConsistency | Entry status matches execution state |
| WAL_NoZombieComplete | Entry not marked complete while still executing |

**KelpieWAL.tla Bug Patterns:**
| Bug Pattern | Action | Violates |
|-------------|--------|----------|
| Append_DuplicateIdempotency | Create duplicate entry for same key | WAL_IdempotencyGuarantee |
| Append_ReusedId | Reuse entry ID after crash | WAL_Monotonicity |
| Complete_Premature | Mark complete before execution finishes | WAL_NoZombieComplete |
| CompleteRecovery_Partial | Skip pending entries during recovery | WAL_RecoveryCompleteness |

---

## Phase 4: DST-TLA+ Alignment

### 4.1 Invariant Verification Helpers

**Status:** ✅ COMPLETE

**Deliverable:** `crates/kelpie-server/tests/common/invariants.rs`

**Implemented verification functions:**
- `verify_single_activation()` - SingleActivation invariant
- `verify_placement_consistency()` - PlacementConsistency invariant
- `verify_lease_validity()` - LeaseValidityIfActive invariant
- `verify_capacity_bounds()` - CapacityBounds invariant
- `verify_capacity_consistency()` - CapacityConsistency invariant
- `verify_lease_exclusivity()` - LeaseExclusivity invariant
- `verify_core_invariants()` / `verify_all_invariants()` - Composite checks
- `InvariantViolation` enum with detailed error types

### 4.2 Bug Pattern Tests

**Status:** ✅ COMPLETE

**Deliverables:**
- `crates/kelpie-server/tests/common/tla_scenarios.rs` - Scenario implementations
- `crates/kelpie-server/tests/tla_bug_patterns_dst.rs` - Test harness

**Bug patterns tested:**
| TLA+ Pattern | Test | Result |
|--------------|------|--------|
| TryClaimActor_Racy | test_toctou_race_dual_activation | ✅ Detects SingleActivation violation |
| LeaseExpires_Racy | test_zombie_actor_reclaim_race | ✅ Detects SingleActivation violation |
| RegisterActor_Racy | test_concurrent_registration_race | ✅ No violation with atomic claims |
| CommitTransaction_StateOnly | test_partial_commit_detected | ✅ Detects PartialCommit violation |
| TryClaimActor_Atomic | test_safe_concurrent_claim | ✅ No violations (safe behavior) |

**Test verification:** `cargo test -p kelpie-server --test tla_bug_patterns_dst` - 18 tests pass

---

## Phase 5: Implementation Fixes

**Status:** PENDING

---

## Checkpoints

- [x] Plan created
- [x] Phase 1.1: Codebase map complete (14 components)
- [x] Phase 1.2: RLM analysis complete (27 issues found)
- [x] Phase 1.3: Knowledge graph generated (MAP.md, ISSUES.md)
- [x] Phase 3.1: TLA+ specs analyzed (3 existing)
- [x] Phase 4.1: Invariant verification helpers complete
- [x] Phase 4.2: Bug pattern tests complete (18 tests, all pass)
- [x] Phase 2: ADR reconciliation complete (`.progress/037_*`)
- [x] Phase 3.3: KelpieWAL.tla created
- [ ] Phase 3.3: KelpieCluster.tla (aspirational, lower priority)
- [ ] Phase 5: Implementation fixes complete

---

## What to Try [REQUIRED]

### Works Now ✅

| What | How to Try | Expected Result |
|------|------------|-----------------|
| Knowledge graph | Read `.kelpie-index/understanding/20260124_152350_*/MAP.md` | Full codebase map |
| Issue list | Read `ISSUES.md` | 27 issues by severity |
| TLA+ specs | Read `docs/tla/*.tla` | 4 specs with SpecSafe/SpecBuggy |
| KelpieWAL.tla | Read `docs/tla/KelpieWAL.tla` | WAL durability/recovery model |
| Invariant verification | `cargo test -p kelpie-server common::invariants` | 6 tests pass |
| TLA+ bug pattern tests | `cargo test -p kelpie-server --test tla_bug_patterns_dst` | 18 tests pass |
| TOCTOU detection | Run `test_toctou_race_dual_activation` | SingleActivation violation detected |
| Zombie detection | Run `test_zombie_actor_reclaim_race` | SingleActivation violation detected |
| Partial commit detection | Run `test_partial_commit_detected` | PartialCommit violation detected |

### Doesn't Work Yet ❌

| What | Why | When Expected |
|------|-----|---------------|
| Real dispatcher integration | Tests use SimulatedRegistry, not real dispatcher | Phase 5 |
| Real FDB testing | Tests use in-memory simulation | Phase 5 |
| Stateright integration | Not implemented | Future |

### Known Limitations ⚠️

- SimulatedRegistry is a simplified model, not full FDB semantics
- Bug pattern tests verify detection, not prevention in production code
- TLA+ specs are verified but not auto-run in CI
- Real TOCTOU prevention requires FDB transactional guarantees

---

## Phase 4: DST-TLA+ Alignment (COMPLETE)

### 4.1 TLA+ Invariants → DST Verification Mapping

| TLA+ Invariant | Verification Function | Status |
|----------------|----------------------|--------|
| SingleActivation | `verify_single_activation()` | ✅ IMPLEMENTED |
| PlacementConsistency | `verify_placement_consistency()` | ✅ IMPLEMENTED |
| LeaseValidityIfActive | `verify_lease_validity()` | ✅ IMPLEMENTED |
| CapacityBounds | `verify_capacity_bounds()` | ✅ IMPLEMENTED |
| CapacityConsistency | `verify_capacity_consistency()` | ✅ IMPLEMENTED |
| LeaseExclusivity | `verify_lease_exclusivity()` | ✅ IMPLEMENTED |
| TransactionAtomicity | `InvariantViolation::PartialCommit` | ✅ IMPLEMENTED |

### 4.2 TLA+ Bug Patterns → DST Tests

| TLA+ Bug Pattern | DST Test | Status |
|------------------|----------|--------|
| TryClaimActor_Racy | `test_toctou_race_dual_activation` | ✅ DETECTS VIOLATION |
| LeaseExpires_Racy | `test_zombie_actor_reclaim_race` | ✅ DETECTS VIOLATION |
| RegisterActor_Racy | `test_concurrent_registration_race` | ✅ SAFE (atomic claims) |
| CommitTransaction_StateOnly | `test_partial_commit_detected` | ✅ DETECTS VIOLATION |
| TryClaimActor_Atomic | `test_safe_concurrent_claim` | ✅ NO VIOLATION |

---

## Completion Notes

### Phase 4 Completion (2026-01-24)

**Summary:** Created comprehensive TLA+ invariant verification infrastructure and bug pattern tests.

**Files created:**
1. `crates/kelpie-server/tests/common/invariants.rs` (~700 lines)
   - 7 invariant verification functions
   - `InvariantViolation` enum with 9 variants
   - `SystemState` struct for snapshot verification
   - Unit tests for each invariant

2. `crates/kelpie-server/tests/common/tla_scenarios.rs` (~500 lines)
   - `SimulatedRegistry` with safe/racy behavior modes
   - `SimulatedNode` for local actor tracking
   - 5 scenario functions mapping to TLA+ bug patterns

3. `crates/kelpie-server/tests/tla_bug_patterns_dst.rs` (~300 lines)
   - 5 individual test functions with detailed assertions
   - Integration test running all patterns
   - 18 total tests, all passing

**Key insights:**
- TLA+ SpecBuggy patterns (TryClaimActor_Racy, LeaseExpires_Racy) correctly produce invariant violations
- TLA+ SpecSafe patterns (TryClaimActor_Atomic) correctly produce NO violations
- The invariant verification infrastructure can be used in real DST tests

**Next steps:**
- Phase 5: Apply verification to real implementation code
- Integrate invariant checks into existing DST tests
- Create CI check for invariant verification
