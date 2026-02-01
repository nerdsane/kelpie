# kelpie-storage

**Examined:** 2026-01-23T00:00:53.529927+00:00

## Summary

Dual-backend storage: FoundationDB provides linearizability via MVCC, MemoryKV for testing only

## Connections

- kelpie-runtime
- kelpie-core

## Details

- FdbKV: Production backend with FoundationDB integration
- Linearizability: FDB's MVCC provides linearizable reads/writes
- ACID: Atomic commit via FDB transactions, buffered writes, automatic conflict retry
- Read-your-writes: Transaction buffer checked before storage
- MemoryKV: Testing mock only, NOT linearizable (RwLock allows staleness)
- Per-actor isolation via FDB subspaces
- Missing: Range scans not transactional (phantom reads possible)

## Issues

### [LOW] Transaction finalization uses assert! instead of Result

**Evidence:** fdb.rs: assert!(!self.finalized) - panics instead of returning error

### [MEDIUM] Range scans not transactional - phantom reads possible

**Evidence:** list_keys() creates new transaction each call, ignores write buffer
