# Issues Found During Examination

**Task:** Assess Kelpie DST implementation against FoundationDB/TigerBeetle standards using ADRs and TLA+ specs
**Generated:** 2026-01-24T20:22:26.817296+00:00
**Total Issues:** 14

---

## CRITICAL (4)

### [kelpie-dst] Async task scheduling is non-deterministic - same seed may produce different task interleavings

**Evidence:** simulation.rs uses tokio::runtime::Builder::new_current_thread() but tokio's internal scheduler is not controlled

*Found: 2026-01-24T20:22:19.784487+00:00*

---

### [kelpie-dst] TLA+ SingleActivation invariant not tested - no concurrent activation attempts

**Evidence:** actor_lifecycle_dst.rs only tests sequential activations, never concurrent claims

*Found: 2026-01-24T20:22:19.784489+00:00*

---

### [docs/tla] TLA+ specs define concurrent scenarios but DST tests are all sequential

**Evidence:** KelpieSingleActivation defines concurrent StartClaim/CommitClaim but tests never race activations

*Found: 2026-01-24T20:22:20.031453+00:00*

---

### [docs/tla] Liveness properties (temporal) not verified at all

**Evidence:** EventualActivation, EventualLeaseResolution etc. require fairness-based checking, no such tests exist

*Found: 2026-01-24T20:22:20.031454+00:00*

---

## HIGH (6)

### [kelpie-dst] No invariant verification framework - tests check success/failure, not state consistency

**Evidence:** All tests use assert_eq! on operation results, not on system invariants

*Found: 2026-01-24T20:22:19.784490+00:00*

---

### [kelpie-dst] No recovery path testing - faults injected but recovery not verified

**Evidence:** test_dst_kv_state_atomicity_gap documents atomicity violation as expected rather than fixing it

*Found: 2026-01-24T20:22:19.784491+00:00*

---

### [docs/adr] ADR-004 requires partition testing but no partition tests exist

**Evidence:** Linearizability ADR specifies 'minority partitions fail operations' but cluster_dst.rs only tests packet loss

*Found: 2026-01-24T20:22:19.905682+00:00*

---

### [docs/adr] ADR-005 claims deterministic replay but async scheduling breaks this

**Evidence:** ADR says 'any failure reproducible via DST_SEED' but tokio task ordering varies

*Found: 2026-01-24T20:22:19.905684+00:00*

---

### [docs/tla] KelpieWAL AtomicVisibility invariant documented as violated

**Evidence:** test_dst_kv_state_atomicity_gap explicitly expects invariant violation

*Found: 2026-01-24T20:22:20.031455+00:00*

---

### [docs/tla] KelpieLease invariants have no test coverage

**Evidence:** LeaseUniqueness, BeliefConsistency defined but no lease tests exist

*Found: 2026-01-24T20:22:20.031456+00:00*

---

## MEDIUM (3)

### [kelpie-dst] Asymmetric network partitions not supported

**Evidence:** network.rs partition check is bidirectional: (a == from && b == to) || (a == to && b == from)

*Found: 2026-01-24T20:22:19.784492+00:00*

---

### [docs/adr] ADR-004 specifies lease infrastructure but tests don't exercise it

**Evidence:** ADR mentions LeaseUniqueness invariant but no LeaseManager tests exist

*Found: 2026-01-24T20:22:19.905685+00:00*

---

### [kelpie-core] No compile-time enforcement of deterministic I/O

**Evidence:** TimeProvider/RngProvider are traits but nothing prevents direct tokio::time::sleep() calls

*Found: 2026-01-24T20:22:20.148801+00:00*

---

## LOW (1)

### [kelpie-core] TigerStyle assertions not systematically verified under DST

**Evidence:** assert! statements exist but DST doesn't specifically exercise assertion paths

*Found: 2026-01-24T20:22:20.148802+00:00*

---
