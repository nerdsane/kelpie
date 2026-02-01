# Issues Found During Examination

**Task:** Identify gaps between implementation attempts, ADR coverage, and TLA+ models
**Generated:** 2026-01-24T19:25:36.577164+00:00
**Total Issues:** 17

---

## CRITICAL (3)

### [crates/kelpie-registry] KelpieLease.tla models lease-based ownership but implementation uses heartbeats only

**Evidence:** No LeaseManager, no TTL expiration, no lease renewal in registry impl

*Found: 2026-01-24T19:25:24.404829+00:00*

---

### [crates/kelpie-cluster] KelpieClusterMembership.tla models split-brain prevention but implementation has none

**Evidence:** No quorum, no election, no fencing in cluster impl

*Found: 2026-01-24T19:25:24.536354+00:00*

---

### [crates/kelpie-cluster] Migration lacks explicit source deactivation - can violate single activation

**Evidence:** migration.rs: complete_migration() activates target without confirming source stopped

*Found: 2026-01-24T19:25:24.536356+00:00*

---

## HIGH (10)

### [docs/adr] ADRs don't reference which TLA+ specs verify their claims

**Evidence:** No ADR mentions a TLA+ spec by name except ADR-004 which mentions lease protocol but no spec file

*Found: 2026-01-24T19:24:58.487330+00:00*

---

### [docs/tla] KelpieLease.tla models lease-based ownership but implementation uses heartbeats, not leases

**Evidence:** Registry impl has HeartbeatTracker, no LeaseManager

*Found: 2026-01-24T19:24:58.614510+00:00*

---

### [docs/tla] KelpieClusterMembership.tla models split-brain prevention but implementation has none

**Evidence:** Cluster impl has no quorum, no election, no fencing

*Found: 2026-01-24T19:24:58.614512+00:00*

---

### [docs/tla] KelpieWAL.tla models recovery but implementation has no automatic recovery

**Evidence:** wal.rs has pending_entries() but no recovery orchestration

*Found: 2026-01-24T19:24:58.614513+00:00*

---

### [crates/kelpie-storage] WAL idempotency check is not atomic - race condition between find and insert

**Evidence:** wal.rs: append_with_idempotency() calls find_by_idempotency_key() then append() non-atomically

*Found: 2026-01-24T19:25:24.232752+00:00*

---

### [crates/kelpie-storage] WAL has no automatic crash recovery - only provides pending_entries()

**Evidence:** wal.rs: no recovery orchestration, caller must implement

*Found: 2026-01-24T19:25:24.232753+00:00*

---

### [crates/kelpie-registry] Placement has no distributed coordination - relies on external FDB but FDB integration incomplete

**Evidence:** fdb.rs exists but ADR-004 says 'FDB backend integration not started'

*Found: 2026-01-24T19:25:24.404830+00:00*

---

### [crates/kelpie-registry] Generation counter alone insufficient for single activation - two nodes could both see gen=1

**Evidence:** placement.rs: no atomic read-check-write with FDB

*Found: 2026-01-24T19:25:24.404831+00:00*

---

### [crates/kelpie-cluster] No persistent migration journal - crashes lose in-flight migration state

**Evidence:** migration.rs: no WAL or checkpoint for migration state

*Found: 2026-01-24T19:25:24.536357+00:00*

---

### [crates/kelpie-server] Server relies on registry single-activation but registry lacks lease-based guarantees

**Evidence:** Server assumes single activation but registry uses heartbeats not leases

*Found: 2026-01-24T19:25:24.656365+00:00*

---

## MEDIUM (4)

### [docs/adr] Lease protocol in ADR-004 has no corresponding lease TLA+ spec mapping

**Evidence:** KelpieLease.tla exists but ADR-004 doesn't reference it

*Found: 2026-01-24T19:24:58.487332+00:00*

---

### [crates/kelpie-storage] MemoryWal provides no durability - test-only

**Evidence:** wal.rs: in-memory storage loses all data on crash

*Found: 2026-01-24T19:25:24.232754+00:00*

---

### [crates/kelpie-cluster] Join protocol not implemented

**Evidence:** handler.rs: 'ignoring join request (not implemented)'

*Found: 2026-01-24T19:25:24.536358+00:00*

---

### [crates/kelpie-server] AgentActor crash recovery depends on incomplete WAL recovery

**Evidence:** ADR-013 mentions checkpoint every iteration but WAL has no auto-recovery

*Found: 2026-01-24T19:25:24.656366+00:00*

---
