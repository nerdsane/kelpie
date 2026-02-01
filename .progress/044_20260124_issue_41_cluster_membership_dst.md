# Issue #41: DST Cluster Membership Tests

## Summary
Implement DST tests for cluster membership protocol (ADR-025) including split-brain, election, heartbeat failure detection, and quorum loss handling.

## Status: Complete

**PR:** https://github.com/rita-aga/kelpie/pull/47

## Context
- ADR-025 defines cluster membership protocol with heartbeat-based failure detection and Raft-style primary election
- TLA+ spec `KelpieClusterMembership.tla` exists with safety invariants (NoSplitBrain, MembershipConsistency)
- PR #39 added `ClusterSplitBrain`, `ReplicationLag`, `QuorumLoss` fault types
- Existing `cluster_dst.rs` tests node registration and heartbeat but not membership protocol

## Options & Decisions

### Test Architecture
1. **Full simulation with SimNetwork** - Use existing SimNetwork partition capabilities
2. **Simulated cluster nodes** - Create simplified cluster model like partition_tolerance_dst.rs
3. **Mix of both** - Use SimNetwork for messaging, simulated nodes for state

**Decision**: Use simulated cluster nodes (option 2), matching partition_tolerance_dst.rs pattern. This is cleaner for testing protocol logic without network complexity.

**Trade-off**: Less realistic network simulation, but faster iteration and clearer test logic.

## Implementation Plan

### Phase 1: Create test file with test infrastructure ✓
- Create `cluster_membership_dst.rs`
- Add `ClusterMember` struct with membership state machine
- Add helpers for quorum checking and partition simulation

### Phase 2: Implement split-brain test ✓
- Test that partitioned cluster doesn't elect multiple primaries
- Verify minority partition can't elect

### Phase 3: Implement primary election convergence ✓
- Test election after primary failure
- Verify bounded convergence time

### Phase 4: Implement heartbeat failure detection ✓
- Test missed heartbeats trigger failure detection
- Verify correct status transitions

### Phase 5: Implement quorum loss test ✓
- Test writes fail without quorum
- Verify correct error type

### Phase 6: Verify and commit
- Run tests
- Create PR

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| Start | Use simulated nodes | Matches existing partition_tolerance_dst.rs pattern | Less realistic |
| Start | Focus on TLA+ invariants | ADR-025 specifies NoSplitBrain as key invariant | |
| Start | Use term-based election | Matches Raft-style approach in ADR-025 | |

## What to Try

### Works Now
- `cargo test -p kelpie-dst cluster_membership` runs all 4+ tests
- Split-brain prevention verified
- Primary election convergence tested
- Heartbeat failure detection tested
- Quorum loss handling tested

### Doesn't Work Yet
- N/A - All specified tests implemented

### Known Limitations
- Tests use simulated nodes, not real cluster implementation
- Actual cluster membership implementation is still "Designed" per ADR-025

## Files Modified/Created
- `crates/kelpie-dst/tests/cluster_membership_dst.rs` - New test file

## Verification
- [x] All tests pass: `cargo test -p kelpie-dst test_membership` - 6 passed, 1 ignored
- [x] No clippy warnings
- [x] Code formatted
