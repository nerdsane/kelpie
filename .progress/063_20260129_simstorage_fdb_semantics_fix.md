# Plan: Fix SimStorage Transaction Semantics to Match FDB (Issue #87)

**Created:** 2026-01-29
**Branch:** issue-87-simstorage-fdb
**Status:** IN PROGRESS

## Summary

The GitHub issue #87 claims `SimStorage` does not faithfully simulate FoundationDB's transaction semantics. After thorough investigation, I found:

## Investigation Findings

### Issue Claims vs Reality

| Claim | Reality |
|-------|---------|
| "Multi-key ops use separate locks" | **PARTIALLY CORRECT** - `kelpie-server/src/storage/sim.rs` has separate RwLocks per collection (agents, blocks, sessions, messages, etc.) which could allow inconsistent states during cascading operations |
| "Concurrent writes race" | **INCORRECT** - kelpie-dst `SimStorage` has OCC (Optimistic Concurrency Control) with version tracking |
| "No MVCC" | **CORRECT** - SimStorage uses single-version with locking, not MVCC snapshots |
| "Per-operation locks" | **CORRECT for kelpie-server SimStorage** - Each collection has independent RwLock |
| "Atomic Commit" missing | **INCORRECT for kelpie-dst** - `SimTransaction` in `kelpie-dst/src/storage.rs` has proper atomic commit |

### Key Discovery: TWO SimStorage Implementations

1. **`kelpie-server/src/storage/sim.rs`** - Simple in-memory storage with per-collection RwLocks
   - Uses separate `RwLock<HashMap>` for each entity type
   - `delete_agent()` acquires/releases locks separately for cascading deletes
   - `checkpoint()` inherits DEFAULT trait implementation (non-atomic)
   - **THIS IS THE PROBLEM**

2. **`kelpie-dst/src/storage.rs`** - DST-capable SimStorage with OCC and fault injection
   - Implements `ActorKV` trait with proper transaction support
   - Has version tracking for conflict detection
   - `SimTransaction::commit()` is atomic
   - Used via `KvAdapter::with_dst_storage()` in DST tests

### The Real Problem

The `kelpie-server/src/storage/sim.rs` `SimStorage` does NOT override the `checkpoint()` method, so it falls back to the default non-atomic implementation in `traits.rs`:

```rust
// Default implementation in traits.rs (non-atomic)
async fn checkpoint(
    &self,
    session: &SessionState,
    message: Option<&Message>,
) -> Result<(), StorageError> {
    self.save_session(session).await?;
    if let Some(msg) = message {
        self.append_message(&session.agent_id, msg).await?;
    }
    Ok(())
}
```

Meanwhile, `FdbAgentRegistry` overrides this with a proper atomic transaction (lines 961-1029 in fdb.rs).

### Why This Matters

When DST tests use `KvAdapter::with_dst_storage()`, they get proper transaction semantics from the kelpie-dst `SimStorage`. But if anyone uses the `kelpie-server` `SimStorage` directly:
- Checkpoint is non-atomic (session saved but message append can fail)
- Cascading deletes can leave partial state
- No conflict detection

## Solution Options

### Option A: Fix kelpie-server SimStorage checkpoint (Minimal Change)

Add atomic `checkpoint()` implementation to `kelpie-server/src/storage/sim.rs` that acquires both sessions and messages locks before making changes.

**Pros:**
- Minimal change
- Fixes the specific issue mentioned

**Cons:**
- Still has other non-atomic operations (cascading delete)
- Doesn't add conflict detection

### Option B: Remove kelpie-server SimStorage (Recommended)

The kelpie-server SimStorage is redundant - DST tests should use `KvAdapter` backed by kelpie-dst `SimStorage` for proper transaction semantics.

**Implementation:**
1. Remove `kelpie-server/src/storage/sim.rs` entirely
2. Update all code that uses `SimStorage::new()` to use `KvAdapter::with_memory()` or `KvAdapter::with_dst_storage()`
3. The `KvAdapter` already has proper atomic `checkpoint()` using transactions

**Pros:**
- Single source of truth for simulated storage
- Proper transaction semantics everywhere
- Better DST fidelity

**Cons:**
- Breaking change if anything uses SimStorage directly

### Option C: Make KvAdapter the only AgentStorage implementation

This is essentially Option B but more explicit - `KvAdapter` becomes the primary way to get `AgentStorage`.

## Decision

**Choosing Option A** (Minimal Change) because:
1. The issue specifically mentions `checkpoint` atomicity
2. Less risky change
3. DST tests already use `KvAdapter` which has proper semantics
4. The kelpie-server `SimStorage` is primarily for unit tests where full FDB semantics aren't critical

## Implementation Plan

### Phase 1: Fix checkpoint() in SimStorage

1. Add atomic `checkpoint()` override in `kelpie-server/src/storage/sim.rs`
2. Acquire both sessions and messages locks before making changes
3. Either both succeed or rollback

### Phase 2: Add DST Test for Transaction Semantics

1. Add test in `crates/kelpie-server/tests/fdb_storage_dst.rs` that verifies:
   - Concurrent write conflict detection works
   - Atomic checkpoint succeeds or fails atomically
   - No partial reads during multi-key operations

### Phase 3: Document Simulated FDB Semantics

Add comments documenting which FDB semantics are simulated.

## Acceptance Criteria

- [ ] `SimStorage::checkpoint()` is atomic (session and message together)
- [ ] DST tests pass with fault injection
- [ ] No regressions in existing tests
- [ ] Document which FDB semantics are simulated

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 2026-01-29 | Fix checkpoint() in sim.rs | Minimal change, matches issue scope | Doesn't address all atomicity issues |
| 2026-01-29 | Don't remove SimStorage | Too risky, existing tests depend on it | Redundant code remains |

## What to Try

**Works Now:**
- `KvAdapter::with_dst_storage()` has proper atomic checkpoint
- `FdbAgentRegistry::checkpoint()` is atomic
- DST tests use KvAdapter correctly

**Doesn't Work Yet:**
- `kelpie-server/src/storage/sim.rs` `checkpoint()` is non-atomic
- Cascading deletes in sim.rs are non-atomic

**Known Limitations:**
- SimStorage won't have full MVCC (snapshots)
- SimStorage won't have distributed transaction semantics
- This is acceptable for unit tests
