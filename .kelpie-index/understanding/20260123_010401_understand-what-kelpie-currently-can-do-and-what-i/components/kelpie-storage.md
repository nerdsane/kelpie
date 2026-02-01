# kelpie-storage

**Examined:** 2026-01-23T01:02:14.053306+00:00

## Summary

Actor KV storage with MemoryKV (in-memory) and FdbKV (FoundationDB) backends, transaction support

## Connections

- kelpie-core
- kelpie-runtime
- kelpie-server

## Details

**WORKING (9 tests pass, 8 ignored for FDB):**
- MemoryKV - in-memory KV store for testing/dev
- Transaction API - commit, abort, read-your-writes isolation
- Key encoding with ordering preservation
- Subspace isolation per actor
- ActorKV trait abstraction

**FDB Backend (requires running cluster):**
- FdbKV connects to FoundationDB cluster
- Tuple-encoded keys for efficient range scans
- Transaction atomicity with FDB guarantees
- 8 tests ignored (require FDB cluster)

**Key features:**
- Actor-scoped key prefixing for isolation
- Transaction buffer with atomic commit
- CRUD operations (get, set, delete, exists, list_keys)

## Issues

### [MEDIUM] FDB tests require external cluster - not run in CI

**Evidence:** 8 ignored tests in fdb.rs

### [LOW] No WAL/journaling for crash recovery in MemoryKV

**Evidence:** memory.rs - data lost on crash
