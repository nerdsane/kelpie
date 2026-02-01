# Registry DST Tests TLA+ Alignment

**Plan ID:** 062_20260129_registry_dst_tla_alignment
**Created:** 2026-01-29
**Issue:** #90 - Align Registry DST Tests with TLA+ Invariants
**Status:** COMPLETED

---

## Problem Summary

The `KelpieRegistry.tla` specification defines critical safety invariants that were NOT previously verified by the registry DST tests:

### TLA+ Spec Invariants (from KelpieRegistry.tla)

```tla
SingleActivation ==
    \A a \in Actors :
        Cardinality({n \in Nodes : placement[a] = n}) <= 1

PlacementConsistency ==
    \A a \in Actors :
        placement[a] # NULL => nodeStatus[placement[a]] # Failed
```

### Previous DST Test Gaps (NOW FIXED)

The `registry_actor_dst.rs` tests previously:
- ❌ No multi-node setup (tests run with single simulated node) → ✅ NOW IMPLEMENTED
- ❌ No node failure scenarios (cannot test PlacementConsistency) → ✅ NOW IMPLEMENTED
- ❌ No concurrent placement conflicts (sequential operations only) → ✅ NOW IMPLEMENTED
- ❌ No placement state verification (just checks CRUD operations) → ✅ NOW IMPLEMENTED
- ❌ No invariant checking framework integration → ✅ NOW IMPLEMENTED

---

## Implementation Summary

### New Tests Added (10 new tests)

1. **test_registry_single_activation_invariant** - Concurrent activation with 5 nodes
2. **test_registry_single_activation_high_contention** - 20 nodes racing for same actor
3. **test_registry_placement_consistency_invariant** - Node failure clears placements
4. **test_registry_no_placement_on_failed_node** - Cannot place on failed node
5. **test_registry_node_recovery** - Recovered nodes accept placements
6. **test_registry_placement_race_after_failure** - Race to reclaim after failure
7. **test_registry_single_activation_with_storage_faults** - SingleActivation under faults
8. **test_registry_placement_consistency_with_partition** - PlacementConsistency under partition
9. **test_registry_placement_deterministic** - Same seed = same winner
10. **test_registry_invariants_verified_every_operation** - Verify after every op

### Key Components Implemented

1. **RegistryPlacementState** - Models TLA+ registry state
   - `node_status: HashMap<String, NodeStatus>` - Active/Suspect/Failed
   - `placements: HashMap<String, String>` - actor_id → node_id
   - Converts to `SystemState` for invariant verification

2. **RegistryPlacementProtocol** - Thread-safe registry operations
   - `try_place_actor()` - OCC-style placement with yield points
   - `fail_node()` - Clears all placements on failed node
   - `recover_node()` - Returns node to Active status
   - `verify_invariants()` - Uses InvariantChecker

3. **verify_registry_tla_invariants()** - Standalone verification helper

---

## Acceptance Criteria (ALL COMPLETED)

- [x] Multi-node simulation setup in registry DST
- [x] Test: Concurrent activation → only one succeeds (SingleActivation)
- [x] Test: Node failure → actors not placed on failed nodes (PlacementConsistency)
- [x] Test: Node recovery → proper re-registration
- [x] `verify_tla_invariants()` called after every operation
- [x] Fault injection for node failures, network partitions
- [x] All tests pass: `cargo test -p kelpie-server --test registry_actor_dst --features dst`
- [x] `cargo clippy` passes (no warnings)
- [x] `cargo fmt --check` passes

---

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 2026-01-29 | Use placement model separate from RegistryActor | RegistryActor is for agent metadata, not node placement | Tests model placement semantics independent of agent registry |
| 2026-01-29 | Integrate with existing invariants.rs | Reuse proven InvariantChecker, SingleActivation, PlacementConsistency | Requires state translation to SystemState |
| 2026-01-29 | Use tokio::task::yield_now() for interleaving | Enables concurrent task interleaving deterministically | Requires madsim for true determinism |

---

## Test Results

```
running 15 tests
test test_registry_placement_consistency_invariant ... ok
test test_registry_no_placement_on_failed_node ... ok
test test_registry_node_recovery ... ok
test test_registry_placement_consistency_with_partition ... ok
test test_registry_invariants_verified_every_operation ... ok
test test_registry_single_activation_high_contention ... ok
test test_registry_single_activation_with_storage_faults ... ok
test test_registry_placement_race_after_failure ... ok
test test_registry_placement_deterministic ... ok
test test_registry_single_activation_invariant ... ok
test test_agent_lifecycle_with_registry_dst ... ok
test test_registry_operations_dst ... ok
test test_registry_survives_deactivation_dst ... ok
test test_registry_unregister_dst ... ok
test test_concurrent_registrations_dst ... ok

test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

---

## Files Modified

- `crates/kelpie-server/tests/registry_actor_dst.rs` - Added 10 new TLA+ aligned tests

## Verification Commands

```bash
# Run tests
cargo test -p kelpie-server --test registry_actor_dst --features dst

# Run with specific seed for reproducibility
DST_SEED=42 cargo test -p kelpie-server --test registry_actor_dst --features dst

# Clippy check
cargo clippy -p kelpie-server --test registry_actor_dst --features dst -- -D warnings

# Format check
cargo fmt -p kelpie-server -- --check
```
