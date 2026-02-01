# Issues Found During Examination

**Task:** Verify implementation against TLA+ specifications in docs/tla - check if properties and invariants hold
**Generated:** 2026-01-25T03:48:05.509337+00:00
**Total Issues:** 30

---

## CRITICAL (14)

### [kelpie-registry] SingleActivation invariant VIOLATED - no OCC/CAS for distributed placement

**Evidence:** registry.rs try_claim_actor() has no version comparison

*Found: 2026-01-25T03:47:12.989382+00:00*

---

### [kelpie-registry] FdbRegistry completely unimplemented - all methods are todo!()

**Evidence:** fdb.rs all trait methods

*Found: 2026-01-25T03:47:12.989383+00:00*

---

### [kelpie-registry] No fencing tokens - zombie actors can corrupt state

**Evidence:** Lease struct has no fencing_token field

*Found: 2026-01-25T03:47:12.989385+00:00*

---

### [kelpie-runtime] Idle timeout completely unenforced - should_deactivate() is dead code

**Evidence:** activation.rs:423-429 never called anywhere in codebase

*Found: 2026-01-25T03:47:13.156618+00:00*

---

### [kelpie-storage] WAL recovery never invoked - pending_entries() is dead code on startup

**Evidence:** No call to pending_entries() in lib.rs or main startup path

*Found: 2026-01-25T03:47:58.400928+00:00*

---

### [kelpie-cluster] No consensus mechanism - nodes maintain independent membership views

**Evidence:** cluster.rs register_node() is unilateral, no quorum agreement

*Found: 2026-01-25T03:47:58.561478+00:00*

---

### [kelpie-cluster] check_quorum() defined but NEVER CALLED - split-brain possible

**Evidence:** error.rs:110 exists but grep shows zero calls in cluster code

*Found: 2026-01-25T03:47:58.561479+00:00*

---

### [kelpie-cluster] No primary election - NoSplitBrain invariant violated

**Evidence:** No leader/primary concept in code

*Found: 2026-01-25T03:47:58.561481+00:00*

---

### [kelpie-cluster] join_cluster() is a no-op - TODO comment admits Phase 3 needed

**Evidence:** cluster.rs:134 returns Ok(()) without action

*Found: 2026-01-25T03:47:58.561481+00:00*

---

### [kelpie-cluster] Migration missing source deactivation - dual activation possible

**Evidence:** migrate() calls registry.migrate_actor() but no deactivate RPC to source

*Found: 2026-01-25T03:47:58.561482+00:00*

---

### [kelpie-dst] SingleActivation invariant has ZERO test coverage

**Evidence:** No test for concurrent claim mutual exclusion

*Found: 2026-01-25T03:47:58.683730+00:00*

---

### [kelpie-dst] LeaseUniqueness invariant has ZERO test coverage

**Evidence:** No test for concurrent lease acquire atomicity

*Found: 2026-01-25T03:47:58.683731+00:00*

---

### [kelpie-dst] SerializableIsolation has ZERO test coverage

**Evidence:** No transaction conflict detection tests

*Found: 2026-01-25T03:47:58.683732+00:00*

---

### [kelpie-dst] Network partition/split-brain has ZERO test coverage

**Evidence:** No fault for network partitions between node subsets

*Found: 2026-01-25T03:47:58.683733+00:00*

---

## HIGH (10)

### [kelpie-registry] LeaseManager not integrated with Registry - two parallel paths

**Evidence:** Registry trait doesn't call LeaseManager

*Found: 2026-01-25T03:47:12.989386+00:00*

---

### [kelpie-registry] No grace period for lease expiry - immediate reclaim allows overlap

**Evidence:** lease.rs acquire() has no grace period check

*Found: 2026-01-25T03:47:12.989387+00:00*

---

### [kelpie-registry] No clock skew handling - MAX_CLOCK_SKEW not defined

**Evidence:** lease.rs and node.rs have no clock skew constants

*Found: 2026-01-25T03:47:12.989388+00:00*

---

### [kelpie-runtime] Lifecycle guard uses assert! - optimized away in release builds

**Evidence:** activation.rs:206 assert!(state == Active)

*Found: 2026-01-25T03:47:13.156619+00:00*

---

### [kelpie-runtime] No deactivation guard in dispatcher - invokes can race with deactivation

**Evidence:** dispatcher.rs handle_invoke() has no state check

*Found: 2026-01-25T03:47:13.156620+00:00*

---

### [kelpie-storage] WAL→Execute→Complete not atomic - crash between 2-3 causes duplicates

**Evidence:** storage code shows 3 separate operations without transaction wrapper

*Found: 2026-01-25T03:47:58.400930+00:00*

---

### [kelpie-cluster] No migration rollback - failed migrations leave actor stranded

**Evidence:** migrate() error path just marks failed, no recovery

*Found: 2026-01-25T03:47:58.561484+00:00*

---

### [kelpie-cluster] No term/epoch - older state can override newer

**Evidence:** No term field in cluster state structures

*Found: 2026-01-25T03:47:58.561485+00:00*

---

### [kelpie-dst] WAL crash-recovery replay not tested

**Evidence:** test_eventual_wal_recovery doesn't verify actual persistence

*Found: 2026-01-25T03:47:58.683734+00:00*

---

### [kelpie-dst] MigrationAtomicity mid-failure not tested

**Evidence:** Only happy path and individual component failures tested

*Found: 2026-01-25T03:47:58.683735+00:00*

---

## MEDIUM (6)

### [docs/tla] TTrace files (6 total) appear to be TLC model checker output traces but are not documented

**Evidence:** docs/tla/*_TTrace_*.tla files exist but no documentation on how to reproduce

*Found: 2026-01-25T03:47:12.861784+00:00*

---

### [kelpie-runtime] Race between contains_key and process_invocation during deactivation

**Evidence:** dispatcher.rs:285-348

*Found: 2026-01-25T03:47:13.156621+00:00*

---

### [kelpie-storage] Idempotency lookup is O(n) scan - no index on idempotency_key

**Evidence:** KvWal::find_by_idempotency_key scans all entries

*Found: 2026-01-25T03:47:58.400931+00:00*

---

### [kelpie-storage] WAL cleanup never scheduled - unbounded storage growth

**Evidence:** cleanup() public but never called

*Found: 2026-01-25T03:47:58.400932+00:00*

---

### [kelpie-dst] stress_test_teleport_operations may be non-deterministic

**Evidence:** Uses probabilistic success threshold >= n/3

*Found: 2026-01-25T03:47:58.683737+00:00*

---

### [kelpie-core] check_quorum() helper exists but is unused by cluster code

**Evidence:** error.rs:110-120 defined, cluster.rs doesn't call it

*Found: 2026-01-25T03:47:58.810008+00:00*

---
