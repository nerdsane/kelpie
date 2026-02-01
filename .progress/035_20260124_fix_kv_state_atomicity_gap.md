# Fix KV-State Atomicity Gap (Issue #21)

**Status:** Complete
**Created:** 2026-01-24
**Issue:** [#21](https://github.com/rita-aga/kelpie-21-kv-atomicity/issues/21)

## Problem

KV writes (`ctx.kv_set`) were persisting immediately to storage, while state updates (`ctx.state`) were only committed later in a transaction. This created an atomicity gap where:

1. Actor writes KV: `ctx.kv_set("balance", 100)` - **IMMEDIATE persist**
2. Actor updates state: `state.last_transfer = "txn-1"` - **in-memory only**
3. Crash during state commit
4. Result: KV persisted (`balance=100`), state lost (`last_transfer=None`)

This violates the `AtomicVisibility` invariant from `KelpieWAL.tla`.

## Solution: Option A - Transactional Batching

**Chosen approach:** Buffer KV writes during invoke, commit atomically with state.

### Implementation (Already in Place)

The fix was implemented in commit `135112ce`:

1. **BufferingContextKV** (`kelpie-core/src/actor.rs:282-416`)
   - Wraps the underlying KV store
   - Buffers all `set()` and `delete()` operations
   - Provides read-your-writes semantics via local cache
   - Operations are NOT persisted until explicitly drained

2. **Transactional Commit** (`kelpie-runtime/src/activation.rs:219-367`)
   - `process_invocation()` creates `BufferingContextKV` before invoke
   - After invoke, drains buffered ops
   - `save_all_transactional()` commits state + KV atomically:
     ```rust
     async fn save_all_transactional(&mut self, buffered_ops: &[BufferedKVOp]) -> Result<()> {
         let mut txn = self.kv.begin_transaction().await?;

         // Apply all buffered KV operations
         for op in buffered_ops {
             match op {
                 BufferedKVOp::Set { key, value } => txn.set(key, value).await?,
                 BufferedKVOp::Delete { key } => txn.delete(key).await?,
             }
         }

         // Set state within same transaction
         txn.set(STATE_KEY, &state_bytes).await?;

         // Atomic commit - all or nothing
         txn.commit().await
     }
     ```

3. **State Rollback on Failure** (`kelpie-runtime/src/activation.rs:231-305`)
   - State snapshot taken before invoke
   - If transaction fails, state is rolled back to snapshot
   - Buffered KV ops are discarded (never applied to transaction)

4. **SimTransaction Fault Injection** (`kelpie-dst/src/storage.rs:369-529`)
   - `CrashDuringTransaction` fault simulates crash at commit
   - When injected, returns error WITHOUT applying any writes
   - Proves atomicity: neither state nor KV persisted

### Options Considered

| Option | Pros | Cons | Decision |
|--------|------|------|----------|
| **A: Transactional Batching** | Simple, aligns with FDB transactions, no WAL needed | Buffering overhead | **CHOSEN** |
| B: WAL-Based Recovery | Proven pattern, async durability | Complex, duplicates FDB transactions | Rejected |
| C: Compensating Transactions | Works without true transactions | Complex rollback logic, race conditions | Rejected |

**Rationale:** FoundationDB already provides ACID transactions. Buffering KV writes and committing them with state in a single transaction is the simplest approach that leverages FDB's guarantees.

## Verification

### DST Tests

All tests pass with 100% crash fault injection:

```bash
$ cargo test -p kelpie-dst --test actor_lifecycle_dst -- --nocapture
running 11 tests
test test_dst_kv_state_atomicity_gap ... ok
test test_dst_exploratory_bug_hunting ... ok  # 100 iterations, 0 bugs
# ... all 10 tests pass
```

### Test: `test_dst_kv_state_atomicity_gap`

Proves atomicity under 100% crash during transaction commit:
- Injects `CrashDuringTransaction` fault with 100% probability
- Actor performs KV write (`balance=100`) and state update (`last_transfer="txn-1"`)
- Transaction commit crashes
- **Verifies:** Both KV and state are None (neither persisted)

### Test: `test_kv_state_atomicity_under_crash` (Added)

Dedicated test per issue acceptance criteria:
- Uses `CrashDuringTransaction` at various probabilities
- Verifies atomicity invariant: `kv_persisted == state_persisted` always
- Tests multiple crash/recovery cycles

### Test: `test_dst_exploratory_bug_hunting`

100 iterations with realistic fault mix:
- 5% `CrashDuringTransaction`
- 3% `StorageWriteFail`
- 2% `StorageReadFail`
- 5% `StorageLatency`

**Result:** 0 bugs found across 1000 operations (10 ops x 100 iterations)

## What to Try Now

### Works

- **Atomic KV+State:** Run any actor that uses both `ctx.kv_set()` and `ctx.state` - changes are atomic
- **Crash Recovery:** Kill the process during invocation - either all changes persist or none
- **DST Verification:** `cargo test -p kelpie-dst --test actor_lifecycle_dst`

### Doesn't Work Yet

- N/A - all acceptance criteria met

### Known Limitations

- KV ops are buffered in memory until commit - very large batches could OOM
- No partial commit option - all-or-nothing only

## Acceptance Criteria Checklist

- [x] `AtomicVisibility` invariant holds: KV and state are atomic
- [x] Test `test_dst_kv_state_atomicity_gap` passes
- [x] New test `test_kv_state_atomicity_under_crash`
- [x] No partial state visible after crash recovery
- [x] Document chosen approach in ADR (ADR-008)
- [x] Update `docs/tla/KelpieWAL.tla` (AtomicVisibility invariant present)

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 2026-01-12 | Use BufferingContextKV | Simple, no new dependencies | Memory overhead for large batches |
| 2026-01-12 | Commit state+KV in same txn | Leverages FDB ACID | None |
| 2026-01-24 | Add dedicated atomicity test | Per issue acceptance criteria | More test code |

## References

- [Issue #21](https://github.com/rita-aga/kelpie-21-kv-atomicity/issues/21)
- [ADR-008: Transaction API](../docs/adr/008-transaction-api.md)
- [KelpieWAL.tla](../docs/tla/KelpieWAL.tla)
- Commit `135112ce`: Original implementation
