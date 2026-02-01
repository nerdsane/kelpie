# docs/tla

**Examined:** 2026-01-24T19:24:58.614499+00:00

## Summary

10 TLA+ specs covering WAL, Registry, SingleActivation, Lease, Migration, Teleport, FDBTransaction, ClusterMembership, ActorState, ActorLifecycle

## Connections

- docs/adr
- crates/kelpie-storage
- crates/kelpie-registry
- crates/kelpie-cluster

## Details

Each spec has safety invariants, liveness properties, and BUGGY mode for testing. Specs model: WAL (idempotency, recovery), Registry (single activation, failure detection), Lease (TTL, renewal), Migration (3-phase, crash recovery), Teleport (architecture validation), FDB (serializable isolation), Cluster (membership, split-brain), ActorState (rollback), ActorLifecycle (activation ordering).

## Issues

### [HIGH] KelpieLease.tla models lease-based ownership but implementation uses heartbeats, not leases

**Evidence:** Registry impl has HeartbeatTracker, no LeaseManager

### [HIGH] KelpieClusterMembership.tla models split-brain prevention but implementation has none

**Evidence:** Cluster impl has no quorum, no election, no fencing

### [HIGH] KelpieWAL.tla models recovery but implementation has no automatic recovery

**Evidence:** wal.rs has pending_entries() but no recovery orchestration
