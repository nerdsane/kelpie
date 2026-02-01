# 052 FdbRegistry Multi-Node Cluster Membership

**Spec:** specs/077-fdbregistry-multinode-cluster.md
**Issue:** #77
**Started:** 2026-01-27
**Status:** COMPLETE
**Completed:** 2026-01-27

## Objective

Implement distributed cluster membership via FoundationDB for multi-node Kelpie deployments, including:
- Node state machine matching TLA+ specification
- Primary election with Raft-style terms
- Heartbeat-based failure detection
- Membership view synchronization
- Split-brain prevention
- Actor migration on node failure

## Implementation Summary

### Files Created/Modified

1. **`crates/kelpie-registry/src/membership.rs`** (NEW)
   - `NodeState` enum matching TLA+ (Left, Joining, Active, Leaving, Failed)
   - `PrimaryInfo` struct (node_id, term, elected_at_ms)
   - `MembershipView` struct (active_nodes, view_number, quorum calculations)
   - `ClusterState` struct for DST invariant checking
   - State transition validation

2. **`crates/kelpie-registry/src/cluster.rs`** (NEW, FDB feature-gated)
   - `ClusterMembership` - FDB-backed cluster membership manager
   - `ClusterNodeInfo` - Node info stored in FDB
   - `MigrationCandidate`, `MigrationResult`, `MigrationQueue` - Actor migration types
   - Node join/leave operations
   - Primary election with quorum checks
   - Heartbeat and failure detection
   - Actor migration on node failure

3. **`crates/kelpie-dst/tests/cluster_membership_production_dst.rs`** (NEW)
   - 8 DST tests verifying TLA+ invariants
   - Uses production types (MembershipView, NodeState, PrimaryInfo)
   - Tests: NoSplitBrain, quorum, step-down, heartbeat, partition heal, determinism, state transitions, actor migration

### Implementation Plan

### Phase 1: Core Types (FR-1)
- [x] Add `NodeState` enum matching TLA+ exactly
- [x] Keep `NodeStatus` for backwards compatibility
- [x] Add state transition validation

### Phase 2: Cluster Membership Types (FR-2, FR-5)
- [x] Add `PrimaryInfo` struct
- [x] Add `MembershipView` struct
- [x] Extend FdbRegistry with cluster membership keys
- [x] Add FDB schema for /kelpie/cluster/*

### Phase 3: Primary Election (FR-2, FR-3)
- [x] Implement `try_become_primary()` with quorum check
- [x] Implement `step_down()` on quorum loss
- [x] Add primary term counter in FDB

### Phase 4: Failure Detection (FR-4)
- [x] Integrate heartbeat timeout with node state transitions
- [x] Mark nodes as Failed when heartbeat times out

### Phase 5: Partition Handling (FR-6)
- [x] Implement quorum checking for all operations
- [x] Primary step-down on partition

### Phase 6: Actor Migration (FR-7)
- [x] Implement `MigrationQueue` for tracking actors needing migration
- [x] Implement `queue_actors_for_migration()` called when node fails
- [x] Implement `process_migration_queue()` processed by primary
- [x] Maintain single activation guarantee during migration

### Phase 7: DST Tests
- [x] Create tests using production types
- [x] Add invariant verification (NoSplitBrain, MembershipConsistency)
- [x] Test actor migration and single activation guarantee

## Verification

```bash
cargo test -p kelpie-registry --features fdb  # 80 passed
cargo test -p kelpie-dst --test cluster_membership_production_dst  # 8 passed
cargo clippy -p kelpie-registry --features fdb  # Clean (deprecation warnings pre-existing)
cargo fmt -p kelpie-registry -p kelpie-dst -- --check  # Clean
```

## Progress Log

### 2026-01-27 Session Start
- Analyzed existing codebase
- Created implementation plan
- Starting with Phase 1: Core Types

### 2026-01-27 Session Complete
- Implemented all phases FR-1 through FR-7
- All 8 DST tests passing
- Spec marked COMPLETE

