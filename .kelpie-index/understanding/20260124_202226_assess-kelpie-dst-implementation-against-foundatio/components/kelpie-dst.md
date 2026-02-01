# kelpie-dst

**Examined:** 2026-01-24T20:22:19.784480+00:00

## Summary

DST infrastructure has good foundations but critical gaps vs FoundationDB/TigerBeetle standards

## Connections

- kelpie-core
- docs/adr
- docs/tla

## Details

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

## Issues

### [CRITICAL] Async task scheduling is non-deterministic - same seed may produce different task interleavings

**Evidence:** simulation.rs uses tokio::runtime::Builder::new_current_thread() but tokio's internal scheduler is not controlled

### [CRITICAL] TLA+ SingleActivation invariant not tested - no concurrent activation attempts

**Evidence:** actor_lifecycle_dst.rs only tests sequential activations, never concurrent claims

### [HIGH] No invariant verification framework - tests check success/failure, not state consistency

**Evidence:** All tests use assert_eq! on operation results, not on system invariants

### [HIGH] No recovery path testing - faults injected but recovery not verified

**Evidence:** test_dst_kv_state_atomicity_gap documents atomicity violation as expected rather than fixing it

### [MEDIUM] Asymmetric network partitions not supported

**Evidence:** network.rs partition check is bidirectional: (a == from && b == to) || (a == to && b == from)
