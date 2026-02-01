# Issues Found During Examination

**Task:** Understand what Kelpie currently can do and what is properly working, then map the path to formal verification
**Generated:** 2026-01-23T01:04:01.007801+00:00
**Total Issues:** 17

---

## HIGH (1)

### [kelpie-cluster] Cluster coordination not integrated with runtime - distributed single-activation not enforced

**Evidence:** runtime activation.rs lacks distributed lock

*Found: 2026-01-23T01:02:14.272405+00:00*

---

## MEDIUM (6)

### [kelpie-runtime] No distributed lock for single-activation guarantee - only works single-node

**Evidence:** activation.rs lacks distributed coordination

*Found: 2026-01-23T01:01:50.422972+00:00*

---

### [kelpie-storage] FDB tests require external cluster - not run in CI

**Evidence:** 8 ignored tests in fdb.rs

*Found: 2026-01-23T01:02:14.053312+00:00*

---

### [kelpie-cluster] No tests found for kelpie-cluster in test index

**Evidence:** index_tests shows no kelpie-cluster tests

*Found: 2026-01-23T01:02:14.272407+00:00*

---

### [kelpie-registry] PlacementStrategy defined but actual algorithms not implemented

**Evidence:** placement.rs - strategy enum unused in placement logic

*Found: 2026-01-23T01:03:14.735934+00:00*

---

### [kelpie-registry] No actual network heartbeat sending - only tracking

**Evidence:** heartbeat.rs - send_heartbeats flag unused

*Found: 2026-01-23T01:03:14.735936+00:00*

---

### [kelpie-server] Requires external LLM API key for production - not testable without mock

**Evidence:** llm.rs config detection

*Found: 2026-01-23T01:03:14.885921+00:00*

---

## LOW (10)

### [kelpie-runtime] No actor cleanup policy - actors stay in HashMap indefinitely

**Evidence:** dispatcher.rs:max_actors check but no TTL/idle eviction

*Found: 2026-01-23T01:01:50.422974+00:00*

---

### [kelpie-dst] SimTeleportStorage ignores DeterministicRng parameter (_rng unused)

**Evidence:** teleport.rs - HashMap iteration may be non-deterministic

*Found: 2026-01-23T01:01:50.531225+00:00*

---

### [kelpie-dst] Max steps/time limits defined but not enforced in simulation

**Evidence:** simulation.rs - limits in config but no runtime checks

*Found: 2026-01-23T01:01:50.531227+00:00*

---

### [kelpie-storage] No WAL/journaling for crash recovery in MemoryKV

**Evidence:** memory.rs - data lost on crash

*Found: 2026-01-23T01:02:14.053314+00:00*

---

### [kelpie-vm] MockVm command execution is hardcoded (only ~6 commands)

**Evidence:** mock.rs - shell simulation extremely basic

*Found: 2026-01-23T01:02:14.157170+00:00*

---

### [kelpie-vm] Snapshot cleanup ignores errors silently in Firecracker backend

**Evidence:** firecracker.rs - cleanup error suppression

*Found: 2026-01-23T01:02:14.157172+00:00*

---

### [kelpie-registry] Failed nodes stay tracked forever - memory leak risk

**Evidence:** heartbeat.rs - no cleanup for failed nodes

*Found: 2026-01-23T01:03:14.735937+00:00*

---

### [kelpie-server] FDB storage tests require external cluster

**Evidence:** storage/fdb.rs tests ignored

*Found: 2026-01-23T01:03:14.885922+00:00*

---

### [kelpie-wasm] WASM runtime is stub-only - no actual implementation

**Evidence:** lib.rs contains only placeholder struct

*Found: 2026-01-23T01:03:48.737547+00:00*

---

### [kelpie-agent] kelpie-agent is stub-only - agent implementation lives in kelpie-server

**Evidence:** lib.rs contains only placeholder struct

*Found: 2026-01-23T01:03:55.839051+00:00*

---
