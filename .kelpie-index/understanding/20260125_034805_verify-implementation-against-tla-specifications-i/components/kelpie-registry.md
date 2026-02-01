# kelpie-registry

**Examined:** 2026-01-25T03:47:12.989374+00:00

## Summary

Actor placement registry with MemoryRegistry (functional) and FdbRegistry (stub). CRITICAL: Does NOT implement TLA+ SingleActivation invariant - no OCC/CAS, no fencing tokens, lease manager not integrated.

## Connections

- kelpie-core
- kelpie-cluster
- kelpie-runtime

## Details

**What's Implemented:**
- MemoryRegistry with RwLock-based mutual exclusion (single-process only)
- ActorPlacement with generation field (exists but unused for conflict detection)
- MemoryLeaseManager with basic acquire/release/renew operations
- Node registration and heartbeat timeout detection

**Critical Spec Violations:**
1. **No OCC/CAS**: try_claim_actor() has no version checking - concurrent nodes can both succeed
2. **FdbRegistry is TODO**: All methods return todo!() - distributed case completely unimplemented
3. **Generation field unused**: ActorPlacement.generation exists but never compared during claims
4. **Lease manager not integrated**: LeaseManager exists separately but Registry trait doesn't use it
5. **No fencing tokens**: Zombie prevention mechanism completely absent
6. **Release doesn't invalidate**: unregister_actor() just removes entry, no version bump to invalidate in-flight claims

**Race Condition Example:**
Node A: try_claim(actor-1) → reads v=1
Node B: try_claim(actor-1) → reads v=1
Node A: writes(v=2) → SUCCESS
Node B: writes(v=2) → SUCCESS (no v=1 check!)
Result: Both nodes think they own actor-1

## Issues

### [CRITICAL] SingleActivation invariant VIOLATED - no OCC/CAS for distributed placement

**Evidence:** registry.rs try_claim_actor() has no version comparison

### [CRITICAL] FdbRegistry completely unimplemented - all methods are todo!()

**Evidence:** fdb.rs all trait methods

### [CRITICAL] No fencing tokens - zombie actors can corrupt state

**Evidence:** Lease struct has no fencing_token field

### [HIGH] LeaseManager not integrated with Registry - two parallel paths

**Evidence:** Registry trait doesn't call LeaseManager

### [HIGH] No grace period for lease expiry - immediate reclaim allows overlap

**Evidence:** lease.rs acquire() has no grace period check

### [HIGH] No clock skew handling - MAX_CLOCK_SKEW not defined

**Evidence:** lease.rs and node.rs have no clock skew constants
