# Codebase Map

**Task:** Verify implementation against TLA+ specifications in docs/tla - check if properties and invariants hold
**Generated:** 2026-01-25T03:48:05.509088+00:00
**Components:** 7
**Issues Found:** 30

---

## Components Overview

### docs/tla

**Summary:** 12 TLA+ specifications defining distributed system invariants for Kelpie: SingleActivation, Lease, WAL, Registry, Migration, Cluster Membership, FDB Transactions, Actor Lifecycle, Actor State, Teleport, Linearizability, and Agent Actor.

**Connects to:** kelpie-registry, kelpie-runtime, kelpie-storage, kelpie-cluster, kelpie-dst

**Details:**

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

**Issues (1):**
- [MEDIUM] TTrace files (6 total) appear to be TLC model checker output traces but are not documented

---

### kelpie-cluster

**Summary:** Cluster membership and migration coordination. CRITICAL: Fails ALL TLA+ ClusterMembership invariants - no consensus mechanism, no quorum enforcement, no primary election, no term/epoch, join_cluster() is a no-op.

**Connects to:** kelpie-registry, kelpie-runtime, kelpie-core

**Details:**

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

**Issues (7):**
- [CRITICAL] No consensus mechanism - nodes maintain independent membership views
- [CRITICAL] check_quorum() defined but NEVER CALLED - split-brain possible
- [CRITICAL] No primary election - NoSplitBrain invariant violated
- [CRITICAL] join_cluster() is a no-op - TODO comment admits Phase 3 needed
- [CRITICAL] Migration missing source deactivation - dual activation possible
- [HIGH] No migration rollback - failed migrations leave actor stranded
- [HIGH] No term/epoch - older state can override newer

---

### kelpie-core

**Summary:** Core types, errors, and constants following TigerStyle conventions. Provides foundation for other crates but doesn't directly implement TLA+ invariants - those are in dependent crates.

**Connects to:** kelpie-runtime, kelpie-registry, kelpie-storage, kelpie-cluster, kelpie-dst

**Details:**

**What's Implemented:**
- ActorId, NodeId with validation
- Error types with is_retriable() classification
- Constants with units in names (TigerStyle)
- TimeProvider and RngProvider traits for DST
- check_quorum() helper (unused by callers)

**TigerStyle Compliance:**
✅ Constants with units: ACTOR_INVOCATION_TIMEOUT_MS_MAX
✅ Big-endian naming: actor_state_size_bytes_max
✅ Explicit error handling via Result<T, Error>

**Spec Relevance:**
- Defines error types used by other crates
- Provides TimeProvider/RngProvider for determinism
- check_quorum() exists but callers don't use it

**Issues (1):**
- [MEDIUM] check_quorum() helper exists but is unused by cluster code

---

### kelpie-dst

**Summary:** Deterministic Simulation Testing framework with 41 files. Has fault injection for teleport, LLM, storage, sandbox. CRITICAL: Does NOT test the TLA+ safety invariants (SingleActivation, LeaseUniqueness, SerializableIsolation) - major coverage gaps.

**Connects to:** kelpie-core, kelpie-runtime, kelpie-registry, kelpie-storage, kelpie-cluster

**Details:**

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

**Issues (7):**
- [CRITICAL] SingleActivation invariant has ZERO test coverage
- [CRITICAL] LeaseUniqueness invariant has ZERO test coverage
- [CRITICAL] SerializableIsolation has ZERO test coverage
- [CRITICAL] Network partition/split-brain has ZERO test coverage
- [HIGH] WAL crash-recovery replay not tested
- [HIGH] MigrationAtomicity mid-failure not tested
- [MEDIUM] stress_test_teleport_operations may be non-deterministic

---

### kelpie-registry

**Summary:** Actor placement registry with MemoryRegistry (functional) and FdbRegistry (stub). CRITICAL: Does NOT implement TLA+ SingleActivation invariant - no OCC/CAS, no fencing tokens, lease manager not integrated.

**Connects to:** kelpie-core, kelpie-cluster, kelpie-runtime

**Details:**

**What's Implemented:**
- MemoryRegistry with RwLock-based mutual exclusion (single-process only)
- ActorPlacement with generation field (exists but unused for conflict detection)
- MemoryLeaseManager with basic acquire/release/renew operations
- Node registration and heartbeat timeout detection

**Critical Spec Violations:**
1. **No OCC/CAS**: try_claim_actor() has no version checking - concurrent nodes can both succeed
2. **FdbRegistry is TODO**: All methods return todo!() - distributed case completely unimplemented
3. **Generation field unused**: ActorPlacement.generation exists but never compared during claims
4. **Lease manager not integrated**: LeaseManager exists separately but Registry trait doesn't use it
5. **No fencing tokens**: Zombie prevention mechanism completely absent
6. **Release doesn't invalidate**: unregister_actor() just removes entry, no version bump to invalidate in-flight claims

