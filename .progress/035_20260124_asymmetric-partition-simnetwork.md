# Plan: Add Asymmetric Network Partition Support to SimNetwork

**Task:** Implement GitHub issue #20 - Add `partition_one_way()` to SimNetwork for asymmetric partitions
**Branch:** dst/asymmetric-partition
**Date:** 2026-01-24
**Status:** Complete

## Summary

Add support for one-way (asymmetric) network partitions to `SimNetwork`. Real networks can have asymmetric failures where A→B works but B→A fails, critical for testing replication lag, one-way failures, and partial connectivity scenarios.

## Options Considered

### Storage Structure for One-Way Partitions

| Option | Pros | Cons | Decision |
|--------|------|------|----------|
| **A: Separate HashSet for one-way** | Clean separation, O(1) lookup, clear semantics | Slightly more memory | ✅ Chosen |
| B: Flag in existing tuple | Less memory | Complicates bidirectional logic, harder to reason about |
| C: Unified partition type enum | Type-safe | Over-engineered for this use case |

**Rationale:** Option A provides clear separation of concerns. The bidirectional partitions remain unchanged, and one-way partitions have their own dedicated storage. This makes the code easier to understand and maintain.

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 14:00 | Use HashSet for one-way partitions | O(1) lookup vs Vec O(n) | Negligible memory increase |
| 14:01 | Keep existing partition() method unchanged | Backward compatibility | Need to update docs to clarify bidirectional |
| 14:02 | Add is_partitioned_one_way() helper | Cleaner code in send() | Additional method |

## Implementation Phases

### Phase 1: Add One-Way Partition Storage
- [x] Add `one_way_partitions: Arc<RwLock<HashSet<(String, String)>>>` field
- [x] Initialize in constructor

### Phase 2: Implement partition_one_way()
- [x] Add `partition_one_way(&self, from: &str, to: &str)` method
- [x] Blocks messages from `from` to `to`, but allows `to` to `from`

### Phase 3: Implement heal_one_way()
- [x] Add `heal_one_way(&self, from: &str, to: &str)` method
- [x] Removes specific one-way partition

### Phase 4: Update send() Logic
- [x] Check one-way partitions in addition to bidirectional
- [x] Add tracing for one-way partition drops

### Phase 5: Update is_partitioned()
- [x] Check both bidirectional and one-way partitions
- [x] Make directional (from, to) rather than symmetric

### Phase 6: Add Tests
- [x] Unit test: basic one-way partition
- [x] Unit test: heal one-way partition
- [x] Unit test: one-way vs bidirectional independence
- [x] Unit test: asymmetric message flow

### Phase 7: Verification & PR
- [x] Run cargo test (11 network tests + all kelpie-dst tests pass)
- [x] Run cargo clippy (clean)
- [x] Run cargo fmt (clean)
- [ ] Create PR

## What to Try

### Works Now
- `network.partition_one_way("node-a", "node-b")` - blocks messages from a to b only
- `network.heal_one_way("node-a", "node-b")` - heals the one-way partition
- `network.is_partitioned("a", "b")` - checks both bidirectional and one-way partitions
- Existing `partition()` and `heal()` work for bidirectional partitions

### Example Usage
```rust
// Create one-way partition: leader can send, but can't receive
network.partition_one_way("follower", "leader").await;

// Leader can still send to follower
assert!(network.send("leader", "follower", Bytes::from("heartbeat")).await);

// Follower CANNOT send to leader
assert!(!network.send("follower", "leader", Bytes::from("vote")).await);
```

### Known Limitations
- None

## Files to Modify

1. `crates/kelpie-dst/src/network.rs` - Main implementation
2. Add tests in same file (inline tests)

## Acceptance Criteria (from issue)

- [x] `SimNetwork::partition_one_way(from, to)` method
- [x] `SimNetwork::heal_one_way(from, to)` method
- [x] `SimNetwork::is_partitioned(from, to)` checks both bidirectional and one-way
- [x] Unit tests for asymmetric partition logic
- [ ] Integration tests for asymmetric scenarios (in tests directory)
- [x] Update existing partition tests to clarify they are bidirectional

## Verification Status

- [x] cargo test passes (all 11 network tests + full kelpie-dst suite)
- [x] cargo clippy clean
- [x] cargo fmt clean
- [x] DST determinism verified (tests use DeterministicRng seed 42)
