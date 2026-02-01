# kelpie-storage

**Examined:** 2026-01-24T23:26:05.572685+00:00

## Summary

COMPLETE - Memory and FDB backends with linearizable transactions, WAL implementation

## Connections

- kelpie-core
- kelpie-runtime

## Details

Fully implemented:
- MemoryKV: In-memory HashMap, good for testing/DST
- FdbKV: Production FoundationDB backend with tuple layer encoding
- Transactions: Both backends support ACID with read-your-writes
- FDB auto-retry on conflicts (up to 5 attempts)
- WAL: MemoryWal and KvWal with idempotency support
- ScopedKV: Actor-scoped wrapper for isolation

Note: FDB tests marked #[ignore] - require running FDB cluster

## Issues

### [LOW] Transaction module appears unused - dead code

**Evidence:** transaction.rs defines Transaction/TransactionOp but backends use own implementations

### [LOW] Size validation uses debug asserts only - not enforced in release builds

**Evidence:** encode_key(), set() use assert! not Result::Err
