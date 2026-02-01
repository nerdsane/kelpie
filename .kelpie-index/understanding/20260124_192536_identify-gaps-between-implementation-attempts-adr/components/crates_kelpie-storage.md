# crates/kelpie-storage

**Examined:** 2026-01-24T19:25:24.232746+00:00

## Summary

Storage crate with WAL, KV traits, memory backend, and FDB backend stub

## Connections

- docs/tla/KelpieWAL.tla
- docs/adr/002-foundationdb-integration.md
- docs/adr/008-transaction-api.md

## Details

Files: kv.rs (traits), wal.rs (WAL with Pending/Complete/Failed states), transaction.rs, memory.rs (in-memory), fdb.rs (FDB backend). WAL has idempotency checking but no automatic recovery. FDB file exists but needs verification of completeness.

## Issues

### [HIGH] WAL idempotency check is not atomic - race condition between find and insert

**Evidence:** wal.rs: append_with_idempotency() calls find_by_idempotency_key() then append() non-atomically

### [HIGH] WAL has no automatic crash recovery - only provides pending_entries()

**Evidence:** wal.rs: no recovery orchestration, caller must implement

### [MEDIUM] MemoryWal provides no durability - test-only

**Evidence:** wal.rs: in-memory storage loses all data on crash
