# kelpie-core

**Examined:** 2026-01-25T03:47:58.809995+00:00

## Summary

Core types, errors, and constants following TigerStyle conventions. Provides foundation for other crates but doesn't directly implement TLA+ invariants - those are in dependent crates.

## Connections

- kelpie-runtime
- kelpie-registry
- kelpie-storage
- kelpie-cluster
- kelpie-dst

## Details

**What's Implemented:**
- ActorId, NodeId with validation
- Error types with is_retriable() classification
- Constants with units in names (TigerStyle)
- TimeProvider and RngProvider traits for DST
- check_quorum() helper (unused by callers)

**TigerStyle Compliance:**
✅ Constants with units: ACTOR_INVOCATION_TIMEOUT_MS_MAX
✅ Big-endian naming: actor_state_size_bytes_max
✅ Explicit error handling via Result<T, Error>

**Spec Relevance:**
- Defines error types used by other crates
- Provides TimeProvider/RngProvider for determinism
- check_quorum() exists but callers don't use it

## Issues

### [MEDIUM] check_quorum() helper exists but is unused by cluster code

**Evidence:** error.rs:110-120 defined, cluster.rs doesn't call it
