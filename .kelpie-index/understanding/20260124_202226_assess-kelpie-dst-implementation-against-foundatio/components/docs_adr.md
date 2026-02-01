# docs/adr

**Examined:** 2026-01-24T20:22:19.905676+00:00

## Summary

ADRs define comprehensive requirements but implementation gaps exist between spec and reality

## Connections

- kelpie-dst
- docs/tla

## Details

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

## Issues

### [HIGH] ADR-004 requires partition testing but no partition tests exist

**Evidence:** Linearizability ADR specifies 'minority partitions fail operations' but cluster_dst.rs only tests packet loss

### [HIGH] ADR-005 claims deterministic replay but async scheduling breaks this

**Evidence:** ADR says 'any failure reproducible via DST_SEED' but tokio task ordering varies

### [MEDIUM] ADR-004 specifies lease infrastructure but tests don't exercise it

**Evidence:** ADR mentions LeaseUniqueness invariant but no LeaseManager tests exist
