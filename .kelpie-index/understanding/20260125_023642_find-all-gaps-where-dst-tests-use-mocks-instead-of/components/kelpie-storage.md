# kelpie-storage

**Examined:** 2026-01-25T02:36:25.954369+00:00

## Summary

Storage crate has NO TimeProvider injection - 105 async functions without simulated time support

## Connections

- kelpie-dst
- kelpie-runtime
- foundationdb

## Details

6 files analyzed:
- kv.rs: 23 async functions, NO TimeProvider - GAP
- wal.rs: 38 async functions, NO TimeProvider - GAP  
- memory.rs: 17 async functions, NO TimeProvider - GAP
- fdb.rs: 27 async functions, uses FDB directly, NO TimeProvider - CRITICAL GAP
- transaction.rs: No async code
- lib.rs: Re-exports, uses FDB

Total: 105 async functions across storage crate, NONE have TimeProvider injection.
SimStorage in kelpie-dst implements ActorKV trait, so it CAN substitute for storage in tests,
but the REAL storage code cannot be tested under simulated time.

## Issues

### [HIGH] kelpie-storage has 0% TimeProvider coverage - 105 async functions without time injection

**Evidence:** kv.rs(23), wal.rs(38), memory.rs(17), fdb.rs(27) all lack TimeProvider

### [HIGH] Real storage code cannot be tested under simulated time conditions

**Evidence:** No TimeProvider in any storage file
