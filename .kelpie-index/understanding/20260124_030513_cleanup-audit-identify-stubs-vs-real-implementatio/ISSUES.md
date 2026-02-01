# Issues Found During Examination

**Task:** Cleanup audit: Identify stubs vs real implementations in kelpie-cluster, kelpie-agent; verify single-activation gaps; check ADR accuracy. Goal: authoritative issue list for Option B cleanup.
**Generated:** 2026-01-24T03:05:13.554075+00:00
**Total Issues:** 22

---

## HIGH (11)

### [kelpie-cluster] join_cluster() is non-functional stub - multi-node deployment broken

**Evidence:** cluster.rs:252-267 TODO(Phase 3)

*Found: 2026-01-24T03:02:07.686977+00:00*

---

### [kelpie-cluster] Failure-triggered migrations never executed - actors on failed nodes not recovered

**Evidence:** cluster.rs:367-373 TODO(Phase 6)

*Found: 2026-01-24T03:02:07.686979+00:00*

---

### [kelpie-cluster] No consensus algorithm - cluster membership has no agreement protocol

**Evidence:** No Raft/Paxos code found in any file

*Found: 2026-01-24T03:02:07.686980+00:00*

---

### [kelpie-runtime] Local mode TOCTOU race allows duplicate actor activations

**Evidence:** dispatcher.rs:408-427 check-then-act without mutex

*Found: 2026-01-24T03:03:11.635596+00:00*

---

### [kelpie-runtime] Distributed mode has race window between get_placement() and try_claim_actor()

**Evidence:** dispatcher.rs:404-450 non-atomic sequence

*Found: 2026-01-24T03:03:11.635598+00:00*

---

### [kelpie-runtime] No lease/heartbeat - node crash orphans actors forever

**Evidence:** dispatcher.rs:450-475 no TTL or health check

*Found: 2026-01-24T03:03:11.635599+00:00*

---

### [kelpie-registry] MemoryRegistry has TOCTOU race in try_claim_actor() - two separate locks

**Evidence:** registry.rs:393-420 separate RwLock acquisitions

*Found: 2026-01-24T03:03:59.920851+00:00*

---

### [kelpie-registry] FdbRegistry lease check is outside transaction - TOCTOU between check and claim

**Evidence:** fdb.rs:683-722 get_lease() before transact()

*Found: 2026-01-24T03:03:59.920852+00:00*

---

### [docs/adr] ADR-001/004 claim single-activation as Complete but no distributed mechanism exists

**Evidence:** ADR-001 status table shows Complete for dispatcher.rs only

*Found: 2026-01-24T03:04:34.028275+00:00*

---

### [docs/adr] ADR-004 promises CP behavior via FDB but FDB lease integration Not Started

**Evidence:** ADR-004 implementation status: lease-based ownership Not Started

*Found: 2026-01-24T03:04:34.028276+00:00*

---

### [docs/adr] ADRs document aspirational design as if implemented

**Evidence:** Multiple ✅ Complete markers for unimplemented features

*Found: 2026-01-24T03:04:34.028277+00:00*

---

## MEDIUM (9)

### [kelpie-cluster] TcpTransport uses fake node ID on accept - no handshake protocol

**Evidence:** rpc.rs:607-611 temp_node_id fabricated

*Found: 2026-01-24T03:02:07.686981+00:00*

---

### [kelpie-cluster] MemoryTransport::connect() broken - receivers immediately dropped

**Evidence:** rpc.rs:226-235 _rx_to_other unused

*Found: 2026-01-24T03:02:07.686983+00:00*

---

### [kelpie-cluster] JoinRequest and ClusterStateRequest RPC handlers not implemented

**Evidence:** handler.rs:351-356 returns None

*Found: 2026-01-24T03:02:07.686984+00:00*

---

### [kelpie-runtime] unwrap() on mutex lock can panic if poisoned

**Evidence:** dispatcher.rs:411,462 .lock().unwrap()

*Found: 2026-01-24T03:03:11.635600+00:00*

---

### [kelpie-runtime] ActiveActor::activate() lacks any locking mechanism

**Evidence:** activation.rs:108-147 no distributed coordination

*Found: 2026-01-24T03:03:11.635601+00:00*

---

### [kelpie-registry] FDB tests ignored - no CI coverage for distributed code

**Evidence:** fdb.rs:865-916 #[ignore = requires running FDB cluster]

*Found: 2026-01-24T03:03:59.920853+00:00*

---

### [kelpie-registry] LeaseRenewalTask silent failures - renewal errors only logged, node keeps serving

**Evidence:** fdb.rs:806 warn! but continues loop

*Found: 2026-01-24T03:03:59.920854+00:00*

---

### [kelpie-registry] MemoryRegistry claims 'linearizable' but is single-node only

**Evidence:** registry.rs:21 misleading documentation

*Found: 2026-01-24T03:03:59.920855+00:00*

---

### [docs/adr] ADR-005 Stateright integration is scaffolded only, not functional

**Evidence:** ADR-005: Model implementation is aspirational pseudocode

*Found: 2026-01-24T03:04:34.028279+00:00*

---

## LOW (2)

### [kelpie-agent] kelpie-agent references in ISSUES.md are stale - crate deleted

**Evidence:** Not in Cargo.toml workspace.members, git status shows D crates/kelpie-agent/*

*Found: 2026-01-24T03:02:31.775214+00:00*

---

### [kelpie-server] Some code analysis showed truncation - may need manual verification

**Evidence:** stream_complete() and handle_get_state() appeared incomplete in analysis

*Found: 2026-01-24T03:05:05.665161+00:00*

---
