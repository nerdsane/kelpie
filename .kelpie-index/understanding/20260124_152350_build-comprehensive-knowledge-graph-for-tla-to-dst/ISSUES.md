# Issues Found During Examination

**Task:** Build comprehensive knowledge graph for TLA+ to DST alignment - extract invariants, state machines, concurrency patterns, TOCTOU risks
**Generated:** 2026-01-24T15:23:50.884819+00:00
**Total Issues:** 27

---

## HIGH (8)

### [kelpie-registry] Zombie actor risk: no coordination between heartbeat failure and lease expiry allows dual activation

**Evidence:** fdb.rs lease mechanism has no node-alive check

*Found: 2026-01-24T15:19:23.735115+00:00*

---

### [kelpie-storage] WAL has no replay mechanism - recovery coordinator missing

**Evidence:** wal.rs pending_entries() exists but no code calls it on startup

*Found: 2026-01-24T15:19:23.864490+00:00*

---

### [kelpie-dst] No invariant verification helpers - tests use weak is_ok()/is_err() assertions

**Evidence:** sandbox_io.rs:348-373 shows pattern

*Found: 2026-01-24T15:22:01.097912+00:00*

---

### [kelpie-dst] Stateright model checking not integrated - only pseudocode exists

**Evidence:** No stateright imports found, no Model trait implementations

*Found: 2026-01-24T15:22:01.097914+00:00*

---

### [kelpie-cluster] join_cluster() is stub - does nothing, single-node only

**Evidence:** cluster.rs:423-435 iterates seeds but takes no action

*Found: 2026-01-24T15:22:01.357646+00:00*

---

### [kelpie-cluster] Failure detection runs but never executes migrations

**Evidence:** cluster.rs:566 TODO(Phase 6)

*Found: 2026-01-24T15:22:01.357647+00:00*

---

### [kelpie-sandbox] State TOCTOU in Firecracker: state read then released then written, allowing interleaving

**Evidence:** firecracker.rs:482-489

*Found: 2026-01-24T15:23:23.663409+00:00*

---

### [kelpie-memory] No thread safety - CoreMemory/WorkingMemory are Clone but not Arc<Mutex>

**Evidence:** Multiple concurrent add_block() calls would race

*Found: 2026-01-24T15:23:23.827317+00:00*

---

## MEDIUM (12)

### [kelpie-server] Shutdown race between initiation and rejection needs atomic state transition

**Evidence:** test_shutdown_with_inflight_requests tests this but fix unclear

*Found: 2026-01-24T15:16:36.429135+00:00*

---

### [kelpie-runtime] Distributed mode TOCTOU race detected but not prevented - client retry required

**Evidence:** dispatcher.rs:512-530, 639-643

*Found: 2026-01-24T15:19:23.476128+00:00*

---

### [kelpie-runtime] Stale registry entries on node crash - no TTL-based cleanup

**Evidence:** dispatcher.rs:667 missing heartbeat coordination

*Found: 2026-01-24T15:19:23.476129+00:00*

---

### [kelpie-registry] try_claim_actor implementation may be incomplete - async reads in sync closure issue

**Evidence:** fdb.rs:603-620 shows _lease_value not awaited

*Found: 2026-01-24T15:19:23.735116+00:00*

---

### [kelpie-storage] Memory transaction not atomic - sequential writes, crash vulnerability

**Evidence:** memory.rs:90-196 commit applies writes sequentially

*Found: 2026-01-24T15:19:23.864492+00:00*

---

### [kelpie-dst] Missing fault types: ConcurrentAccessConflict, DeadlockDetection, DataRace, PartialWrite

**Evidence:** Gap analysis in fault.rs

*Found: 2026-01-24T15:22:01.097915+00:00*

---

### [kelpie-dst] ClockSkew/ClockJump faults declared but never injected

**Evidence:** Time faults not integrated with SimClock

*Found: 2026-01-24T15:22:01.097916+00:00*

---

### [kelpie-cluster] No consensus algorithm - relies on FDB not yet integrated

**Evidence:** lib.rs comments: No consensus algorithm - Designed to use FDB

*Found: 2026-01-24T15:22:01.357649+00:00*

---

### [kelpie-cluster] TcpTransport incomplete - reader_task truncated

**Evidence:** rpc.rs TCP implementation partial

*Found: 2026-01-24T15:22:01.357650+00:00*

---

### [kelpie-sandbox] Async I/O without atomicity - VM could be partially configured if task cancels

**Evidence:** firecracker.rs:540-582

*Found: 2026-01-24T15:23:23.663410+00:00*

---

### [kelpie-memory] Checkpoint not atomic with state mutations - crash during checkpoint loses state

**Evidence:** No WAL visible in checkpoint.rs

*Found: 2026-01-24T15:23:23.827318+00:00*

---

### [kelpie-memory] Expired entries still count toward capacity until pruned

**Evidence:** working.rs expired entries remain in current_bytes

*Found: 2026-01-24T15:23:23.827319+00:00*

---

## LOW (7)

### [kelpie-server] BUG-001/BUG-002 patterns documented but should be verified with DST

**Evidence:** Tests exist but TLA+ invariants not formally defined

*Found: 2026-01-24T15:16:36.429137+00:00*

---

### [kelpie-runtime] No auto-restart of dispatcher task on crash

**Evidence:** runtime.rs:175-185

*Found: 2026-01-24T15:19:23.476131+00:00*

---

### [kelpie-registry] Sequential lock acquisition in MemoryRegistry could allow stale node state

**Evidence:** registry.rs:330-360

*Found: 2026-01-24T15:19:23.735117+00:00*

---

### [kelpie-storage] FDB batch size limit implicit - should be explicit

**Evidence:** fdb.rs transaction has no explicit size check

*Found: 2026-01-24T15:19:23.864493+00:00*

---

### [kelpie-core] StorageBackend::FoundationDb requires fdb_cluster_file but validation is runtime not compile-time

**Evidence:** config.rs:128-132

*Found: 2026-01-24T15:22:01.495518+00:00*

---

### [kelpie-sandbox] Process cleanup race - process might be dead when kill() called

**Evidence:** firecracker.rs:608-612

*Found: 2026-01-24T15:23:23.663411+00:00*

---

### [kelpie-vm] Snapshot checksum is CRC32 - weak for integrity, consider SHA256

**Evidence:** snapshot.rs:85-87

*Found: 2026-01-24T15:23:23.983108+00:00*

---
