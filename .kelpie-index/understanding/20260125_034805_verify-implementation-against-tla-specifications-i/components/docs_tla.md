# docs/tla

**Examined:** 2026-01-25T03:47:12.861773+00:00

## Summary

12 TLA+ specifications defining distributed system invariants for Kelpie: SingleActivation, Lease, WAL, Registry, Migration, Cluster Membership, FDB Transactions, Actor Lifecycle, Actor State, Teleport, Linearizability, and Agent Actor.

## Connections

- kelpie-registry
- kelpie-runtime
- kelpie-storage
- kelpie-cluster
- kelpie-dst

## Details

The TLA+ specs define critical safety and liveness properties:

**Core Safety Invariants:**
- SingleActivation: At most one node active per actor (FDB OCC required)
- LeaseUniqueness: At most one valid lease holder (CAS + fencing tokens)
- MigrationAtomicity: Complete state transfer before target activation
- WAL Durability/Idempotency: Log-before-execute, replay on recovery
- SerializableIsolation: Conflict detection, atomic commits
- NoSplitBrain: Primary election with quorum

**Liveness Properties:**
- EventualActivation, EventualRecovery, EventualLeaseResolution
- EventualMembershipConvergence, EventualDeactivation

**Implementation Requirements from Specs:**
1. FDB optimistic concurrency control (OCC) for all placement operations
2. Fencing tokens for lease-based zombie prevention
3. Grace periods accounting for clock skew
4. 3-phase migration with source deactivation before target activation
5. Term/epoch-based conflict resolution for cluster membership
6. WAL replay on startup for crash recovery

## Issues

### [MEDIUM] TTrace files (6 total) appear to be TLC model checker output traces but are not documented

**Evidence:** docs/tla/*_TTrace_*.tla files exist but no documentation on how to reproduce
