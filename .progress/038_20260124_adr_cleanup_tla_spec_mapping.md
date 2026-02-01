# Plan: ADR Cleanup and TLA+ Spec Mapping

**Created**: 2026-01-24
**Status**: Complete

## Summary

Cleaned up ADRs and created comprehensive TLA+ spec mapping:

**Deleted**: 5 superseded ADRs (007-libkrun, 015, 016, 017, 019)
**Renumbered**: ADR-008-snapshot → ADR-021
**Documented**: Complete ADR → TLA+ mapping in docs/tla/README.md
**Updated**: ADR README with verification pipeline concept

**TLA+ Spec Status**:
- 4 existing specs cover 11 guarantees
- 4 specs need fixes (liveness, RollbackCorrectness)
- 6 new specs needed (Lease, Migration, ActorLifecycle, FDBTransaction, Teleport, ClusterMembership)

## Objective

1. Delete superseded/irrelevant ADRs
2. Extract guarantees from remaining ADRs
3. Map guarantees to TLA+ specs (existing + needed)
4. Create comprehensive spec inventory

## Phase 1: Delete Superseded ADRs

### ADRs to Delete

| File | Reason |
|------|--------|
| `007-libkrun-sandbox-integration.md` | Superseded by consolidated VM approach (ADR-020) |
| `015-vminstance-teleport-backends.md` | Superseded by ADR-020 |
| `016-vz-objc-bridge.md` | Superseded by ADR-020 |
| `017-firecracker-backend-wrapper.md` | Explicitly marked "Superseded by ADR-020" |
| `019-vm-backends-crate.md` | Explicitly marked "Superseded by ADR-020" |

### Duplicate ADR Numbers to Resolve

| Conflict | Files | Resolution |
|----------|-------|------------|
| ADR-007 | `007-fdb-backend-implementation.md`, `007-libkrun-sandbox-integration.md` | Delete libkrun, keep FDB |
| ADR-008 | `008-transaction-api.md`, `008-snapshot-type-system.md` | Renumber snapshot to ADR-021 |

## Phase 2: Remaining ADRs and Their Guarantees

### Core Distributed System ADRs