**Race Condition Example:**
Node A: try_claim(actor-1) → reads v=1
Node B: try_claim(actor-1) → reads v=1
Node A: writes(v=2) → SUCCESS
Node B: writes(v=2) → SUCCESS (no v=1 check!)
Result: Both nodes think they own actor-1

**Issues (6):**
- [CRITICAL] SingleActivation invariant VIOLATED - no OCC/CAS for distributed placement
- [CRITICAL] FdbRegistry completely unimplemented - all methods are todo!()
- [CRITICAL] No fencing tokens - zombie actors can corrupt state
- [HIGH] LeaseManager not integrated with Registry - two parallel paths
- [HIGH] No grace period for lease expiry - immediate reclaim allows overlap
- [HIGH] No clock skew handling - MAX_CLOCK_SKEW not defined

---

### kelpie-runtime

**Summary:** Actor runtime with lifecycle state machine. PARTIAL compliance with TLA+ ActorLifecycle spec - state transitions exist but idle timeout never enforced, lifecycle guard uses assert! not runtime check.

**Connects to:** kelpie-core, kelpie-registry, kelpie-storage

**Details:**

**What's Implemented:**
- ActivationState enum: Activating → Active → Deactivating → Deactivated
- ActiveActor with idle_timeout field and should_deactivate() method
- process_invocation_with_time() with state assertion
- Deactivation drains mailbox and calls on_deactivate()

**Spec Violations:**
1. **Idle timeout never enforced**: should_deactivate() exists but is NEVER CALLED - dead code
2. **Assert not runtime check**: process_invocation_with_time() uses assert!(state == Active) which is optimized away in release builds
3. **No deactivation guard in dispatcher**: handle_invoke() doesn't check for Deactivating state before routing
4. **Race condition**: Between actors.contains_key() and process_invocation(), deactivation can occur

**What Works:**
- State transitions are properly ordered
- Deactivation drains pending messages
- on_activate/on_deactivate hooks called correctly

**Issues (4):**
- [CRITICAL] Idle timeout completely unenforced - should_deactivate() is dead code
- [HIGH] Lifecycle guard uses assert! - optimized away in release builds
- [HIGH] No deactivation guard in dispatcher - invokes can race with deactivation
- [MEDIUM] Race between contains_key and process_invocation during deactivation

---

### kelpie-storage

**Summary:** WAL and KV storage with MemoryWal/KvWal and transaction support. PARTIAL TLA+ compliance - WAL exists with idempotency, atomic commits work, but recovery never invoked and WAL→Execute→Complete not atomic as unit.

**Connects to:** kelpie-core, kelpie-runtime

**Details:**

**What's Implemented:**
- WAL with WalEntry struct (id, operation, status, idempotency_key)
- MemoryWal and KvWal implementations with JSON serialization
- append_with_idempotency() for duplicate detection
- Transaction buffering with read-your-writes semantics
- FDB backend delegates to FoundationDB's MVCC

**Spec Compliance:**
✅ Read-your-writes: Write buffer checked before storage
✅ Idempotency tracking: Duplicates detected by idempotency_key
✅ Atomic commit: FDB provides ACID, Memory has buffer apply
⚠️ Recovery: pending_entries() exists but NEVER CALLED on startup
⚠️ WAL+Execute+Complete not atomic: Crash between steps 2-3 causes duplicate execution

**Missing:**
- No recovery orchestration on startup
- No state verification (checksum) for WAL entries
- O(n) scan for idempotency lookup (no index)
- cleanup() never scheduled - unbounded growth

**Issues (4):**
- [CRITICAL] WAL recovery never invoked - pending_entries() is dead code on startup
- [HIGH] WAL→Execute→Complete not atomic - crash between 2-3 causes duplicates
- [MEDIUM] Idempotency lookup is O(n) scan - no index on idempotency_key
- [MEDIUM] WAL cleanup never scheduled - unbounded storage growth

---

## Component Connections

```
docs/tla -> kelpie-registry, kelpie-runtime, kelpie-storage, kelpie-cluster, kelpie-dst
kelpie-cluster -> kelpie-registry, kelpie-runtime, kelpie-core
kelpie-core -> kelpie-runtime, kelpie-registry, kelpie-storage, kelpie-cluster, kelpie-dst
kelpie-dst -> kelpie-core, kelpie-runtime, kelpie-registry, kelpie-storage, kelpie-cluster
kelpie-registry -> kelpie-core, kelpie-cluster, kelpie-runtime
kelpie-runtime -> kelpie-core, kelpie-registry, kelpie-storage
kelpie-storage -> kelpie-core, kelpie-runtime
```
