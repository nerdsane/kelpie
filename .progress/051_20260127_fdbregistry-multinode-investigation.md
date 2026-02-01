# FdbRegistry Multi-Node Investigation & Implementation Plan

**Status:** PLANNING
**Created:** 2026-01-27
**Issue:** #77 - FdbRegistry for Multi-Node Deployment
**Related ADRs:** ADR-025 (Cluster Membership), ADR-023 (Actor Registry)
**Related TLA+:** KelpieClusterMembership.tla

## Executive Summary

**Critical Finding:** The cluster membership protocol exists in TLA+ and ADR but is NOT implemented. DST tests verify a simulation model, not the real implementation. FdbRegistry has actor placement/leases but no cluster membership (0 primary election, 0 quorum, 0 partition handling).

## Investigation Findings

### 1. What Exists (Designed, Spec'd)

| Component | Status | Location |
|-----------|--------|----------|
| Node state machine (Left→Joining→Active→Leaving→Failed) | TLA+ only | KelpieClusterMembership.tla |
| Heartbeat-based failure detection | TLA+ only | KelpieClusterMembership.tla |
| Primary election (Raft-style terms) | TLA+ only | KelpieClusterMembership.tla |
| Quorum-based split-brain prevention | TLA+ only | KelpieClusterMembership.tla |
| Partition handling | TLA+ only | KelpieClusterMembership.tla |

### 2. What Exists (Implemented)

| Component | Status | Location | Notes |
|-----------|--------|----------|-------|
| FdbRegistry | 1291 lines | kelpie-registry/src/fdb.rs | Actor placement, leases, heartbeat tracking |
| Heartbeat tracking | Implemented | kelpie-registry | 36 mentions, heartbeat receipt works |
| Node registration | Implemented | kelpie-registry | register_node/unregister_node |
| Cluster struct | Implemented | kelpie-cluster/src/cluster.rs | But TODOs say FdbRegistry is NOT used |

### 3. Critical Gaps (TLA+ vs Implementation)

| TLA+ Spec | Implementation | Gap |
|-----------|----------------|-----|
| `NodeState` enum (5 states) | `ClusterState` enum (different) | **State machine mismatch** |
| `believesPrimary` | None (0 mentions of "primary") | **No primary election** |
| `primaryTerm` (Raft epochs) | None | **No term-based ordering** |
| `CanBecomePrimary` quorum check | None (0 mentions of "quorum") | **No quorum enforcement** |
| `HasValidPrimaryClaim` | None | **No split-brain prevention** |
| `PrimaryStepDown` on partition | None (0 mentions of "partition") | **No partition handling** |
| `membershipView` synchronization | None | **No view sync** |

### 4. DST Test Analysis

| Test File | Lines | Uses Real Code? | Notes |
|-----------|-------|-----------------|-------|
| cluster_membership_dst.rs | 37KB | **NO** | Uses `ClusterMember` struct that "models TLA+" |
| partition_tolerance_dst.rs | 26KB | **NO** | Uses `SimClusterNode` |
| cluster_dst.rs | 48KB | Partial | Some real cluster code |

**Key Quote from Tests:**
> "Production quorum checking will be via FDB transactions."

### 5. Cluster Crate TODOs

```rust
// Line 282: TODO(Phase 3): This currently does nothing. Once FdbRegistry is implemented,
// cluster membership will be managed through FDB transactions instead of gossip.

// Line 290: TODO(Phase 3): Use FDB for cluster membership discovery
// Seed nodes will point to FDB coordinators, not peer Kelpie nodes

// Line 412: TODO(Phase 6): Execute migration via MigrationCoordinator
// Requires cluster RPC for state transfer
```

### 6. DST Harness Capabilities

The harness is comprehensive:
- **Fault types:** StorageWriteFail, StorageReadFail, StorageCorruption, StorageLatency, DiskFull, CrashBeforeWrite, CrashAfterWrite, CrashDuringTransaction, NetworkPartition, NetworkDelay, NetworkPacketLoss, NetworkMessageReorder, ClockSkew, ClockJump, OutOfMemory
- **SimNetwork:** Simulated network with partitions
- **SimClock:** Deterministic time
- **SimStorage:** Simulated storage
- **madsim:** Deterministic task scheduling
- **InvariantChecker:** Runtime invariant validation

### 7. Existing Infrastructure (Phase 1 Complete)

Per `.progress/048_20260125_true_dst_simulation_architecture.md`:
- TimeProvider injection in kelpie-storage WAL: **COMPLETE**
- NetworkProvider abstraction: **NOT COMPLETE**
- Cluster TimeProvider injection: **NOT COMPLETE**
- DST tests using production code: **NOT COMPLETE**

## Architecture: What FdbRegistry for Multi-Node Requires

### Component 1: Cluster Membership in FDB

```
FDB Key Schema:
/kelpie/cluster/nodes/{node_id}           -> NodeInfo + NodeState
/kelpie/cluster/membership_view           -> MembershipView (all active nodes)
/kelpie/cluster/primary                   -> PrimaryInfo (node_id, term)
/kelpie/cluster/primary_term              -> u64 (monotonically increasing)
```

### Component 2: Primary Election Protocol

1. Node wants to become primary
2. FDB transaction:
   - Read current primary_term
   - Read all node states (calculate reachable majority)
   - If no valid primary AND has quorum: set self as primary with term+1
   - Use FDB versionstamp for linearizability

### Component 3: Failure Detection Integration

1. Heartbeats written to FDB (already implemented)
2. Background process checks heartbeat timestamps
3. If node heartbeat expired:
   - Mark node as Failed in FDB (transactional)
   - If failed node was primary: trigger re-election
   - Trigger actor migration for actors on failed node

