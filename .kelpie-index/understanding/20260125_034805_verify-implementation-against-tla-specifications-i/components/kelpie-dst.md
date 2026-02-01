# kelpie-dst

**Examined:** 2026-01-25T03:47:58.683723+00:00

## Summary

Deterministic Simulation Testing framework with 41 files. Has fault injection for teleport, LLM, storage, sandbox. CRITICAL: Does NOT test the TLA+ safety invariants (SingleActivation, LeaseUniqueness, SerializableIsolation) - major coverage gaps.

## Connections

- kelpie-core
- kelpie-runtime
- kelpie-registry
- kelpie-storage
- kelpie-cluster

## Details

**What's Implemented:**
- Simulation harness with SimClock, SimStorage, SimNetwork
- 16+ fault types: storage, network, teleport, LLM, sandbox failures
- Liveness tests for EventualActivation, EventualRecovery, etc.
- BoundedLiveness verification framework
- madsim integration for deterministic task scheduling

**What's Tested:**
✅ Teleport roundtrip under faults
✅ Agent integration with LLM/storage/sandbox faults
✅ Liveness properties (eventual completion)
✅ State transition stress tests

**CRITICAL GAPS - NOT TESTED:**
❌ SingleActivation: No test for mutual exclusion during concurrent claims
❌ LeaseUniqueness: No test for concurrent lease acquire atomicity
❌ SerializableIsolation: No transaction conflict tests at all
❌ MigrationAtomicity: No mid-migration failure scenarios
❌ WALDurability: No crash-recovery replay verification
❌ ClusterMembership: No multi-node view consistency tests
❌ Network Partitions: No split-brain simulation

**Potentially Non-Deterministic Tests:**
- stress_test_teleport_operations: Uses probabilistic success threshold
- Liveness tests: May use wall-clock time instead of simulated time

## Issues

### [CRITICAL] SingleActivation invariant has ZERO test coverage

**Evidence:** No test for concurrent claim mutual exclusion

### [CRITICAL] LeaseUniqueness invariant has ZERO test coverage

**Evidence:** No test for concurrent lease acquire atomicity

### [CRITICAL] SerializableIsolation has ZERO test coverage

**Evidence:** No transaction conflict detection tests

### [CRITICAL] Network partition/split-brain has ZERO test coverage

**Evidence:** No fault for network partitions between node subsets

### [HIGH] WAL crash-recovery replay not tested

**Evidence:** test_eventual_wal_recovery doesn't verify actual persistence

### [HIGH] MigrationAtomicity mid-failure not tested

**Evidence:** Only happy path and individual component failures tested

### [MEDIUM] stress_test_teleport_operations may be non-deterministic

**Evidence:** Uses probabilistic success threshold >= n/3
