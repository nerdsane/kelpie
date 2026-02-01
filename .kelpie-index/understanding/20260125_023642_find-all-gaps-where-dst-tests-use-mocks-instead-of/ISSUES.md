# Issues Found During Examination

**Task:** Find ALL gaps where DST tests use mocks instead of production code with simulated I/O
**Generated:** 2026-01-25T02:36:42.989289+00:00
**Total Issues:** 13

---

## HIGH (9)

### [kelpie-dst] 12 tests don't import any production crates - test algorithms only

**Evidence:** liveness_dst.rs, agent_integration_dst.rs, teleport_service_dst.rs, memory_dst.rs, vm_teleport_dst.rs, proper_dst_demo.rs, integration_chaos_dst.rs, bug_hunting_dst.rs

*Found: 2026-01-25T02:35:32.476677+00:00*

---

### [kelpie-dst] 4 tests use Arc<RwLock<HashMap>> mocks instead of SimStorage for state

**Evidence:** single_activation_dst.rs uses ActivationProtocol with HashMap, partition_tolerance_dst.rs similar

*Found: 2026-01-25T02:35:32.476679+00:00*

---

### [kelpie-dst] 2 tests use MemoryLeaseManager instead of production LeaseManager with SimStorage

**Evidence:** liveness_dst.rs, lease_dst.rs

*Found: 2026-01-25T02:35:32.476680+00:00*

---

### [kelpie-registry] fdb.rs has 25 async functions with FDB calls but no TimeProvider injection

**Evidence:** registry_analysis shows fdb.rs: uses_fdb=true, has_time_injection=false, async_functions=25

*Found: 2026-01-25T02:36:03.600058+00:00*

---

### [kelpie-storage] kelpie-storage has 0% TimeProvider coverage - 105 async functions without time injection

**Evidence:** kv.rs(23), wal.rs(38), memory.rs(17), fdb.rs(27) all lack TimeProvider

*Found: 2026-01-25T02:36:25.954375+00:00*

---

### [kelpie-storage] Real storage code cannot be tested under simulated time conditions

**Evidence:** No TimeProvider in any storage file

*Found: 2026-01-25T02:36:25.954377+00:00*

---

### [kelpie-cluster] rpc.rs uses real network (tokio::net) - cannot test with SimNetwork

**Evidence:** cluster_analysis shows rpc.rs: uses_network=true, 32 async functions

*Found: 2026-01-25T02:36:34.006134+00:00*

---

### [kelpie-cluster] kelpie-cluster has 0% TimeProvider coverage - 80 async functions without time injection

**Evidence:** All 7 files show has_time_injection=false

*Found: 2026-01-25T02:36:34.006136+00:00*

---

### [kelpie-cluster] Gossip protocol cannot be tested under simulated time

**Evidence:** cluster.rs has gossip logic but no time injection

*Found: 2026-01-25T02:36:34.006137+00:00*

---

## MEDIUM (4)

### [kelpie-dst] 7 tests don't use SimNetwork for RPC simulation

**Evidence:** Most tests lack network fault injection at I/O layer

*Found: 2026-01-25T02:35:32.476681+00:00*

---

### [kelpie-runtime] runtime.rs has 5 async functions but no TimeProvider injection

**Evidence:** runtime_analysis shows has_time_injection: false

*Found: 2026-01-25T02:35:48.163572+00:00*

---

### [kelpie-runtime] handle.rs has 11 async functions but no TimeProvider injection

**Evidence:** runtime_analysis shows has_time_injection: false

*Found: 2026-01-25T02:35:48.163573+00:00*

---

### [kelpie-registry] FDB backend cannot be tested under simulated time - real FDB must be running

**Evidence:** fdb.rs directly uses foundationdb crate without abstraction

*Found: 2026-01-25T02:36:03.600060+00:00*

---
