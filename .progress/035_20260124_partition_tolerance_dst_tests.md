# Plan: DST Partition Tolerance Tests

**Issue:** #18 - DST: Add network partition tolerance testing
**Status:** Complete
**Created:** 2026-01-24
**Completed:** 2026-01-24

## Objective

Add partition tolerance tests to verify CP semantics from ADR-004:
- Minority partitions become unavailable
- Majority partitions continue serving
- No split-brain on partition healing

## Options & Decisions

### Option 1: Full Quorum Implementation in Cluster
- **Pros:** Production-ready quorum checking, complete CP semantics
- **Cons:** Requires significant refactor of Cluster, FDB integration for true quorum
- **Verdict:** Too large for this issue - defer full implementation

### Option 2: SimCluster with Quorum Checking for DST (Chosen)
- **Pros:** Tests CP semantics with simulated quorum, focused scope
- **Cons:** SimCluster separate from production Cluster
- **Verdict:** Good for DST validation, establishes test patterns
- **Trade-off:** Tests verify the pattern works, production impl deferred

### Option 3: Just Add SimNetwork Helpers
- **Pros:** Minimal changes
- **Cons:** Can't actually test quorum behavior without quorum logic
- **Verdict:** Insufficient - need some quorum checking to test

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| Start | Use SimCluster pattern | Keeps DST self-contained | Separate from production |
| Start | Add NoQuorum error variant | Needed for explicit failure | Minor API addition |
| Start | Group partition helpers | Better test ergonomics | Simple extension |

## Phases

### Phase 1: Extend SimNetwork ✅
- [x] Add `partition_group()` method for group partitions
- [x] Add `partition_one_way()` for asymmetric partitions
- [x] Add `heal_one_way()` for asymmetric healing
- [x] Add tests for new methods

### Phase 2: Add NoQuorum Error ✅
- [x] Add `NoQuorum` variant to `ClusterError`
- [x] Include context (needed nodes, available nodes)

### Phase 3: Create Partition Tolerance Tests ✅
- [x] `test_minority_partition_unavailable` - 5 nodes, 2|3 split
- [x] `test_majority_partition_continues` - 5 nodes, majority serves
- [x] `test_symmetric_partition_both_unavailable` - 4 nodes, 2|2 split
- [x] `test_partition_healing_no_split_brain` - partition → operations → heal → verify consistency
- [x] `test_asymmetric_partition` - one-way partition behavior

### Phase 4: Verification ✅
- [x] All tests pass with `cargo test -p kelpie-dst`
- [x] Determinism verified (same seed = same result)
- [x] No clippy warnings

## What to Try

### Works Now
- `cargo test -p kelpie-dst partition_tolerance` - runs the new tests
- `DST_SEED=12345 cargo test -p kelpie-dst partition_tolerance` - reproducible run

### Known Limitations
- Tests use SimCluster (simplified quorum), not production Cluster
- Asymmetric partition tests require network one-way support
- Production CP semantics require FDB backend (Phase 3)

## Files Modified

- `crates/kelpie-dst/src/network.rs` - Add partition helpers
- `crates/kelpie-cluster/src/error.rs` - Add NoQuorum error
- `crates/kelpie-dst/tests/partition_tolerance_dst.rs` - New test file

## Completion Notes

Implemented partition tolerance DST tests as specified in issue #18. The tests verify:
1. Minority partition becomes unavailable (returns NoQuorum error)
2. Majority partition continues serving operations
3. Symmetric split (2|2) makes both sides unavailable
4. Partition healing results in consistent state (no split-brain)
5. Asymmetric partitions handled correctly

All tests are deterministic and can be reproduced with `DST_SEED`.
