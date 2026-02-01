# Issue #43: Bridge TLA+ Specs to DST Tests

**Status:** IN PROGRESS
**Created:** 2026-01-25
**Issue:** https://github.com/rita-aga/kelpie/issues/43

## Problem

DST tests exist and TLA+ specs exist, but they're disconnected:
- Tests verify "operation succeeded"
- TLA+ specs define WHAT properties must hold
- Tests don't verify THE SAME PROPERTIES as specs

## Solution

1. Implement missing TLA+ invariants in Rust
2. Create `InvariantCheckingSimulation` harness
3. Update existing DST tests with explicit invariant assertions
4. Create spec-to-test mapping documentation

## Current State

**Existing invariants (6):**
- SingleActivation (KelpieSingleActivation.tla)
- ConsistentHolder (KelpieSingleActivation.tla)
- PlacementConsistency (KelpieRegistry.tla)
- LeaseUniqueness (KelpieLease.tla)
- Durability (KelpieWAL.tla)
- AtomicVisibility (KelpieWAL.tla)

**Missing key invariants to add:**
- NoSplitBrain (KelpieClusterMembership.tla)
- ReadYourWrites (KelpieLinearizability.tla)
- MonotonicReads (KelpieLinearizability.tla)
- FencingTokenMonotonic (KelpieLease.tla)
- SnapshotConsistency (KelpieTeleport.tla)

## Phases

### Phase 1: Add Missing Key Invariants ✅ COMPLETE
- [x] NoSplitBrain - cluster membership
- [x] ReadYourWrites - linearizability
- [x] FencingTokenMonotonic - lease safety
- [x] SnapshotConsistency - teleport
- [x] 31 unit tests for all invariants

### Phase 2: Create InvariantCheckingSimulation Harness ✅ COMPLETE
- [x] Create harness that wraps InvariantChecker
- [x] Preset groups: with_cluster_invariants(), with_lease_invariants(), etc.
- [x] check_state() method for manual checking
- [x] State snapshot recording for debugging

### Phase 3: Update Existing DST Tests
- [ ] single_activation_dst.rs - add invariant checks (future PR)
- [ ] lease_dst.rs - add invariant checks (future PR)
- [ ] cluster_membership_dst.rs - add invariant checks (future PR)
- [ ] partition_tolerance_dst.rs - add invariant checks (future PR)

### Phase 4: Create Documentation ✅ COMPLETE
- [x] docs/tla/INVARIANT_MAPPING.md - spec-to-test mapping

## Quick Decision Log

| Time | Decision | Rationale |
|------|----------|-----------|
| Start | Add 5 key invariants first | Cover most critical safety properties |
| Phase 1 | Use operation history for linearizability | Matches TLA+ approach |

## Files to Modify

1. `crates/kelpie-dst/src/invariants.rs` - Add new invariants
2. `crates/kelpie-dst/src/lib.rs` - Export new types
3. `crates/kelpie-dst/tests/*.rs` - Add invariant checks
4. `docs/tla/INVARIANT_MAPPING.md` - New documentation
