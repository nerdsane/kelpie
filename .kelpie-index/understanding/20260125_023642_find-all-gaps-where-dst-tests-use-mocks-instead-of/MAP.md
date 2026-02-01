# Codebase Map

**Task:** Find ALL gaps where DST tests use mocks instead of production code with simulated I/O
**Generated:** 2026-01-25T02:36:42.989038+00:00
**Components:** 5
**Issues Found:** 13

---

## Components Overview

### kelpie-cluster

**Summary:** Cluster crate has NO TimeProvider and uses real network - 80 async functions untestable via DST

**Connects to:** kelpie-dst, kelpie-registry, kelpie-runtime

**Details:**

7 files analyzed:
- rpc.rs: 32 async functions, uses real network (tokio::net) - CRITICAL GAP
- cluster.rs: 18 async functions, has gossip protocol - GAP
- handler.rs: 18 async functions - GAP
- migration.rs: 11 async functions - GAP
- lib.rs: 1 async function
- config.rs, error.rs: No async code

Total: 80 async functions, NONE have TimeProvider injection.
rpc.rs uses real network - cannot be tested with SimNetwork.
Gossip protocol timing cannot be tested under simulated time.

**Issues (3):**
- [HIGH] rpc.rs uses real network (tokio::net) - cannot test with SimNetwork
- [HIGH] kelpie-cluster has 0% TimeProvider coverage - 80 async functions without time injection
- [HIGH] Gossip protocol cannot be tested under simulated time

---

### kelpie-dst

**Summary:** DST test infrastructure with 24 test files - many use mocks instead of production code

**Connects to:** kelpie-runtime, kelpie-registry, kelpie-storage, kelpie-cluster

**Details:**

Analysis of 24 DST test files:
- FULL_SIM (3): single_activation_dst, actor_lifecycle_dst, fdb_transaction_dst
- PARTIAL_SIM (3): agent_integration_dst, fdb_faults_dst, partition_tolerance_dst  
- MOCK_ONLY (2): liveness_dst, lease_dst
- UNKNOWN (16): Various tests without clear simulation strategy

Key finding: Even "FULL_SIM" tests often use HashMap-based mocks for state instead of actual production code with SimStorage.

**Issues (4):**
- [HIGH] 12 tests don't import any production crates - test algorithms only
- [HIGH] 4 tests use Arc<RwLock<HashMap>> mocks instead of SimStorage for state
- [HIGH] 2 tests use MemoryLeaseManager instead of production LeaseManager with SimStorage
- [MEDIUM] 7 tests don't use SimNetwork for RPC simulation

---

### kelpie-registry

**Summary:** Registry has good TimeProvider injection in core files, but fdb.rs (25 async fns) lacks it

**Connects to:** kelpie-dst, kelpie-storage, foundationdb

**Details:**

8 files analyzed:
- registry.rs: Has TimeProvider ✓, 53 async functions - GOOD
- lease.rs: Has TimeProvider ✓, 20 async functions - GOOD
- node.rs: Has TimeProvider ✓ - GOOD
- placement.rs: Has TimeProvider ✓ - GOOD
- fdb.rs: NO TimeProvider, 25 async functions, uses FDB directly - CRITICAL GAP
- lib.rs: Uses FDB, 1 async function - minor
- heartbeat.rs: No async code
- error.rs: Type definitions only

Key issue: fdb.rs is the FDB backend with 25 async functions but no TimeProvider.
This means FDB operations can't be tested under simulated time.

**Issues (2):**
- [HIGH] fdb.rs has 25 async functions with FDB calls but no TimeProvider injection
- [MEDIUM] FDB backend cannot be tested under simulated time - real FDB must be running

---

### kelpie-runtime

**Summary:** Runtime has partial TimeProvider injection but gaps remain in runtime.rs and handle.rs

**Connects to:** kelpie-dst, kelpie-storage, kelpie-core

**Details:**

6 files analyzed:
- dispatcher.rs: Has TimeProvider ✓, 19 async functions
- activation.rs: Has TimeProvider ✓, 17 async functions  
- mailbox.rs: Has TimeProvider ✓
- runtime.rs: NO TimeProvider, 5 async functions - GAP
- handle.rs: NO TimeProvider, 11 async functions - GAP
- lib.rs: Re-exports only

Good: Uses storage abstractions (ActorKV trait), no direct FDB or network calls.
Gap: runtime.rs and handle.rs lack TimeProvider injection for their async code.

**Issues (2):**
- [MEDIUM] runtime.rs has 5 async functions but no TimeProvider injection
- [MEDIUM] handle.rs has 11 async functions but no TimeProvider injection

---

### kelpie-storage

**Summary:** Storage crate has NO TimeProvider injection - 105 async functions without simulated time support

**Connects to:** kelpie-dst, kelpie-runtime, foundationdb

**Details:**

6 files analyzed:
- kv.rs: 23 async functions, NO TimeProvider - GAP
- wal.rs: 38 async functions, NO TimeProvider - GAP  
- memory.rs: 17 async functions, NO TimeProvider - GAP
- fdb.rs: 27 async functions, uses FDB directly, NO TimeProvider - CRITICAL GAP
- transaction.rs: No async code
- lib.rs: Re-exports, uses FDB

Total: 105 async functions across storage crate, NONE have TimeProvider injection.
SimStorage in kelpie-dst implements ActorKV trait, so it CAN substitute for storage in tests,
but the REAL storage code cannot be tested under simulated time.

**Issues (2):**
- [HIGH] kelpie-storage has 0% TimeProvider coverage - 105 async functions without time injection
- [HIGH] Real storage code cannot be tested under simulated time conditions

---

## Component Connections

```
kelpie-cluster -> kelpie-dst, kelpie-registry, kelpie-runtime
kelpie-dst -> kelpie-runtime, kelpie-registry, kelpie-storage, kelpie-cluster
kelpie-registry -> kelpie-dst, kelpie-storage, foundationdb
kelpie-runtime -> kelpie-dst, kelpie-storage, kelpie-core
kelpie-storage -> kelpie-dst, kelpie-runtime, foundationdb
```
