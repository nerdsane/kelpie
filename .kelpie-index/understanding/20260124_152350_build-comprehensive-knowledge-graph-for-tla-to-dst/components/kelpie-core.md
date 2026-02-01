# kelpie-core

**Examined:** 2026-01-24T15:22:01.495512+00:00

## Summary

Core types, constants (TigerStyle naming), error types with retriability, and compile-time invariants

## Connections

- kelpie-runtime
- kelpie-registry
- kelpie-storage
- kelpie-dst

## Details

**Core Types:**
- ActorId: {namespace: String, id: String} with validation
- ActorRef: location-transparent wrapper
- Architecture: Arm64, X86_64
- SnapshotKind: Suspend, Teleport, Checkpoint
- StorageBackend: Memory, FoundationDb

**Constants (TigerStyle - THING_UNIT_MAX):**
- ACTOR_ID_LENGTH_BYTES_MAX = 256
- ACTOR_STATE_SIZE_BYTES_MAX = 10MB
- ACTOR_INVOCATION_TIMEOUT_MS_MAX = 30,000
- TRANSACTION_SIZE_BYTES_MAX = 10MB (FDB aligned)
- HEARTBEAT_INTERVAL_MS = 1,000
- HEARTBEAT_TIMEOUT_MS = 5,000
- DST_STEPS_COUNT_MAX = 10,000,000

**Error Types (Retriable):**
- TransactionConflict
- NodeUnavailable
- ActorInvocationTimeout

**Compile-Time Invariants:**
- HEARTBEAT_TIMEOUT_MS > HEARTBEAT_INTERVAL_MS
- ACTOR_ID_LENGTH_BYTES_MAX >= 64
- ACTOR_STATE_SIZE_BYTES_MAX <= 100MB
- ACTOR_INVOCATION_TIMEOUT_MS_MAX >= 1000

**TLA+ Foundation:**
- ActorId constraints well-defined
- Constant bounds explicit with units
- Error state machine clear (retriable vs non-retriable)

## Issues

### [LOW] StorageBackend::FoundationDb requires fdb_cluster_file but validation is runtime not compile-time

**Evidence:** config.rs:128-132
