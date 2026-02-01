# kelpie-registry

**Examined:** 2026-01-24T03:03:59.920845+00:00

## Summary

Two implementations: MemoryRegistry (single-node, TOCTOU races) and FdbRegistry (distributed with leases, mostly complete but has TOCTOU in try_claim_actor). FDB implementation exists but tests are ignored (require external cluster).

## Connections

- kelpie-runtime
- kelpie-cluster
- kelpie-storage

## Details

**MemoryRegistry (in-memory, single-node):**
- Uses RwLock<HashMap> - only works within single process
- TOCTOU race in try_claim_actor(): two separate lock acquisitions
- State lost on restart - no persistence
- Claims to be "linearizable" but only locally

**FdbRegistry (FoundationDB, distributed):**
- REAL implementation exists with lease-based single-activation
- Lease TTL: 30 seconds, renewal every 10 seconds
- Uses FDB transactions for atomicity
- ISSUE: Lease check is OUTSIDE transaction (lines 683-722)
  - Reads lease, checks if expired, THEN starts transaction to claim
  - Race: Node A reads expired, Node B renews, Node A claims anyway
- ISSUE: Async read before write - FDB 0.10 limitation workaround
- Tests marked #[ignore] - require running FDB cluster

**Both registries lack:**
- Distributed consensus for multi-node coordination
- Thundering herd mitigation on lease expiry
- Threshold-based failure handling in renewal task

## Issues

### [HIGH] MemoryRegistry has TOCTOU race in try_claim_actor() - two separate locks

**Evidence:** registry.rs:393-420 separate RwLock acquisitions

### [HIGH] FdbRegistry lease check is outside transaction - TOCTOU between check and claim

**Evidence:** fdb.rs:683-722 get_lease() before transact()

### [MEDIUM] FDB tests ignored - no CI coverage for distributed code

**Evidence:** fdb.rs:865-916 #[ignore = requires running FDB cluster]

### [MEDIUM] LeaseRenewalTask silent failures - renewal errors only logged, node keeps serving

**Evidence:** fdb.rs:806 warn! but continues loop

### [MEDIUM] MemoryRegistry claims 'linearizable' but is single-node only

**Evidence:** registry.rs:21 misleading documentation