### Component 4: Partition-Safe Quorum

1. Primary continuously verifies quorum via FDB
2. If FDB transaction fails (primary is partitioned from FDB): step down
3. FDB coordinators provide the quorum guarantee (leveraging FDB's own consensus)

## Implementation Plan

### Phase A: State Machine Alignment (3-4 days)

**Goal:** Implement TLA+ node state machine in Rust

1. Create `NodeState` enum matching TLA+:
   ```rust
   pub enum NodeState {
       Left,
       Joining,
       Active,
       Leaving,
       Failed,
   }
   ```

2. Update `NodeInfo` to include state machine
3. Add state transition methods with TigerStyle assertions
4. Add FDB transaction for state transitions

**DST Verification:**
- [ ] State machine transitions match TLA+ exactly
- [ ] Invalid transitions are rejected
- [ ] Concurrent transitions are serialized

### Phase B: Primary Election (4-5 days)

**Goal:** Implement Raft-style primary election via FDB

1. Add `PrimaryInfo` struct:
   ```rust
   pub struct PrimaryInfo {
       node_id: NodeId,
       term: u64,
       elected_at_ms: u64,
   }
   ```

2. Implement `try_become_primary()`:
   - FDB transaction that checks quorum and no valid primary
   - Increments term atomically
   - Uses FDB versionstamp for ordering

3. Implement `PrimaryStepDown`:
   - Called when primary loses quorum
   - Clears primary status in FDB

**DST Verification:**
- [ ] NoSplitBrain invariant holds under all fault schedules
- [ ] Higher term always wins
- [ ] Minority partition cannot elect primary

### Phase C: Membership View Synchronization (3-4 days)

**Goal:** Implement view synchronization via FDB

1. Add `MembershipView` stored in FDB:
   ```rust
   pub struct MembershipView {
       active_nodes: HashSet<NodeId>,
       view_number: u64,
   }
   ```

2. Use FDB watches to detect membership changes
3. Implement view merge on partition heal

**DST Verification:**
- [ ] MembershipConsistency invariant holds
- [ ] View numbers are monotonic
- [ ] Partition heal triggers view sync

### Phase D: Integration with Actor Registry (2-3 days)

**Goal:** Connect cluster membership to actor placement

1. Failure detection triggers actor migration
2. Primary coordinates migration decisions
3. Only nodes in Active state can host actors

**DST Verification:**
- [ ] Actors on failed nodes are migrated
- [ ] No actor activation on non-Active nodes
- [ ] Single activation maintained during migration

### Phase E: DST Test Migration (5-7 days)

**Goal:** Make DST tests use real implementation, not mocks

1. Replace `ClusterMember` simulation with real `Cluster` struct
2. Replace `SimClusterNode` with real `FdbRegistry`
3. Inject SimNetwork, SimClock, SimStorage into production code
4. Verify TLA+ invariants against real implementation

**Verification:**
- [ ] All TLA+ invariants (NoSplitBrain, MembershipConsistency, JoinAtomicity, LeaveDetectionWeak) verified
- [ ] Tests run against production code, not mocks
- [ ] Same seed = same result

## DST Requirements (FDB-Level Simulation)

### Fault Injection Coverage

| Fault | TLA+ Action | Must Test |
|-------|-------------|-----------|
| NetworkPartition | CreatePartition, HealPartition | Primary in minority steps down |
| NodeCrash | MarkNodeFailed | Failure detection triggers migration |
| HeartbeatMiss | DetectFailure | Suspect → Failed transition |
| FDB Transaction Conflict | N/A (FDB handles) | Retry logic works |
| Clock Skew | ClockSkew | Heartbeat expiry still works |
| Message Reorder | NetworkMessageReorder | View sync handles out-of-order |

### Invariant Checks

From KelpieClusterMembership.tla:
- `NoSplitBrain`: At most one node has a valid primary claim
- `MembershipConsistency`: Active nodes with same view number have same view
- `JoinAtomicity`: Node is either fully joined or not joined
- `LeaveDetectionWeak`: Left nodes not in any active node's view
- `TypeOK`: All variables have correct types

### Liveness Properties

- `EventualMembershipConvergence`: When network heals and stable, all active nodes have same view

## Success Criteria

1. [ ] Two+ nodes can form a cluster via FDB
2. [ ] Primary election works (one primary at a time)
3. [ ] Node failure detected via heartbeat timeout
4. [ ] Failed node's actors are migrated
5. [ ] Minority partition cannot elect primary (no split-brain)
6. [ ] DST tests verify TLA+ invariants against REAL implementation
7. [ ] `DST_SEED=X cargo test cluster_membership` is reproducible

## Effort Estimate

| Phase | Days | Risk |
|-------|------|------|
| A: State Machine Alignment | 3-4 | Low |
| B: Primary Election | 4-5 | Medium |
| C: Membership View Sync | 3-4 | Medium |
| D: Actor Registry Integration | 2-3 | Low |
| E: DST Test Migration | 5-7 | Medium |
| **Total** | **17-23** | |

## Dependencies

- **Blocked by:** None (builds on existing FdbRegistry)
- **Depends on:** FDB running for integration tests
- **Enables:** Multi-node deployment, automatic failover

## References

- [ADR-025: Cluster Membership Protocol](docs/adr/025-cluster-membership-protocol.md)
- [ADR-023: Actor Registry Design](docs/adr/023-actor-registry-design.md)
- [KelpieClusterMembership.tla](docs/tla/KelpieClusterMembership.tla)
- [FoundationDB Testing Paper](https://www.foundationdb.org/files/fdb-paper.pdf)
- [Progress 048: True DST Simulation Architecture](.progress/048_20260125_true_dst_simulation_architecture.md)
