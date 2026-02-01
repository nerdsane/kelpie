# Codebase Map

**Task:** Assess Kelpie DST implementation against FoundationDB/TigerBeetle standards using ADRs and TLA+ specs
**Generated:** 2026-01-24T20:22:26.817049+00:00
**Components:** 4
**Issues Found:** 14

---

## Components Overview

### docs/adr

**Summary:** ADRs define comprehensive requirements but implementation gaps exist between spec and reality

**Connects to:** kelpie-dst, docs/tla

**Details:**

**ADR-005 DST Framework Requirements:**
- Single source of randomness ✓ Implemented
- Simulated time ✓ Implemented  
- Simulated I/O ✓ Implemented
- Explicit fault injection ✓ Implemented
- 16+ fault types ✓ Implemented
- Deterministic replay ⚠️ Partial (async scheduling non-deterministic)

**ADR-004 Linearizability Requirements:**
- SingleActivation invariant ❌ Not tested under concurrency
- LeaseUniqueness ❌ No lease infrastructure in tests
- NoSplitBrain ❌ No partition tests
- ConsistentHolder ⚠️ Partial (no concurrent scenarios)

**ADR-004 Failure Scenarios Required:**
- Network partitions with quorum ❌ Not tested
- Minority partitions unavailable ❌ Not tested
- Lease expiry and reacquisition ❌ Not tested
- Split-brain prevention ❌ Not tested

**Gap: ADRs promise CP semantics but no tests verify unavailability during partitions**

**Issues (3):**
- [HIGH] ADR-004 requires partition testing but no partition tests exist
- [HIGH] ADR-005 claims deterministic replay but async scheduling breaks this
- [MEDIUM] ADR-004 specifies lease infrastructure but tests don't exercise it

---

### docs/tla

**Summary:** 17 TLA+ specs define rigorous invariants but DST tests verify only a subset

**Connects to:** kelpie-dst, docs/adr

**Details:**

**Key Specs and Their Invariants:**

**KelpieSingleActivation.tla:**
- SingleActivation: At most one Active node ❌ Not tested under concurrency
- ConsistentHolder: Active implies fdb_holder match ⚠️ Partial
- EventualActivation: Claims resolve ❌ No liveness testing

**KelpieRegistry.tla:**
- PlacementConsistency: No actors on Failed nodes ⚠️ Partial
- EventualFailureDetection ❌ No liveness testing
- EventualCacheInvalidation ❌ No cache tests

**KelpieActorLifecycle.tla:**
- LifecycleOrdering: pending > 0 implies Active ⚠️ Partial
- GracefulDeactivation: Deactivating implies pending = 0 ⚠️ Partial
- NoInvokeWhileDeactivating ❌ Not tested

**KelpieLease.tla:**
- LeaseUniqueness: One holder per actor ❌ Not tested
- BeliefConsistency: Node belief matches reality ❌ Not tested

**KelpieWAL.tla:**
- Durability: Completed entries persist ⚠️ Partial
- Idempotency: No duplicates ⚠️ Partial  
- AtomicVisibility ❌ Documented as broken in tests

**Coverage Summary:**
- Safety invariants: ~30% tested (mostly happy path)
- Liveness properties: ~0% tested (no temporal verification)
- Concurrent scenarios: ~0% tested

**Issues (4):**
- [CRITICAL] TLA+ specs define concurrent scenarios but DST tests are all sequential
- [CRITICAL] Liveness properties (temporal) not verified at all
- [HIGH] KelpieWAL AtomicVisibility invariant documented as violated
- [HIGH] KelpieLease invariants have no test coverage

---

### kelpie-core

**Summary:** Core types and traits support DST but enforcement is discipline-based, not compile-time

**Connects to:** kelpie-dst

**Details:**

**DST Support in Core:**
- TimeProvider trait for clock injection ✓
- RngProvider trait for random injection ✓
- Error types with is_retriable() for fault handling ✓

**Gaps:**
- No compile-time enforcement that business logic uses injected providers
- Code can still call `tokio::time::sleep()` or `rand::random()` directly
- No static analysis to detect non-deterministic escapes

**TigerStyle Compliance:**
- Constants with units ✓
- Big-endian naming ✓
- Assertions expected but not verified in DST context

**Issues (2):**
- [MEDIUM] No compile-time enforcement of deterministic I/O
- [LOW] TigerStyle assertions not systematically verified under DST

---

### kelpie-dst

**Summary:** DST infrastructure has good foundations but critical gaps vs FoundationDB/TigerBeetle standards

**Connects to:** kelpie-core, docs/adr, docs/tla

**Details:**

**What's Implemented Well:**
- DeterministicRng with ChaCha20, seed-based replay via DST_SEED
- SimClock with explicit time advancement
- SimStorage with fault injection
- SimNetwork with partitions, delays, reordering
- 16+ fault types defined
- 31 files, 16 test files

**Critical Gap #1: Non-Deterministic Async Execution**
The simulation uses tokio's single-threaded runtime but does NOT control task scheduling. Two tasks spawned via `tokio::spawn()` will interleave non-deterministically even with same seed. FoundationDB embeds all nodes in a single-threaded simulator with deterministic task ordering.

**Critical Gap #2: No Invariant Verification Framework**
Tests check "did operation succeed/fail" but don't verify TLA+ invariants hold after each step. No InvariantChecker trait that runs after faults.

**Critical Gap #3: Concurrent Operations Not Tested**
All tests are sequential. No test spawns concurrent activations, concurrent invocations, or racing operations. TLA+ specs define concurrent scenarios that aren't exercised.

**Critical Gap #4: No Recovery Verification**
Tests inject faults but don't verify recovery paths. No test crashes mid-operation and verifies state is consistent after recovery.

**Critical Gap #5: Asymmetric Partitions Not Supported**
SimNetwork only supports bidirectional partitions. Cannot simulate A→B works but B→A fails.

**Issues (5):**
- [CRITICAL] Async task scheduling is non-deterministic - same seed may produce different task interleavings
- [CRITICAL] TLA+ SingleActivation invariant not tested - no concurrent activation attempts
- [HIGH] No invariant verification framework - tests check success/failure, not state consistency
- [HIGH] No recovery path testing - faults injected but recovery not verified
- [MEDIUM] Asymmetric network partitions not supported

---

## Component Connections

```
docs/adr -> kelpie-dst, docs/tla
docs/tla -> kelpie-dst, docs/adr
kelpie-core -> kelpie-dst
kelpie-dst -> kelpie-core, docs/adr, docs/tla
```
