# kelpie-storage

**Examined:** 2026-01-25T03:47:58.400921+00:00

## Summary

WAL and KV storage with MemoryWal/KvWal and transaction support. PARTIAL TLA+ compliance - WAL exists with idempotency, atomic commits work, but recovery never invoked and WAL→Execute→Complete not atomic as unit.

## Connections

- kelpie-core
- kelpie-runtime

## Details

**What's Implemented:**
- WAL with WalEntry struct (id, operation, status, idempotency_key)
- MemoryWal and KvWal implementations with JSON serialization
- append_with_idempotency() for duplicate detection
- Transaction buffering with read-your-writes semantics
- FDB backend delegates to FoundationDB's MVCC

**Spec Compliance:**
✅ Read-your-writes: Write buffer checked before storage
✅ Idempotency tracking: Duplicates detected by idempotency_key
✅ Atomic commit: FDB provides ACID, Memory has buffer apply
⚠️ Recovery: pending_entries() exists but NEVER CALLED on startup
⚠️ WAL+Execute+Complete not atomic: Crash between steps 2-3 causes duplicate execution

**Missing:**
- No recovery orchestration on startup
- No state verification (checksum) for WAL entries
- O(n) scan for idempotency lookup (no index)
- cleanup() never scheduled - unbounded growth

## Issues

### [CRITICAL] WAL recovery never invoked - pending_entries() is dead code on startup

**Evidence:** No call to pending_entries() in lib.rs or main startup path

### [HIGH] WAL→Execute→Complete not atomic - crash between 2-3 causes duplicates

**Evidence:** storage code shows 3 separate operations without transaction wrapper

### [MEDIUM] Idempotency lookup is O(n) scan - no index on idempotency_key

**Evidence:** KvWal::find_by_idempotency_key scans all entries

### [MEDIUM] WAL cleanup never scheduled - unbounded storage growth

**Evidence:** cleanup() public but never called
