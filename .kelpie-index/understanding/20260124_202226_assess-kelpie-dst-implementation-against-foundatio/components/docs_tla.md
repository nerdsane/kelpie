# docs/tla

**Examined:** 2026-01-24T20:22:20.031444+00:00

## Summary

17 TLA+ specs define rigorous invariants but DST tests verify only a subset

## Connections

- kelpie-dst
- docs/adr

## Details

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

## Issues

### [CRITICAL] TLA+ specs define concurrent scenarios but DST tests are all sequential

**Evidence:** KelpieSingleActivation defines concurrent StartClaim/CommitClaim but tests never race activations

### [CRITICAL] Liveness properties (temporal) not verified at all

**Evidence:** EventualActivation, EventualLeaseResolution etc. require fairness-based checking, no such tests exist

### [HIGH] KelpieWAL AtomicVisibility invariant documented as violated

**Evidence:** test_dst_kv_state_atomicity_gap explicitly expects invariant violation

### [HIGH] KelpieLease invariants have no test coverage

**Evidence:** LeaseUniqueness, BeliefConsistency defined but no lease tests exist
