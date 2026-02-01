# kelpie-storage

**Examined:** 2026-01-24T15:19:23.864482+00:00

## Summary

Per-actor KV storage with WAL, transaction support, Memory and FDB backends

## Connections

- kelpie-server
- kelpie-runtime
- kelpie-core

## Details

**WAL (Write-Ahead Log):**
- Operations: CreateAgent, UpdateAgent, SendMessage, DeleteAgent, UpdateBlock, Custom
- Entry: id, operation, actor_id, payload, status (Pending/Complete/Failed)
- Append durability: uses atomic transaction
- Idempotency: append_with_idempotency() deduplicates by key

**WAL Recovery Gap:**
- pending_entries() returns pending ops
- NO REPLAY MECHANISM in code - recovery coordinator missing
- Requires external coordinator to replay on startup

**Memory Transaction (Testing):**
- Write buffer until commit
- NOT ATOMIC: writes applied sequentially
- Crash vulnerability: partial writes persist

**FDB Transaction (Production):**
- STRONG atomicity: all ops in single FDB transaction
- ACID guarantees from FDB
- Retry logic with exponential backoff
- Crash-safe: FDB guarantees all-or-nothing

**Invariants:**
1. WAL Entry Uniqueness - idempotency key deduplication
2. Entry Status Monotonicity - Pending → Complete/Failed only
3. Transaction Finalization - exactly one commit/abort
4. Actor Isolation - namespace separation in key space
5. Write Buffer Boundedness - 10000 ops max (memory)

## Issues

### [HIGH] WAL has no replay mechanism - recovery coordinator missing

**Evidence:** wal.rs pending_entries() exists but no code calls it on startup

### [MEDIUM] Memory transaction not atomic - sequential writes, crash vulnerability

**Evidence:** memory.rs:90-196 commit applies writes sequentially

### [LOW] FDB batch size limit implicit - should be explicit

**Evidence:** fdb.rs transaction has no explicit size check