#### ADR-001: Virtual Actor Model
**Guarantees requiring TLA+ verification:**
- G1.1: Single activation (at most one instance per ActorId cluster-wide)
- G1.2: Single-threaded execution (no concurrent invocations per actor)
- G1.3: Lifecycle ordering (activate before invoke, invoke before deactivate)
- G1.4: Location transparency (caller doesn't need to know physical location)
- G1.5: Automatic deactivation after idle timeout

#### ADR-002: FoundationDB Integration
**Guarantees requiring TLA+ verification:**
- G2.1: Linearizable transactions for registry operations
- G2.2: Atomic lease acquisition/renewal
- G2.3: Key space isolation (actors can't access each other's data)
- G2.4: Transaction conflict detection and retry

#### ADR-004: Linearizability Guarantees
**Guarantees requiring TLA+ verification:**
- G4.1: Operations appear atomic and sequential
- G4.2: Single activation via lease-based ownership
- G4.3: Durable state survives node failures
- G4.4: Exactly-once semantics with idempotency tokens
- G4.5: Failure recovery (actors reactivate after lease expiry)

#### ADR-005: DST Framework
**Guarantees requiring TLA+ verification:**
- G5.1: Deterministic replay via seed
- G5.2: Fault injection doesn't violate safety invariants
- G5.3: All invariants from other ADRs are testable

### Storage ADRs

#### ADR-007: FDB Backend Implementation (keep)
**Guarantees requiring TLA+ verification:**
- G7.1: Transaction atomicity (all-or-nothing)
- G7.2: Read-your-writes consistency within transaction

#### ADR-008: Transaction API (keep, renumber snapshot)
**Guarantees requiring TLA+ verification:**
- G8.1: Buffered writes committed atomically
- G8.2: Rollback restores pre-invocation state
- G8.3: No partial commits on failure

### Agent/Letta ADRs (Application-Level)

These ADRs define API contracts, not distributed system invariants.
TLA+ specs generally NOT required unless they have consistency requirements.

| ADR | TLA+ Needed? | Reason |
|-----|--------------|--------|
| ADR-006 (Letta Compatibility) | No | API contract, not invariant |
| ADR-009 (Memory Tools) | No | Tool definitions |
| ADR-010 (Heartbeat/Pause) | Maybe | Pause state consistency? |
| ADR-011 (Agent Types) | No | Type definitions |
| ADR-012 (Session Storage) | Maybe | Session consistency? |
| ADR-013 (Actor-Based Server) | Covered by ADR-001 | Actor model invariants |
| ADR-014 (Agent Service Layer) | No | Service architecture |

### VM ADRs

#### ADR-018: VmConfig Kernel/Initrd Fields (keep)
No distributed system guarantees - configuration only.

#### ADR-020: Consolidated VM Crate (keep)
**Guarantees requiring TLA+ verification:**
- G20.1: Teleport state consistency (snapshot matches pre-teleport state)
- G20.2: Cross-arch state transfer preserves application state

## Phase 3: TLA+ Spec Inventory

### Existing Specs and Coverage

| Spec | Covers Guarantees | Gaps |
|------|-------------------|------|
| KelpieSingleActivation.tla | G1.1, G4.2 | No liveness, assumes FDB atomicity |
| KelpieRegistry.tla | G2.1 (partial) | No cache model, no migration |
| KelpieActorState.tla | G8.1, G8.2, G8.3, G1.3 | RollbackCorrectness incomplete, no liveness |
| KelpieWAL.tla | G4.4 (partial) | No liveness, no concurrent clients |

### New Specs Needed

| New Spec | Guarantees | Priority |
|----------|------------|----------|
| KelpieClusterMembership.tla | Node join/leave, membership view consistency | High |
| KelpieMigration.tla | 3-phase migration atomicity, G4.5 recovery | High |
| KelpieLease.tla | G2.2, lease renewal/expiry/conflict | High |
| KelpieIdempotency.tla | G4.4 exactly-once with tokens | Medium |
| KelpieTeleport.tla | G20.1, G20.2 snapshot consistency | Medium |
| KelpieActorLifecycle.tla | G1.3, G1.5 idle timeout | Medium |

### Fixes to Existing Specs

| Spec | Fix |
|------|-----|
| All specs | Add liveness properties (eventually operators) |
| KelpieSingleActivation.tla | Model FDB transaction semantics explicitly |
| KelpieActorState.tla | Implement RollbackCorrectness invariant |
| KelpieRegistry.tla | Add node cache model for cache coherence bugs |

## Phase 4: Comprehensive Invariant List

### Safety Invariants (Must Always Hold)

| ID | Invariant | Source ADR | TLA+ Spec |
|----|-----------|------------|-----------|
| S1 | SingleActivation: count(active instances) ≤ 1 | ADR-001, 004 | KelpieSingleActivation |
| S2 | LeaseExclusivity: valid lease → only holder owns actor | ADR-002, 004 | KelpieSingleActivation, KelpieLease |
| S3 | TransactionAtomicity: commit is all-or-nothing | ADR-008 | KelpieActorState |
| S4 | StateConsistency: no active invocation → memory = persisted | ADR-008 | KelpieActorState |
| S5 | CapacityBounds: node actors ≤ capacity | ADR-001 | KelpieRegistry |
| S6 | PlacementConsistency: active → placement exists | ADR-001 | KelpieRegistry |
| S7 | WALDurability: acknowledged op → WAL entry exists | ADR-007 | KelpieWAL |
| S8 | WALIdempotency: same key → same entry | ADR-004 | KelpieWAL |
| S9 | MigrationAtomicity: migration complete → state transferred | NEW | KelpieMigration |
| S10 | TeleportConsistency: restored state = pre-teleport state | ADR-020 | KelpieTeleport |

### Liveness Invariants (Must Eventually Hold)

| ID | Invariant | Source ADR | TLA+ Spec |
|----|-----------|------------|-----------|
| L1 | EventualActivation: claim → eventually active or rejected | ADR-001 | KelpieSingleActivation |
| L2 | EventualDeactivation: idle timeout → eventually deactivated | ADR-001 | KelpieActorLifecycle |
| L3 | EventualFailureDetection: node dead → eventually detected | ADR-004 | KelpieRegistry |
| L4 | EventualRecovery: node failed → actors eventually re-activate | ADR-004 | KelpieMigration |
| L5 | EventualMigration: migration started → eventually complete/failed | NEW | KelpieMigration |

## Execution Checklist

- [x] Phase 1: Delete 5 superseded ADRs
- [x] Phase 1: Renumber ADR-008-snapshot to ADR-021
- [ ] Phase 2: Add "Implementation Status" section to ADR-001, 004, 005 (optional - ADRs are aspirational)
- [x] Phase 3: Document spec inventory in docs/tla/README.md
- [ ] Phase 3: Create stub files for new specs (future work)
- [x] Phase 4: Create invariants.md mapping document (in docs/tla/README.md)

## Quick Decision Log

| Time | Decision | Rationale |
|------|----------|-----------|
| 2026-01-24 | Delete superseded ADRs entirely | User confirmed: superseded = delete, aspirational = keep |
| 2026-01-24 | Keep ADRs aspirational, add impl status | User vision: ADRs define where we want to go |
| 2026-01-24 | Agent ADRs don't need TLA+ | Application-level, not distributed invariants |
