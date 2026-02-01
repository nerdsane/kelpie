# kelpie-cluster

**Examined:** 2026-01-25T03:47:58.561470+00:00

## Summary

Cluster membership and migration coordination. CRITICAL: Fails ALL TLA+ ClusterMembership invariants - no consensus mechanism, no quorum enforcement, no primary election, no term/epoch, join_cluster() is a no-op.

## Connections

- kelpie-registry
- kelpie-runtime
- kelpie-core

## Details

**What's Implemented:**
- ClusterManager with transport, registry, state tracking
- Heartbeat broadcasting (one-way, no acks)
- Failure detection via heartbeat timeout
- MigrationCoordinator with 3-phase protocol
- MigrationInfo state machine tracking

**ClusterMembership Spec Violations:**
1. **No membership view**: Nodes maintain independent registries with no consensus
2. **No quorum enforcement**: check_quorum() exists but NEVER CALLED
3. **No primary election**: No leader concept at all
4. **No term/epoch**: No versioning of cluster state changes
5. **Join is no-op**: join_cluster() returns Ok(()) without doing anything (has TODO comment)

**Migration Spec Violations:**
1. **Source deactivation missing**: After transfer, source actor remains active
2. **No rollback on failure**: Failed migrations leave actor in undefined state
3. **No state verification**: No checksum before target activation

**Split-Brain Scenario:**
Partition [A] | [B,C]: Both accept placements without quorum check → state divergence

## Issues

### [CRITICAL] No consensus mechanism - nodes maintain independent membership views

**Evidence:** cluster.rs register_node() is unilateral, no quorum agreement

### [CRITICAL] check_quorum() defined but NEVER CALLED - split-brain possible

**Evidence:** error.rs:110 exists but grep shows zero calls in cluster code

### [CRITICAL] No primary election - NoSplitBrain invariant violated

**Evidence:** No leader/primary concept in code

### [CRITICAL] join_cluster() is a no-op - TODO comment admits Phase 3 needed

**Evidence:** cluster.rs:134 returns Ok(()) without action

### [CRITICAL] Migration missing source deactivation - dual activation possible

**Evidence:** migrate() calls registry.migrate_actor() but no deactivate RPC to source

### [HIGH] No migration rollback - failed migrations leave actor stranded

**Evidence:** migrate() error path just marks failed, no recovery

### [HIGH] No term/epoch - older state can override newer

**Evidence:** No term field in cluster state structures
